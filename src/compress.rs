/// Compression mode for entries.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compress {
    /// No compression.
    None = 0,
    /// Zstandard compression.
    Zstd = 1,
    /// Automatically compress if entry is larger than 2KB threshold.
    /// Note: This is never stored on disk, only used as a policy hint.
    #[default]
    Auto = 2,
}

impl Compress {
    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            0 => Compress::None,
            1 => Compress::Zstd,
            // Invalid/unknown values default to None (safest option)
            // Auto is never stored on disk, only used as input policy
            _ => Compress::None,
        }
    }
}
