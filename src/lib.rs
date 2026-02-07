use fs2::FileExt;
use memmap2::Mmap;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zerocopy::{FromBytes, Immutable, IntoBytes, Unaligned};

pub(crate) mod ffi;

const BNDL_MAGIC: &[u8; 8] = b"BINDL001";
const BNDL_ALIGN: usize = 8;
const ENTRY_SIZE: usize = std::mem::size_of::<Entry>();
const FOOTER_SIZE: usize = std::mem::size_of::<Footer>();
const HEADER_SIZE: usize = 8;
const AUTO_COMPRESS_THRESHOLD: usize = 2048;
const FOOTER_MAGIC: u32 = 0x62626262;

fn pad<
    const SIZE: usize,
    T: Copy + TryFrom<usize> + std::ops::Sub<T, Output = T> + std::ops::Rem<T, Output = T>,
>(
    n: T,
) -> T
where
    <T as std::ops::Sub>::Output: std::ops::Rem<T>,
{
    if let Ok(size) = T::try_from(SIZE) {
        return (size - (n % size)) % size;
    }

    unreachable!()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compress {
    None = 0,
    Zstd = 1,
    #[default]
    Auto = 2,
}

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Clone, Copy, Debug, Default)]
pub struct Entry {
    pub offset: [u8; std::mem::size_of::<u64>()], // Use [u8; 8] for disk stability
    pub compressed_size: [u8; std::mem::size_of::<u64>()],
    pub uncompressed_size: [u8; std::mem::size_of::<u64>()],
    pub crc32: [u8; std::mem::size_of::<u32>()],
    pub name_len: [u8; std::mem::size_of::<u16>()],
    pub compression_type: u8,
    pub _reserved: u8,
}

// Add helpers to convert back to numbers for Rust logic
impl Entry {
    pub fn offset(&self) -> u64 {
        u64::from_le_bytes(self.offset)
    }

    pub fn compressed_size(&self) -> u64 {
        u64::from_le_bytes(self.compressed_size)
    }

    pub fn uncompressed_size(&self) -> u64 {
        u64::from_le_bytes(self.uncompressed_size)
    }

    pub fn name_len(&self) -> usize {
        u16::from_le_bytes(self.name_len) as usize
    }

    pub fn compression_type(&self) -> Compress {
        match self.compression_type {
            0 => Compress::None,
            1 => Compress::Zstd,
            _ => Compress::default(),
        }
    }
}

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Debug)]
struct Footer {
    pub index_offset: u64,
    pub entry_count: u32,
    pub magic: u32,
}

pub struct Bindle {
    path: PathBuf,
    file: File,
    mmap: Option<Mmap>,
    index: BTreeMap<String, Entry>,
    data_end: u64,
}

pub enum Either<A, B> {
    Left(A),
    Right(B),
}

pub struct Reader<'a> {
    decoder: Either<zstd::Decoder<'static, BufReader<io::Cursor<&'a [u8]>>>, io::Cursor<&'a [u8]>>,
}

impl<'a> Read for Reader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.decoder {
            Either::Left(x) => x.read(buf),
            Either::Right(x) => x.read(buf),
        }
    }
}

// Note: Seeking is only supported for uncompressed entries in this simple implementation.
// Seeking in compressed streams requires a frame-aware decoder.
impl<'a> Seek for Reader<'a> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match &mut self.decoder {
            Either::Left(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Seeking not supported on compressed streams",
            )),
            Either::Right(x) => x.seek(pos),
        }
    }
}

pub struct Writer<'a> {
    pub(crate) bindle: &'a mut Bindle,
    pub(crate) encoder: Option<zstd::Encoder<'a, std::fs::File>>,
    pub(crate) name: String,
    pub(crate) start_offset: u64,
    pub(crate) uncompressed_size: u64,
}

impl<'a> Drop for Writer<'a> {
    fn drop(&mut self) {
        let _ = self.close_drop();
    }
}

impl<'a> std::io::Write for Writer<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_chunk(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> Writer<'a> {
    pub fn write_chunk(&mut self, data: &[u8]) -> io::Result<()> {
        if self.name.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "closed"));
        }

        self.uncompressed_size += data.len() as u64;

        if let Some(encoder) = &mut self.encoder {
            encoder.write_all(data)?;
        } else {
            self.bindle.file.write_all(data)?;
        }

        Ok(())
    }

    fn close_drop(&mut self) -> io::Result<()> {
        if self.name.is_empty() {
            return Ok(());
        }

        let (compression_type, current_pos) = if let Some(encoder) = self.encoder.take() {
            let mut f = encoder.finish()?;
            let pos = f.stream_position()?;
            // Sync the main file handle to match the encoder's position
            self.bindle.file.seek(SeekFrom::Start(pos))?;
            (1, pos)
        } else {
            let pos = self.bindle.file.stream_position()?;
            (0, pos)
        };

        let compressed_size = current_pos - self.start_offset;

        // Handle 8-byte alignment padding
        let pad_len = pad::<8, u64>(current_pos);
        if pad_len > 0 {
            self.bindle.file.write_all(&vec![0u8; pad_len as usize])?;
        }

        self.bindle.data_end = current_pos + pad_len;

        let entry = Entry {
            offset: self.start_offset.to_le_bytes(),
            compressed_size: compressed_size.to_le_bytes(),
            uncompressed_size: self.uncompressed_size.to_le_bytes(),
            compression_type,
            name_len: (self.name.len() as u16).to_le_bytes(),
            ..Default::default()
        };

        self.bindle.index.insert(self.name.clone(), entry);
        self.name.clear(); // Mark as closed
        Ok(())
    }

    pub fn close(mut self) -> io::Result<()> {
        self.close_drop()
    }
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
        let footer = Footer::read_from_bytes(&m[footer_pos..]).unwrap();

        if footer.magic != FOOTER_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid footer, the file may be corrupt",
            ));
        }

        let data_end = footer.index_offset;
        let count = footer.entry_count;
        let mut index = BTreeMap::new();

        let mut cursor = data_end as usize;
        for _ in 0..count {
            // Ensure there is enough data left for an Entry header
            if cursor + ENTRY_SIZE > footer_pos {
                break;
            }

            let entry = Entry::read_from_bytes(&m[cursor..cursor + ENTRY_SIZE]).unwrap();
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
                self.file.write_all(&vec![0u8; pad])?;
            }
        }

        let footer = Footer {
            index_offset: index_start,
            entry_count: self.index.len() as u32,
            magic: FOOTER_MAGIC,
        };
        self.file.write_all(footer.as_bytes())?;
        self.file.flush()?;
        self.mmap = Some(unsafe { Mmap::map(&self.file)? });
        self.file.lock_shared()?;
        Ok(())
    }

    pub fn vacuum(&mut self) -> io::Result<()> {
        let tmp_path = self.path.with_extension("tmp");

        // Create and populate the temporary file
        {
            let mut new_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)?;

            new_file.write_all(BNDL_MAGIC)?;
            let mut current_offset = HEADER_SIZE as u64;

            // Copy only live entries to the new file
            for entry in self.index.values_mut() {
                let mut buf = vec![0u8; entry.compressed_size() as usize];
                self.file.seek(SeekFrom::Start(entry.offset()))?;
                self.file.read_exact(&mut buf)?;

                new_file.seek(SeekFrom::Start(current_offset as u64))?;
                new_file.write_all(&buf)?;

                entry.offset = current_offset.to_le_bytes();
                let pad = pad::<8, u64>(entry.compressed_size());
                if pad > 0 {
                    new_file.write_all(&vec![0u8; pad as usize])?;
                }
                current_offset += entry.compressed_size() + pad;
            }

            // Write the index and footer to the TEMP file before closing it
            let index_start = current_offset;
            for (name, entry) in &self.index {
                new_file.write_all(entry.as_bytes())?;
                new_file.write_all(name.as_bytes())?;
                let pad = pad::<BNDL_ALIGN, usize>(ENTRY_SIZE + name.len());
                if pad > 0 {
                    new_file.write_all(&vec![0u8; pad])?;
                }
            }

            let footer = Footer {
                index_offset: index_start,
                entry_count: self.index.len() as u32,
                magic: FOOTER_MAGIC,
            };
            new_file.write_all(footer.as_bytes())?;
            new_file.sync_all()?;
            // new_file is closed here when it goes out of scope
        }

        // Release ALL handles to the original file
        drop(self.mmap.take());
        let _ = self.file.unlock();

        // Re-open self.file in a way that allows us to drop it immediately
        let old_file = std::mem::replace(&mut self.file, File::open(&tmp_path)?);
        drop(old_file);

        // Perform the atomic rename while no handles point to the original path
        std::fs::rename(&tmp_path, &self.path)?;

        // Re-establish the state for the Bindle struct
        let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.lock_shared()?;
        let mmap = unsafe { Mmap::map(&file)? };

        let footer_pos = mmap.len() - FOOTER_SIZE;
        let footer = Footer::read_from_bytes(&mmap[footer_pos..]).unwrap();

        self.file = file;
        self.mmap = Some(mmap);
        self.data_end = footer.index_offset;

        Ok(())
    }

    pub fn read<'a>(&'a self, name: &str) -> Option<Cow<'a, [u8]>> {
        let entry = self.index.get(name)?;
        let mmap = self.mmap.as_ref()?;

        if entry.compression_type == Compress::Zstd as u8 {
            let data = mmap.get(
                entry.offset() as usize..(entry.offset() + entry.compressed_size()) as usize,
            )?;
            let mut out = Vec::with_capacity(entry.uncompressed_size() as usize);
            zstd::Decoder::new(data).ok()?.read_to_end(&mut out).ok()?;
            Some(Cow::Owned(out))
        } else {
            let data = mmap.get(
                entry.offset() as usize..(entry.offset() + entry.uncompressed_size()) as usize,
            )?;
            Some(Cow::Borrowed(data))
        }
    }

    /// Read to an `std::io::Write`
    pub fn read_to<W: std::io::Write>(&self, name: &str, mut w: W) -> std::io::Result<u64> {
        std::io::copy(&mut self.reader(name)?, &mut w)
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

        if entry.compression_type == 1 {
            // Zstd streaming decoder
            let decoder = zstd::Decoder::new(cursor)?;
            Ok(Reader {
                decoder: Either::Left(decoder),
            })
        } else {
            Ok(Reader {
                decoder: Either::Right(cursor),
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
        })
    }
}

impl Drop for Bindle {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_create_and_read() {
        let path = "test_basic.bindl";
        let data = b"Hello, Bindle World!";

        // 1. Create and Write
        {
            let mut fp = Bindle::open(path).expect("Failed to open");
            fp.add("hello.txt", data, Compress::None)
                .expect("Failed to add");
            fp.save().expect("Failed to commit");
        }

        // 2. Open and Read
        {
            let fp = Bindle::open(path).expect("Failed to re-open");
            let result = fp.read("hello.txt").expect("File not found");
            assert_eq!(result.as_ref(), data);
        }

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_zstd_compression() {
        let path = "test_zstd.bindl";
        // Highly compressible data
        let data = vec![b'A'; 1000];

        {
            let mut fp = Bindle::open(path).expect("Failed to open");
            fp.add("large.bin", &data, Compress::Zstd)
                .expect("Failed to add");
            fp.save().expect("Failed to commit");
        }

        let fp = Bindle::open(path).expect("Failed to re-open");

        // Ensure data is correct
        let result = fp.read("large.bin").expect("File not found");
        assert_eq!(result, data);

        // Ensure the file on disk is actually smaller than the raw data (including headers)
        let meta = fs::metadata(path).unwrap();
        assert!(meta.len() < 1000);

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_append_functionality() {
        let path = "test_append.bindl";
        let _ = std::fs::remove_file(path);

        // 1. Initial creation
        {
            let mut fp = Bindle::open(path).expect("Fail open 1");
            fp.add("1.txt", b"First", Compress::Zstd).unwrap();
            fp.save().expect("Fail commit 1");
        } // File handle closed here

        // 2. Append session
        {
            let mut fp = Bindle::open(path).expect("Fail open 2");
            // At this point, entries contains "1.txt"

            fp.add("2.txt", b"Second", Compress::None).unwrap();
            fp.save().expect("Fail commit 2");

            // Now test the read
            let first = fp.read("1.txt").expect("Could not find 1.txt");
            let second = fp.read("2.txt").expect("Could not find 2.txt");

            assert_eq!(first.as_ref(), b"First");
            assert_eq!(second.as_ref(), b"Second");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_invalid_magic() {
        let path = "invalid.bindl";
        fs::write(path, b"NOT_A_PACK_FILE_AT_ALL").unwrap();

        let res = Bindle::open(path);
        assert!(res.is_err());

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_key_shadowing() {
        let path = "test_shadow.bindl";
        let _ = fs::remove_file(path);

        let mut b = Bindle::open(path).expect("Failed to open");

        // 1. Add initial version
        b.add("config.txt", b"v1", Compress::None).unwrap();
        b.save().unwrap();

        // 2. Overwrite with v2 (shadowing)
        b.add("config.txt", b"version_2_is_longer", Compress::None)
            .unwrap();
        b.save().unwrap();

        // 3. Verify latest version is retrieved
        let b2 = Bindle::open(path).expect("Failed to reopen");
        let result = b2.read("config.txt").unwrap();
        assert_eq!(result.as_ref(), b"version_2_is_longer");

        // 4. Verify index count hasn't grown (still 1 entry)
        assert_eq!(b2.len(), 1);

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_vacuum_reclaims_space() {
        let path = "test_vacuum.bindl";
        let _ = fs::remove_file(path);

        let mut b = Bindle::open(path).expect("Failed to open");

        // 1. Add a large file
        let large_data = vec![0u8; 1024];
        b.add("large.bin", &large_data, Compress::None).unwrap();
        b.save().unwrap();
        let size_v1 = fs::metadata(path).unwrap().len();

        // 2. Shadow it with a tiny file
        b.add("large.bin", b"tiny", Compress::None).unwrap();
        b.save().unwrap();
        let size_v2 = fs::metadata(path).unwrap().len();

        // Size should have increased because we appended 'tiny'
        assert!(size_v2 > size_v1);

        // 3. Run Vacuum
        b.vacuum().expect("Vacuum failed");
        let size_v3 = fs::metadata(path).unwrap().len();

        // 4. Verify size is now significantly smaller (reclaimed 1024 bytes)
        assert!(size_v3 < size_v2);

        // 5. Verify data integrity after vacuum
        let b2 = Bindle::open(path).unwrap();
        assert_eq!(b2.read("large.bin").unwrap().as_ref(), b"tiny");

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_directory_pack_unpack_roundtrip() {
        let bindle_path = "roundtrip.bindl";
        let src_dir = "test_src";
        let out_dir = "test_out";

        // Clean up previous runs
        let _ = fs::remove_dir_all(src_dir);
        let _ = fs::remove_dir_all(out_dir);
        let _ = fs::remove_file(bindle_path);

        // 1. Create a dummy directory structure
        fs::create_dir_all(format!("{}/subdir", src_dir)).unwrap();
        fs::write(format!("{}/file1.txt", src_dir), b"Hello World").unwrap();
        fs::write(
            format!("{}/subdir/file2.txt", src_dir),
            b"Compressed Data Content",
        )
        .unwrap();

        // 2. Pack the directory using Rust
        {
            let mut b = Bindle::open(bindle_path).unwrap();
            b.pack(src_dir, Compress::Zstd).expect("Pack failed");
            b.save().expect("Save failed");
        }

        // 3. Unpack the directory using Rust
        {
            let b = Bindle::open(bindle_path).unwrap();
            b.unpack(out_dir).expect("Unpack failed");
        }

        // 4. Verify the contents match exactly
        let content1 = fs::read_to_string(format!("{}/file1.txt", out_dir)).unwrap();
        let content2 = fs::read_to_string(format!("{}/subdir/file2.txt", out_dir)).unwrap();

        assert_eq!(content1, "Hello World");
        assert_eq!(content2, "Compressed Data Content");

        // Cleanup
        fs::remove_dir_all(src_dir).ok();
        fs::remove_dir_all(out_dir).ok();
        fs::remove_file(bindle_path).ok();
    }

    #[test]
    fn test_streaming_manual_chunks() {
        let path = "test_stream.bindl";
        let _ = std::fs::remove_file(path);
        let chunk1 = b"Hello ";
        let chunk2 = b"Streaming ";
        let chunk3 = b"World!";
        let expected = b"Hello Streaming World!";

        {
            let mut b = Bindle::open(path).expect("Failed to open");
            // Start a stream without compression
            let mut s = b
                .writer("streamed_file.txt", Compress::None)
                .expect("Failed to start stream");

            // Write chunks manually
            s.write_chunk(chunk1).unwrap();
            s.write_chunk(chunk2).unwrap();
            s.write_chunk(chunk3).unwrap();

            s.close().expect("Failed to finish stream");
            b.save().expect("Failed to save");
        }

        // Verification
        let b = Bindle::open(path).expect("Failed to reopen");
        let result = b.read("streamed_file.txt").expect("Entry not found");
        assert_eq!(result.as_ref(), expected);
        assert_eq!(result.len(), expected.len());

        let _ = std::fs::remove_file(path);
    }
}
