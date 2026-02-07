use crc32fast::Hasher;
use std::io::{self, Seek, SeekFrom, Write};

use crate::bindle::Bindle;
use crate::entry::Entry;

/// A streaming writer for adding entries to an archive.
///
/// Created by [`Bindle::writer()`]. Automatically compresses data if requested and computes CRC32 for integrity verification.
///
/// The writer must be closed with [`close()`](Writer::close) or will be automatically closed when dropped. After closing, call [`Bindle::save()`] to commit the index.
///
/// # Example
///
/// ```no_run
/// use std::io::Write;
/// use bindle_file::{Bindle, Compress};
///
/// let mut archive = Bindle::open("data.bndl")?;
/// let mut writer = archive.writer("file.txt", Compress::None)?;
/// writer.write_all(b"data")?;
/// writer.close()?;
/// archive.save()?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Writer<'a> {
    pub(crate) bindle: &'a mut Bindle,
    pub(crate) encoder: Option<zstd::Encoder<'a, std::fs::File>>,
    pub(crate) name: String,
    pub(crate) start_offset: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) crc32_hasher: Hasher,
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
        self.crc32_hasher.update(data);

        match &mut self.encoder {
            Some(encoder) => {
                // Compressed: write to zstd encoder
                encoder.write_all(data)?;
            }
            None => {
                // Uncompressed: write directly to file
                self.bindle.file.write_all(data)?;
            }
        }

        Ok(())
    }

    fn close_drop(&mut self) -> io::Result<()> {
        if self.name.is_empty() {
            return Ok(());
        }

        let (compression_type, current_pos) = match self.encoder.take() {
            Some(encoder) => {
                // Compressed: finish encoder and sync position
                let mut f = encoder.finish()?;
                let pos = f.stream_position()?;
                self.bindle.file.seek(SeekFrom::Start(pos))?;
                (1, pos)
            }
            None => {
                // Uncompressed: already wrote directly to file, just get position
                let pos = self.bindle.file.stream_position()?;
                (0, pos)
            }
        };

        let compressed_size = current_pos - self.start_offset;

        // Handle 8-byte alignment padding
        let pad_len = crate::pad::<8, u64>(current_pos);
        if pad_len > 0 {
            crate::write_padding(&mut self.bindle.file, pad_len as usize)?;
        }

        self.bindle.data_end = current_pos + pad_len;

        let crc32_value = self.crc32_hasher.clone().finalize();

        let mut entry = Entry::default();
        entry.set_offset(self.start_offset);
        entry.set_compressed_size(compressed_size);
        entry.set_uncompressed_size(self.uncompressed_size);
        entry.set_crc32(crc32_value);
        entry.set_name_len(self.name.len() as u16);
        entry.compression_type = compression_type;

        self.bindle.index.insert(self.name.clone(), entry);
        self.name.clear(); // Mark as closed

        // Downgrade to shared lock after write completes
        self.bindle.file.lock_shared()?;
        Ok(())
    }

    /// Closes the writer and finalizes the entry.
    ///
    /// Automatically called when the writer is dropped, but calling explicitly allows error handling.
    pub fn close(mut self) -> io::Result<()> {
        self.close_drop()
    }
}
