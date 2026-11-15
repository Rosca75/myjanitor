//! ZIP container (in-house; DEFLATE through `miniz_oxide`). The entry type is the
//! shared contract between the reader (W2-E) and the writer (W3-A).

pub mod read;
pub mod write;

pub use read::ZipArchive;
pub use write::ZipWriter;

/// Compression method: stored.
pub const METHOD_STORED: u16 = 0;
/// Compression method: deflate.
pub const METHOD_DEFLATE: u16 = 8;

/// One central-directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Name as stored (`/` separated). Never trusted as a filesystem path.
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    /// Offset of the local header.
    pub local_header_offset: u64,
    /// MS-DOS date and time as stored.
    pub dos_time: u16,
    pub dos_date: u16,
    pub external_attrs: u32,
    pub extra: Vec<u8>,
    pub comment: Vec<u8>,
}

impl ZipEntry {
    /// True for directory entries.
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }

    /// True when the entry is encrypted (general-purpose flag bit 0).
    pub fn is_encrypted(&self) -> bool {
        self.flags & 1 != 0
    }
}
