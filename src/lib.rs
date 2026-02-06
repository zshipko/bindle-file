use fs2::FileExt;
use memmap2::Mmap;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zerocopy::{FromBytes, Immutable, IntoBytes, Unaligned};

mod ffi;

const BNDL_MAGIC: &[u8; 8] = b"BINDL001";
const BNDL_ALIGN: usize = 8;
const ENTRY_SIZE: usize = std::mem::size_of::<BindleEntry>();
const FOOTER_SIZE: usize = std::mem::size_of::<BindleFooter>();
const HEADER_SIZE: u64 = 8;

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Clone, Copy, Debug, Default)]
pub struct BindleEntry {
    pub offset: [u8; 8], // Use [u8; 8] for disk stability
    pub compressed_size: [u8; 8],
    pub uncompressed_size: [u8; 8],
    pub crc32: [u8; 4],
    pub name_len: [u8; 2],
    pub compression_type: u8,
    pub _reserved: u8,
}

// Add helpers to convert back to numbers for Rust logic
impl BindleEntry {
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
}

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Debug)]
struct BindleFooter {
    pub index_offset: u64,
    pub entry_count: u64,
}

pub struct Bindle {
    path: PathBuf,
    file: File,
    mmap: Option<Mmap>,
    index: BTreeMap<String, BindleEntry>,
    data_end: u64,
}

impl Bindle {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path_buf)?;
        file.lock_shared()?;

        let len = file.metadata()?.len();
        if len == 0 {
            file.write_all(BNDL_MAGIC)?;
            return Ok(Self {
                path: path_buf,
                file,
                mmap: None,
                index: BTreeMap::new(),
                data_end: HEADER_SIZE,
            });
        }

        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        if &header != BNDL_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid header"));
        }

        let m = unsafe { Mmap::map(&file)? };
        let footer_pos = m.len() - FOOTER_SIZE;
        let footer = BindleFooter::read_from_bytes(&m[footer_pos..]).unwrap();

        let data_end = footer.index_offset;
        let count = footer.entry_count;
        let mut index = BTreeMap::new();

        let mut cursor = data_end as usize;
        for _ in 0..count {
            let entry = BindleEntry::read_from_bytes(&m[cursor..cursor + ENTRY_SIZE]).unwrap();
            let n_start = cursor + ENTRY_SIZE;
            let name =
                String::from_utf8_lossy(&m[n_start..n_start + entry.name_len()]).into_owned();
            index.insert(name, entry);
            let total = ENTRY_SIZE + entry.name_len();
            cursor += (total + (BNDL_ALIGN - 1)) & !(BNDL_ALIGN - 1);
        }

        Ok(Self {
            path: path_buf,
            file,
            mmap: Some(m),
            index,
            data_end,
        })
    }

    pub fn add(&mut self, name: &str, data: &[u8], compress: bool) -> io::Result<()> {
        let (processed, c_type) = if compress {
            (zstd::encode_all(data, 3)?, 1)
        } else {
            (data.to_vec(), 0)
        };

        self.file.seek(SeekFrom::Start(self.data_end))?;
        self.file.write_all(&processed)?;

        let offset = self.data_end;
        let c_size = processed.len() as u64;
        let pad = (8 - (c_size % 8)) % 8;
        if pad > 0 {
            self.file.write_all(&vec![0u8; pad as usize])?;
        }

        self.data_end = offset + c_size + pad;

        let entry = BindleEntry {
            offset: offset.to_le_bytes(),
            compressed_size: c_size.to_le_bytes(),
            uncompressed_size: (data.len() as u64).to_le_bytes(),
            compression_type: c_type,
            name_len: (name.len() as u16).to_le_bytes(),
            ..Default::default()
        };

        self.index.insert(name.to_string(), entry);
        Ok(())
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.file.lock_exclusive()?;
        self.file.seek(SeekFrom::Start(self.data_end))?;
        let index_start = self.data_end;

        for (name, entry) in &self.index {
            self.file.write_all(entry.as_bytes())?;
            self.file.write_all(name.as_bytes())?;
            let pad = (BNDL_ALIGN - ((ENTRY_SIZE + name.len()) % BNDL_ALIGN)) % BNDL_ALIGN;
            if pad > 0 {
                self.file.write_all(&vec![0u8; pad])?;
            }
        }

        let footer = BindleFooter {
            index_offset: index_start,
            entry_count: self.index.len() as u64,
        };
        self.file.write_all(footer.as_bytes())?;
        self.file.flush()?;
        self.mmap = Some(unsafe { Mmap::map(&self.file)? });
        self.file.lock_shared()?;
        Ok(())
    }

    pub fn vacuum(&mut self) -> io::Result<()> {
        let tmp_path = self.path.with_extension("tmp");
        let mut new_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        new_file.write_all(BNDL_MAGIC)?;
        let mut current_offset = HEADER_SIZE;

        for entry in self.index.values_mut() {
            let mut buf = vec![0u8; entry.compressed_size() as usize];
            self.file.seek(SeekFrom::Start(entry.offset()))?;
            self.file.read_exact(&mut buf)?;

            new_file.seek(SeekFrom::Start(current_offset))?;
            new_file.write_all(&buf)?;

            entry.offset = current_offset.to_le_bytes();
            let pad = (8 - (entry.compressed_size() % 8)) % 8;
            if pad > 0 {
                new_file.write_all(&vec![0u8; pad as usize])?;
            }
            current_offset += entry.compressed_size() + pad;
        }

        self.data_end = current_offset;
        self.file = new_file;
        self.save()?;
        std::fs::rename(tmp_path, &self.path)?;
        Ok(())
    }

    pub fn read<'a>(&'a self, name: &str) -> Option<Cow<'a, [u8]>> {
        let entry = self.index.get(name)?;
        let mmap = self.mmap.as_ref()?;
        let data =
            mmap.get(entry.offset() as usize..(entry.offset() + entry.compressed_size()) as usize)?;

        if entry.compression_type == 1 {
            let mut out = Vec::with_capacity(entry.uncompressed_size() as usize);
            zstd::Decoder::new(data).ok()?.read_to_end(&mut out).ok()?;
            Some(Cow::Owned(out))
        } else {
            Some(Cow::Borrowed(data))
        }
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn index(&self) -> &BTreeMap<String, BindleEntry> {
        &self.index
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
            fp.add("hello.txt", data, false).expect("Failed to add");
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
            fp.add("large.bin", &data, true).expect("Failed to add");
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
            fp.add("1.txt", b"First", false).unwrap();
            fp.save().expect("Fail commit 1");
        } // File handle closed here

        // 2. Append session
        {
            let mut fp = Bindle::open(path).expect("Fail open 2");
            // At this point, entries contains "1.txt"

            fp.add("2.txt", b"Second", false).unwrap();
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
}
