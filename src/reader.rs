use crc32fast::Hasher;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

pub(crate) enum Either<A, B> {
    Left(A),
    Right(B),
}

/// A streaming reader for archive entries.
///
/// Created by the archive's `reader()` method. Automatically decompresses compressed entries and tracks CRC32 for integrity verification.
///
/// # Example
///
/// ```no_run
/// # use bindle_file::Bindle;
/// # let archive = Bindle::open("data.bndl")?;
/// let mut reader = archive.reader("file.txt")?;
/// std::io::copy(&mut reader, &mut std::io::stdout())?;
/// reader.verify_crc32()?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Reader<'a> {
    pub(crate) decoder:
        Either<zstd::Decoder<'static, BufReader<io::Cursor<&'a [u8]>>>, io::Cursor<&'a [u8]>>,
    pub(crate) crc32_hasher: Hasher,
    pub(crate) expected_crc32: u32,
}

impl<'a> Read for Reader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match &mut self.decoder {
            Either::Left(x) => x.read(buf)?,
            Either::Right(x) => x.read(buf)?,
        };

        if n > 0 {
            self.crc32_hasher.update(&buf[..n]);
        }

        Ok(n)
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

impl<'a> Reader<'a> {
    /// Verifies the CRC32 checksum of the data read so far.
    ///
    /// Should be called after reading all data to ensure integrity.
    /// Returns an error if the computed CRC32 doesn't match the expected value.
    pub fn verify_crc32(&self) -> io::Result<()> {
        let computed_crc = self.crc32_hasher.clone().finalize();
        if computed_crc != self.expected_crc32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "CRC32 mismatch: expected {:x}, got {:x}",
                    self.expected_crc32, computed_crc
                ),
            ));
        }
        Ok(())
    }
}
