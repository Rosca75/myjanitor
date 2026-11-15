//! Bounded file reads shared by the content-inspecting detectors (zeroed, broken files, shortcuts,
//! sidecars). Every read is capped by its caller; nothing here ever opens a file for writing.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Read up to `n` bytes from the start of the file.
pub fn read_head(path: &Path, n: usize) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    read_up_to(&mut f, n)
}

/// Read the last `n` bytes (fewer for a shorter file) and report the file length.
pub fn read_tail(path: &Path, n: usize) -> io::Result<(Vec<u8>, u64)> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(n as u64);
    f.seek(SeekFrom::Start(start))?;
    Ok((read_up_to(&mut f, n)?, len))
}

/// Read up to `n` bytes starting at `offset`.
pub fn read_at(f: &mut File, offset: u64, n: usize) -> io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(offset))?;
    read_up_to(f, n)
}

fn read_up_to<R: Read>(r: &mut R, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(n.min(1 << 20));
    r.take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn head_tail_and_short_files() {
        let t = TempDir::new("fsio").unwrap();
        let p = t.write("f", b"0123456789").unwrap();
        assert_eq!(read_head(&p, 4).unwrap(), b"0123");
        assert_eq!(read_head(&p, 99).unwrap(), b"0123456789");
        assert_eq!(read_tail(&p, 3).unwrap(), (b"789".to_vec(), 10));
        assert_eq!(read_tail(&p, 99).unwrap().0.len(), 10);
        assert!(read_head(&t.join("missing"), 4).is_err());
    }
}
