use zerocopy::{FromBytes, Immutable, IntoBytes, Unaligned};

use crate::compress::Compress;

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Clone, Copy, Debug, Default)]
pub struct Entry {
    offset: u64,
    compressed_size: u64,
    uncompressed_size: u64,
    crc32: u32,
    name_len: u16,
    pub compression_type: u8,
    pub _reserved: u8,
}

// The binary format uses little-endian byte order for all multi-byte integers.
// These methods handle endianness conversion transparently:
// - On little-endian systems (x86, ARM): zero overhead, direct access
// - On big-endian systems: bytes are swapped to/from little-endian

impl Entry {
    pub fn offset(&self) -> u64 {
        u64::from_le(self.offset)
    }

    pub fn set_offset(&mut self, value: u64) {
        self.offset = value.to_le();
    }

    pub fn compressed_size(&self) -> u64 {
        u64::from_le(self.compressed_size)
    }

    pub fn set_compressed_size(&mut self, value: u64) {
        self.compressed_size = value.to_le();
    }

    pub fn uncompressed_size(&self) -> u64 {
        u64::from_le(self.uncompressed_size)
    }

    pub fn set_uncompressed_size(&mut self, value: u64) {
        self.uncompressed_size = value.to_le();
    }

    pub fn crc32(&self) -> u32 {
        u32::from_le(self.crc32)
    }

    pub fn set_crc32(&mut self, value: u32) {
        self.crc32 = value.to_le();
    }

    pub fn name_len(&self) -> usize {
        u16::from_le(self.name_len) as usize
    }

    pub fn set_name_len(&mut self, value: u16) {
        self.name_len = value.to_le();
    }

    pub fn compression_type(&self) -> Compress {
        Compress::from_u8(self.compression_type)
    }
}

#[repr(C, packed)]
#[derive(FromBytes, Unaligned, IntoBytes, Immutable, Debug)]
pub(crate) struct Footer {
    pub index_offset: u64,
    pub entry_count: u32,
    pub magic: u32,
}

impl Footer {
    pub fn new(index_offset: u64, entry_count: u32, magic: u32) -> Self {
        Self {
            index_offset: index_offset.to_le(),
            entry_count: entry_count.to_le(),
            magic: magic.to_le(),
        }
    }

    pub fn index_offset(&self) -> u64 {
        u64::from_le(self.index_offset)
    }

    pub fn entry_count(&self) -> u32 {
        u32::from_le(self.entry_count)
    }

    pub fn magic(&self) -> u32 {
        u32::from_le(self.magic)
    }
}
