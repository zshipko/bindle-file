use crc32fast::Hasher;
use fs2::FileExt;
use memmap2::Mmap;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zerocopy::{FromBytes, IntoBytes};

use crate::compress::Compress;
use crate::entry::{Entry, Footer};
use crate::reader::{Either, Reader};
use crate::writer::Writer;
use crate::{
    pad, write_padding, AUTO_COMPRESS_THRESHOLD, BNDL_ALIGN, BNDL_MAGIC, ENTRY_SIZE, FOOTER_MAGIC,
    FOOTER_SIZE, HEADER_SIZE,
};

pub struct Bindle {
    pub(crate) path: PathBuf,
    pub(crate) file: File,
    pub(crate) mmap: Option<Mmap>,
    pub(crate) index: BTreeMap<String, Entry>,
    pub(crate) data_end: u64,
}

impl Bindle {
    /// Create a new bindle file, this will overwrite the existing file
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let opts = OpenOptions::new()
            .truncate(true)
            .read(true)
            .write(true)
            .create(true)
            .to_owned();
        Self::new(path_buf, opts)
    }

    /// Open or create a bindle file
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let opts = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .to_owned();
        Self::new(path_buf, opts)
    }

    /// Open a bindle file, this will not create it if it doesn't exist
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let opts = OpenOptions::new().read(true).write(true).to_owned();
        Self::new(path_buf, opts)
    }

    /// Create a new `Bindle` from a path and file, the path must match the file
    pub fn new(path: PathBuf, opts: OpenOptions) -> io::Result<Self> {
        let mut file = opts.open(&path)?;
        file.lock_shared()?;
        let len = file.metadata()?.len();

        // Handle completely new/empty files
        if len == 0 {
            file.write_all(BNDL_MAGIC)?;
            return Ok(Self {
                path,
                file,
                mmap: None,
                index: BTreeMap::new(),
                data_end: HEADER_SIZE as u64,
            });
        }

        // Safety check: File must be at least HEADER + FOOTER size (24 bytes)
        // This prevents "attempt to subtract with overflow" when calculating footer_pos
        if len < (HEADER_SIZE + FOOTER_SIZE) as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small to be a valid bindle",
            ));
        }

        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        if &header != BNDL_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid header"));
        }

        let m = unsafe { Mmap::map(&file)? };

        // Calculate footer position. Subtraction is now safe due to the check above.
        let footer_pos = m.len() - FOOTER_SIZE;
        let footer = Footer::read_from_bytes(&m[footer_pos..]).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Failed to read footer")
        })?;

        if footer.magic() != FOOTER_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid footer, the file may be corrupt",
            ));
        }

        let data_end = footer.index_offset();
        let count = footer.entry_count();
        let mut index = BTreeMap::new();

        let mut cursor = data_end as usize;
        for _ in 0..count {
            // Ensure there is enough data left for an Entry header
            if cursor + ENTRY_SIZE > footer_pos {
                break;
            }

            let entry = match Entry::read_from_bytes(&m[cursor..cursor + ENTRY_SIZE]) {
                Ok(e) => e,
                Err(_) => break, // Corrupted entry, stop reading
            };
            let n_start = cursor + ENTRY_SIZE;

            // Validate that the filename exists within the mapped bounds
            if n_start + entry.name_len() > footer_pos {
                break;
            }

            let name =
                String::from_utf8_lossy(&m[n_start..n_start + entry.name_len()]).into_owned();
            index.insert(name, entry);

            let total = ENTRY_SIZE + entry.name_len();
            cursor += (total + (BNDL_ALIGN - 1)) & !(BNDL_ALIGN - 1);
        }

        Ok(Self {
            path,
            file,
            mmap: Some(m),
            index,
            data_end,
        })
    }

    fn should_auto_compress(&self, compress: Compress, len: usize) -> bool {
        compress == Compress::Zstd || (compress == Compress::Auto && len > AUTO_COMPRESS_THRESHOLD)
    }

    pub fn add(&mut self, name: &str, data: &[u8], compress: Compress) -> io::Result<()> {
        let mut stream = self.writer(name, compress)?;
        stream.write_all(data)?;
        stream.close()?;
        Ok(())
    }

    pub fn add_file(
        &mut self,
        name: &str,
        path: impl AsRef<Path>,
        compress: Compress,
    ) -> io::Result<()> {
        let mut stream = self.writer(name, compress)?;
        let mut src = std::fs::File::open(path)?;
        std::io::copy(&mut src, &mut stream)?;
        Ok(())
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.file.lock_exclusive()?;
        self.file.seek(SeekFrom::Start(self.data_end))?;
        let index_start = self.data_end;

        for (name, entry) in &self.index {
            self.file.write_all(entry.as_bytes())?;
            self.file.write_all(name.as_bytes())?;
            let pad = pad::<BNDL_ALIGN, usize>(ENTRY_SIZE + name.len());
            if pad > 0 {
                write_padding(&mut self.file, pad)?;
            }
        }

        let footer = Footer::new(index_start, self.index.len() as u32, FOOTER_MAGIC);
        self.file.write_all(footer.as_bytes())?;

        // Truncate file to current position to remove any old data
        let current_pos = self.file.stream_position()?;
        self.file.set_len(current_pos)?;
        self.file.flush()?;

        self.mmap = Some(unsafe { Mmap::map(&self.file)? });
        self.file.lock_shared()?;
        Ok(())
    }

    pub fn vacuum(&mut self) -> io::Result<()> {
        let backup_path = self.path.with_extension("backup");

        // Release locks and close current file
        drop(self.mmap.take());
        let _ = self.file.unlock();

        // Rename original to backup
        std::fs::rename(&self.path, &backup_path)?;

        // Open backup for reading
        let mut backup_file = File::open(&backup_path)?;

        // Create new file at original path
        let result = {
            let mut new_file = OpenOptions::new()
                .write(true)
                .read(true)
                .create(true)
                .truncate(true)
                .open(&self.path)?;

            new_file.write_all(BNDL_MAGIC)?;
            let mut current_offset = HEADER_SIZE as u64;

            // Copy only live entries from backup to new file
            for entry in self.index.values_mut() {
                let mut buf = vec![0u8; entry.compressed_size() as usize];
                backup_file.seek(SeekFrom::Start(entry.offset()))?;
                backup_file.read_exact(&mut buf)?;

                new_file.seek(SeekFrom::Start(current_offset))?;
                new_file.write_all(&buf)?;

                entry.set_offset(current_offset);
                let pad = pad::<8, u64>(entry.compressed_size());
                if pad > 0 {
                    write_padding(&mut new_file, pad as usize)?;
                }
                current_offset += entry.compressed_size() + pad;
            }

            // Write the index and footer
            let index_start = current_offset;
            for (name, entry) in &self.index {
                new_file.write_all(entry.as_bytes())?;
                new_file.write_all(name.as_bytes())?;
                let pad = pad::<BNDL_ALIGN, usize>(ENTRY_SIZE + name.len());
                if pad > 0 {
                    write_padding(&mut new_file, pad)?;
                }
            }

            let footer = Footer::new(index_start, self.index.len() as u32, FOOTER_MAGIC);
            new_file.write_all(footer.as_bytes())?;
            new_file.sync_all()?;

            Ok(())
        };

        // Handle result
        match result {
            Ok(()) => {
                // Success - delete backup
                std::fs::remove_file(&backup_path).ok();
            }
            Err(e) => {
                // Failure - restore from backup
                std::fs::remove_file(&self.path).ok();
                std::fs::rename(&backup_path, &self.path).ok();
                return Err(e);
            }
        }

        // Re-open the new file
        let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.lock_shared()?;
        let mmap = unsafe { Mmap::map(&file)? };

        let footer_pos = mmap.len() - FOOTER_SIZE;
        let footer = Footer::read_from_bytes(&mmap[footer_pos..]).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Failed to read footer after vacuum")
        })?;

        self.file = file;
        self.mmap = Some(mmap);
        self.data_end = footer.index_offset();

        Ok(())
    }

    pub fn read<'a>(&'a self, name: &str) -> Option<Cow<'a, [u8]>> {
        let entry = self.index.get(name)?;
        let mmap = self.mmap.as_ref()?;

        let data = if entry.compression_type() == Compress::Zstd {
            let compressed_data = mmap.get(
                entry.offset() as usize..(entry.offset() + entry.compressed_size()) as usize,
            )?;
            let mut out = Vec::with_capacity(entry.uncompressed_size() as usize);
            zstd::Decoder::new(compressed_data)
                .ok()?
                .read_to_end(&mut out)
                .ok()?;
            Cow::Owned(out)
        } else {
            let uncompressed_data = mmap.get(
                entry.offset() as usize..(entry.offset() + entry.uncompressed_size()) as usize,
            )?;
            Cow::Borrowed(uncompressed_data)
        };

        // Verify CRC32
        let computed_crc = crc32fast::hash(&data);
        if computed_crc != entry.crc32() {
            return None;
        }

        Some(data)
    }

    /// Read to an `std::io::Write`
    pub fn read_to<W: std::io::Write>(&self, name: &str, mut w: W) -> std::io::Result<u64> {
        let mut reader = self.reader(name)?;
        let bytes_copied = std::io::copy(&mut reader, &mut w)?;
        reader.verify_crc32()?;
        Ok(bytes_copied)
    }

    // Returns a seekable reader for an entry.
    /// If compressed, it provides a transparently decompressing stream.
    pub fn reader<'a>(&'a self, name: &str) -> io::Result<Reader<'a>> {
        let entry = self
            .index
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Entry not found"))?;

        let start = entry.offset() as usize;
        let end = start + entry.compressed_size() as usize;
        let mmap = self
            .mmap
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing mmap"))?;
        let data_slice = &mmap[start..end];

        let cursor = io::Cursor::new(data_slice);

        if entry.compression_type() == Compress::Zstd {
            // Zstd streaming decoder
            let decoder = zstd::Decoder::new(cursor)?;
            Ok(Reader {
                decoder: Either::Left(decoder),
                crc32_hasher: Hasher::new(),
                expected_crc32: entry.crc32(),
            })
        } else {
            Ok(Reader {
                decoder: Either::Right(cursor),
                crc32_hasher: Hasher::new(),
                expected_crc32: entry.crc32(),
            })
        }
    }

    /// The number of entries
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if there are no entries
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Direct readonly access to the index
    pub fn index(&self) -> &BTreeMap<String, Entry> {
        &self.index
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.index.clear()
    }

    /// Checks if an entry exists in the archive index.
    pub fn exists(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Remove an entry from the index.
    /// The data remains in the file until vacuum() is called.
    /// Returns true if the entry existed and was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.index.remove(name).is_some()
    }

    /// Recursively packs a directory into the archive.
    pub fn pack<P: AsRef<Path>>(&mut self, src_dir: P, compress: Compress) -> io::Result<()> {
        self.pack_recursive(src_dir.as_ref(), src_dir.as_ref(), compress)
    }

    fn pack_recursive(
        &mut self,
        base: &Path,
        current: &Path,
        compress: Compress,
    ) -> io::Result<()> {
        if current.is_dir() {
            for entry in std::fs::read_dir(current)? {
                self.pack_recursive(base, &entry?.path(), compress)?;
            }
        } else {
            let name = current
                .strip_prefix(base)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
                .to_string_lossy();
            let mut data = Vec::new();
            File::open(current)?.read_to_end(&mut data)?;
            self.add(&name, &data, compress)?;
        }
        Ok(())
    }

    /// Unpacks all archive entries to a destination directory.
    pub fn unpack<P: AsRef<Path>>(&self, dest: P) -> io::Result<()> {
        let dest_path = dest.as_ref();
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        for (name, _) in &self.index {
            if let Some(data) = self.read(name) {
                let file_path = dest_path.join(name);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(file_path, data)?;
            }
        }
        Ok(())
    }

    pub fn writer<'a>(&'a mut self, name: &str, compress: Compress) -> io::Result<Writer<'a>> {
        self.file.seek(SeekFrom::Start(self.data_end))?;
        let compress = self.should_auto_compress(compress, 0);
        let f = self.file.try_clone()?;
        let start_offset = self.data_end;
        Ok(Writer {
            name: name.to_string(),
            bindle: self,
            encoder: if compress {
                Some(zstd::Encoder::new(f, 3)?)
            } else {
                None
            },
            start_offset,
            uncompressed_size: 0,
            crc32_hasher: Hasher::new(),
        })
    }
}

impl Drop for Bindle {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
