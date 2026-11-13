//! ZIP reading: end of central directory (incl. ZIP64), central directory,
//! entry extraction with CRC check and a decompression-size cap. Owner: W2-E.

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::ZipEntry;

/// An open ZIP archive.
pub struct ZipArchive<R: Read + Seek> {
    reader: R,
    entries: Vec<ZipEntry>,
}

impl<R: Read + Seek> ZipArchive<R> {
    /// Parse the central directory.
    pub fn new(reader: R) -> Result<Self> {
        let _ = reader;
        Err(Error::not_implemented("ZIP reader (W2-E)"))
    }

    /// Central-directory entries in stored order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Index of the entry named `name`.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// Decompressed content of entry `index`, refusing more than `max_bytes`.
    pub fn read(&mut self, index: usize, max_bytes: u64) -> Result<Vec<u8>> {
        let _ = (&mut self.reader, index, max_bytes);
        Err(Error::not_implemented("ZIP entry read (W2-E)"))
    }
}
