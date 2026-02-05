use memmap2::Mmap;
use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use zerocopy::{FromBytes, Immutable, IntoBytes, Unaligned};

mod ffi;

const BNDL_MAGIC: &[u8; 8] = b"BINDL001";
const BNDL_ALIGN: usize = 8;
const ENTRY_SIZE: usize = std::mem::size_of::<Entry>();
const FOOTER_SIZE: usize = std::mem::size_of::<Footer>();
const HEADER_SIZE: u64 = 8;

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Clone, Copy, Debug)]
pub struct Entry {
    pub offset: [u8; 8],
    pub compressed_size: [u8; 8],
    pub uncompressed_size: [u8; 8],
    pub crc32: [u8; 4],
    pub name_len: [u8; 2],
    pub compression_type: u8,
    pub _reserved: u8,
}

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Debug)]
struct Footer {
    pub index_offset: [u8; 8],
    pub entry_count: [u8; 4],
}

pub struct Bindle {
    file: File,
    mmap: Option<Mmap>,
    entries: Vec<(Entry, String)>,
    data_end: u64,
}

impl Bindle {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let len = file.metadata()?.len();

        if len == 0 {
            // New file: Write the magic header immediately
            file.write_all(BNDL_MAGIC)?;
            return Ok(Self {
                file,
                mmap: None,
                entries: Vec::new(),
                data_end: HEADER_SIZE,
            });
        }

        // Existing file: Check header magic
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        if &header != BNDL_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid Bindle header",
            ));
        }
        // Case 2: File exists but is too small to even hold a footer
        if len < FOOTER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small to be a Bindle",
            ));
        }

        let m = unsafe { Mmap::map(&file)? };
        let footer_pos = m.len() - FOOTER_SIZE;

        let footer = Footer::read_from_bytes(&m[footer_pos..])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid footer alignment"))?;

        // If magic is valid, proceed to parse the index
        let data_end = u64::from_le_bytes(footer.index_offset);
        let count = u32::from_le_bytes(footer.entry_count);
        let mut entries = Vec::with_capacity(count as usize);

        let mut cursor = data_end as usize;
        for _ in 0..count {
            let entry_bytes = m
                .get(cursor..cursor + ENTRY_SIZE)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Index out of bounds"))?;
            let entry = Entry::read_from_bytes(entry_bytes)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid entry"))?;

            let n_len = u16::from_le_bytes(entry.name_len) as usize;
            let n_start = cursor + ENTRY_SIZE;
            let n_end = n_start + n_len;

            let name_bytes = m
                .get(n_start..n_end)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Name out of bounds"))?;
            let name = String::from_utf8_lossy(name_bytes).into_owned();

            entries.push((entry, name));

            let total = ENTRY_SIZE + n_len;
            cursor += (total + (BNDL_ALIGN - 1)) & !(BNDL_ALIGN - 1);
        }

        Ok(Self {
            file,
            mmap: Some(m),
            entries,
            data_end,
        })
    }

    /// Reads data for an entry using Cow to avoid unnecessary copies.
    pub fn read<'a>(&'a self, name: &str) -> Option<Cow<'a, [u8]>> {
        let (entry, _) = self.entries.iter().find(|(_, n)| n == name)?;
        let mmap = self.mmap.as_ref()?;

        let offset = u64::from_le_bytes(entry.offset) as usize;
        let c_size = u64::from_le_bytes(entry.compressed_size) as usize;
        let u_size = u64::from_le_bytes(entry.uncompressed_size) as usize;

        let data = mmap.get(offset..offset + c_size)?;

        if entry.compression_type == 1 {
            let mut out = Vec::with_capacity(u_size);
            zstd::Decoder::new(data).ok()?.read_to_end(&mut out).ok()?;
            Some(Cow::Owned(out))
        } else {
            Some(Cow::Borrowed(data))
        }
    }

    /// Streams data directly to a writer (e.g., File, TcpStream) to keep memory usage low.
    pub fn read_to_writer<W: Write>(&self, name: &str, mut writer: W) -> io::Result<u64> {
        let (entry, _) = self
            .entries
            .iter()
            .find(|(_, n)| n == name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Entry not found"))?;

        let mmap = self
            .mmap
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Archive not mapped"))?;

        let offset = u64::from_le_bytes(entry.offset) as usize;
        let c_size = u64::from_le_bytes(entry.compressed_size) as usize;
        let data = mmap.get(offset..offset + c_size).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Data range out of bounds")
        })?;

        if entry.compression_type == 1 {
            let mut decoder = zstd::Decoder::new(data)?;
            io::copy(&mut decoder, &mut writer)
        } else {
            writer.write_all(data)?;
            Ok(data.len() as u64)
        }
    }

    pub fn add(&mut self, name: &str, data: &[u8], compress: bool) -> io::Result<()> {
        // Prevent Duplicate Keys
        if self
            .entries
            .iter()
            .any(|(_, existing_name)| existing_name == name)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Entry '{}' already exists in bindle", name),
            ));
        }

        // Position the file pointer at the end of valid data
        // If data_end is 0, we start after the 8-byte Magic Header
        let write_pos = if self.data_end >= HEADER_SIZE {
            self.data_end
        } else {
            HEADER_SIZE
        };

        self.file.seek(SeekFrom::Start(write_pos))?;

        // Prepare and write data
        let write_data = if compress {
            zstd::encode_all(data, 3)?
        } else {
            data.to_vec()
        };

        let start_offset = self.file.stream_position()?;
        self.file.write_all(&write_data)?;

        // Align to 8 bytes for the next entry or index
        let current_pos = self.file.stream_position()?;
        let pad = (BNDL_ALIGN as u64 - (current_pos % BNDL_ALIGN as u64)) % BNDL_ALIGN as u64;
        if pad > 0 {
            self.file.write_all(&vec![0u8; pad as usize])?;
        }

        // 5. Update state
        self.data_end = self.file.stream_position()?;
        let entry = Entry {
            offset: start_offset.to_le_bytes(),
            compressed_size: (write_data.len() as u64).to_le_bytes(),
            uncompressed_size: (data.len() as u64).to_le_bytes(),
            crc32: crc32fast::hash(&write_data).to_le_bytes(),
            name_len: (name.len() as u16).to_le_bytes(),
            compression_type: if compress { 1 } else { 0 },
            _reserved: 0,
        };

        self.entries.push((entry, name.to_string()));
        Ok(())
    }

    pub fn save(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(self.data_end))?;
        let index_start = self.data_end;

        for (entry, name) in &self.entries {
            self.file.write_all(entry.as_bytes())?;
            self.file.write_all(name.as_bytes())?;
            let current_disk_size = ENTRY_SIZE + name.len();
            let pad = (BNDL_ALIGN - (current_disk_size % BNDL_ALIGN)) % BNDL_ALIGN;
            if pad > 0 {
                self.file.write_all(&vec![0u8; pad])?;
            }
        }

        let footer = Footer {
            index_offset: index_start.to_le_bytes(),
            entry_count: (self.entries.len() as u32).to_le_bytes(),
        };

        self.file.write_all(footer.as_bytes())?;
        self.file.flush()?;

        self.mmap = Some(unsafe { Mmap::map(&self.file)? });
        Ok(())
    }

    /// Returns a list of all entry names in the archive.
    pub fn list(&self) -> Vec<&str> {
        self.entries.iter().map(|(_, name)| name.as_str()).collect()
    }

    pub fn entries(&self) -> &[(Entry, String)] {
        &self.entries
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
