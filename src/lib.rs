//! Bindle is a binary archive format for collecting files.
//!
//! The format uses memory-mapped I/O for fast reads, optional zstd compression,
//! and supports append-only writes with shadowing for updates.
//!
//! # Example
//!
//! ```no_run
//! use bindle_file::{Bindle, Compress};
//!
//! let mut archive = Bindle::open("data.bndl")?;
//! archive.add("file.txt", b"data", Compress::None)?;
//! archive.save()?;
//!
//! let data = archive.read("file.txt").unwrap();
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io::{self, Write};

// Module declarations
mod bindle;
mod compress;
mod entry;
mod reader;
mod writer;

pub(crate) mod ffi;

// Public re-exports
pub use bindle::Bindle;
pub use compress::Compress;
pub use entry::Entry;
pub use reader::Reader;
pub use writer::Writer;

// Constants
pub(crate) const BNDL_MAGIC: &[u8; 8] = b"BINDL001";
pub(crate) const BNDL_ALIGN: usize = 8;
pub(crate) const ENTRY_SIZE: usize = std::mem::size_of::<Entry>();
pub(crate) const FOOTER_SIZE: usize = std::mem::size_of::<entry::Footer>();
pub(crate) const HEADER_SIZE: usize = 8;
pub(crate) const AUTO_COMPRESS_THRESHOLD: usize = 2048;
pub(crate) const FOOTER_MAGIC: u32 = 0x62626262;
const ZEROS: &[u8; 64] = &[0u8; 64]; // Reusable zero buffer for padding

// Helper functions
pub(crate) fn pad<
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

// Helper to write padding zeros without allocating
pub(crate) fn write_padding<W: Write>(writer: &mut W, len: usize) -> io::Result<()> {
    let mut remaining = len;
    while remaining > 0 {
        let chunk = remaining.min(ZEROS.len());
        writer.write_all(&ZEROS[..chunk])?;
        remaining -= chunk;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom};

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

    #[test]
    fn test_crc32_corruption_detection() {
        let path = "test_crc32.bindl";
        let _ = std::fs::remove_file(path);
        let data = b"Test data for CRC32 verification";

        // 1. Create a file with valid data
        {
            let mut b = Bindle::open(path).expect("Failed to open");
            b.add("test.txt", data, Compress::None).unwrap();
            b.save().unwrap();
        }

        // 2. Verify that reading with correct data works
        {
            let b = Bindle::open(path).expect("Failed to reopen");
            let result = b.read("test.txt").expect("Should read successfully");
            assert_eq!(result.as_ref(), data);
        }

        // 3. Corrupt the data by modifying a byte directly in the file
        {
            let mut file = OpenOptions::new()
                .write(true)
                .read(true)
                .open(path)
                .unwrap();

            // Skip the header and modify the first byte of data
            file.seek(SeekFrom::Start(HEADER_SIZE as u64)).unwrap();
            file.write(&[b'X']).unwrap(); // Corrupt first byte
            file.flush().unwrap();
        }

        // 4. Verify that reading corrupted data fails CRC32 check
        {
            let b = Bindle::open(path).expect("Failed to reopen after corruption");
            let result = b.read("test.txt");
            assert!(result.is_none(), "Read should fail due to CRC32 mismatch");
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_crc32_with_compression() {
        let path = "test_crc32_compressed.bindl";
        let _ = std::fs::remove_file(path);
        let data = vec![b'A'; 2000]; // Large enough to trigger compression

        // 1. Create a file with compressed data
        {
            let mut b = Bindle::open(path).expect("Failed to open");
            b.add("compressed.bin", &data, Compress::Zstd).unwrap();
            b.save().unwrap();
        }

        // 2. Verify that reading compressed data works and CRC32 is verified
        {
            let b = Bindle::open(path).expect("Failed to reopen");
            let result = b.read("compressed.bin").expect("Should read successfully");
            assert_eq!(result.as_ref(), data.as_slice());
        }

        // 3. Also test with the streaming reader
        {
            let b = Bindle::open(path).expect("Failed to reopen");
            let mut reader = b.reader("compressed.bin").unwrap();
            let mut output = Vec::new();
            std::io::copy(&mut reader, &mut output).unwrap();
            reader.verify_crc32().expect("CRC32 should match");
            assert_eq!(output, data);
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_remove_entry() {
        let path = "test_remove.bindl";
        let _ = fs::remove_file(path);

        let mut b = Bindle::open(path).expect("Failed to open");

        // Add some entries
        b.add("file1.txt", b"Content 1", Compress::None).unwrap();
        b.add("file2.txt", b"Content 2", Compress::None).unwrap();
        b.add("file3.txt", b"Content 3", Compress::None).unwrap();
        b.save().unwrap();

        assert_eq!(b.len(), 3);
        assert!(b.exists("file2.txt"));

        // Remove an entry
        assert!(b.remove("file2.txt"));
        assert_eq!(b.len(), 2);
        assert!(!b.exists("file2.txt"));

        // Try to remove non-existent entry
        assert!(!b.remove("nonexistent.txt"));

        // Save and reload to verify persistence
        b.save().unwrap();
        let b2 = Bindle::open(path).unwrap();
        assert_eq!(b2.len(), 2);
        assert!(b2.exists("file1.txt"));
        assert!(!b2.exists("file2.txt"));
        assert!(b2.exists("file3.txt"));

        // Verify data still readable for remaining entries
        assert_eq!(b2.read("file1.txt").unwrap().as_ref(), b"Content 1");
        assert_eq!(b2.read("file3.txt").unwrap().as_ref(), b"Content 3");

        fs::remove_file(path).ok();
    }
}
