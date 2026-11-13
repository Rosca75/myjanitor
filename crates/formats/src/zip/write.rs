//! ZIP writing (repack at maximum deflate level, stored entries, ZIP64). Owner: W3-A.

use std::io::Write;

use mj_core::{Error, Result};

/// A ZIP being written.
pub struct ZipWriter<W: Write> {
    writer: W,
}

impl<W: Write> ZipWriter<W> {
    /// Start a new archive.
    pub fn new(writer: W) -> Self {
        ZipWriter { writer }
    }

    /// Add an entry. `level` 0 = stored, 1-10 deflate (10 = miniz "uber").
    pub fn add(&mut self, name: &str, data: &[u8], level: u8, dos_time: u16, dos_date: u16) -> Result<()> {
        let _ = (&mut self.writer, name, data, level, dos_time, dos_date);
        Err(Error::not_implemented("ZIP writer (W3-A)"))
    }

    /// Write the central directory and return the inner writer.
    pub fn finish(self) -> Result<W> {
        Err(Error::not_implemented("ZIP writer (W3-A)"))
    }
}
