//! Lossless repack plumbing shared by the generic ZIP repack and the OOXML
//! package writer: a capped, layout-checked [`SourceArchive`], per-entry copy
//! keeping the smaller of the old and the new compressed form, and the
//! post-write content verification. Owner: W3-A.

use std::io::{Read, Seek, Write};

use mj_core::{Error, Result};

use super::source::{self, ArchiveInfo, LocalInfo};
use super::{Compression, EntryOptions, Prepared, WrittenEntry, ZipWriter, MADE_BY_DOS, MADE_BY_UNIX};
use crate::zip::{ZipArchive, ZipEntry, METHOD_DEFLATE, METHOD_STORED};

/// Zip-bomb and resource caps for one package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub max_entries: u64,
    /// Largest inflated entry.
    pub max_entry: u64,
    /// Total inflated bytes read from one package.
    pub max_total: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Caps { max_entries: 100_000, max_entry: 1 << 30, max_total: 4 << 30 }
    }
}

fn perr(msg: impl Into<String>) -> Error {
    Error::parse("ZIP archive", msg)
}

/// A source archive opened for a repack. It holds two handles on the same bytes:
/// one for the reader, one for raw (still compressed) reads.
pub struct SourceArchive<R: Read + Seek> {
    zip: ZipArchive<R>,
    raw: R,
    pub info: ArchiveInfo,
    pub locals: Vec<LocalInfo>,
    caps: Caps,
    inflated: u64,
}

impl SourceArchive<std::io::BufReader<std::fs::File>> {
    /// Open a file (two independent handles).
    pub fn open_file(path: &std::path::Path, caps: Caps) -> Result<Self> {
        let open = || std::fs::File::open(path).map(std::io::BufReader::new).map_err(|e| Error::io(path, e));
        SourceArchive::open(open()?, open()?, caps)
    }
}

impl SourceArchive<std::io::Cursor<Vec<u8>>> {
    /// Open in-memory bytes.
    pub fn open_bytes(bytes: Vec<u8>, caps: Caps) -> Result<Self> {
        SourceArchive::open(std::io::Cursor::new(bytes.clone()), std::io::Cursor::new(bytes), caps)
    }
}

impl<R: Read + Seek> SourceArchive<R> {
    /// Open and check: entry count and declared total within the caps, every local
    /// header readable, no hidden data (see [`source::check_layout`]). Encrypted
    /// archives are refused (`Unsupported`). `a` and `b` read the same bytes.
    pub fn open(a: R, mut b: R, caps: Caps) -> Result<Self> {
        let zip = ZipArchive::new(a)?;
        let info = source::archive_info(&mut b)?;
        if zip.any_encrypted() {
            return Err(Error::unsupported("ZIP repack", "the archive has encrypted entries"));
        }
        let n = zip.entries().len() as u64;
        if n > caps.max_entries {
            return Err(perr(format!("{n} entries exceed the limit of {}", caps.max_entries)));
        }
        let declared: u64 = zip.entries().iter().fold(0u64, |a, e| a.saturating_add(e.uncompressed_size));
        if declared > caps.max_total {
            return Err(perr("declared content exceeds the size limit (possible zip bomb)"));
        }
        let mut locals = Vec::with_capacity(zip.entries().len());
        for e in zip.entries() {
            locals.push(source::local_info(&mut b, e, info.cd_offset)?);
        }
        source::check_layout(zip.entries(), &locals, info.cd_offset)?;
        Ok(SourceArchive { zip, raw: b, info, locals, caps, inflated: 0 })
    }

    /// Entries in stored order.
    pub fn entries(&self) -> &[ZipEntry] {
        self.zip.entries()
    }

    /// Inflated content of entry `i` (CRC-checked), within the per-entry and total caps.
    pub fn read(&mut self, i: usize) -> Result<Vec<u8>> {
        let limit = self.caps.max_entry.min(self.caps.max_total.saturating_sub(self.inflated));
        let d = self.zip.read(i, limit)?;
        self.inflated += d.len() as u64;
        Ok(d)
    }

    /// Raw compressed bytes of entry `i`.
    pub fn raw(&mut self, i: usize) -> Result<Vec<u8>> {
        let e = self.zip.entries().get(i).cloned().ok_or_else(|| perr("no such entry"))?;
        let l = self.locals.get(i).cloned().ok_or_else(|| perr("no such entry"))?;
        source::read_raw(&mut self.raw, &e, &l, self.info.cd_offset)
    }

    /// Header options reproducing entry `i` (raw name bytes, extras, attributes, comment).
    pub fn entry_options(&self, i: usize) -> Result<EntryOptions> {
        let e = self.zip.entries().get(i).ok_or_else(|| perr("no such entry"))?;
        let l = self.locals.get(i).ok_or_else(|| perr("no such entry"))?;
        if let Ok(s) = std::str::from_utf8(&l.name_raw) {
            if s != e.name {
                return Err(perr(format!("local and central names differ for {}", e.name)));
            }
        }
        Ok(EntryOptions {
            name: l.name_raw.clone(),
            flags: e.flags,
            dos_time: e.dos_time,
            dos_date: e.dos_date,
            // The reader does not expose "version made by"; Unix mode bits in the high
            // half of the external attributes identify Unix-made entries.
            version_made_by: if e.external_attrs >> 16 != 0 { MADE_BY_UNIX } else { MADE_BY_DOS },
            internal_attrs: 0,
            external_attrs: e.external_attrs,
            local_extra: l.extra.clone(),
            central_extra: e.extra.clone(),
            comment: e.comment.clone(),
        })
    }
}

/// How one entry is to be compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryRule {
    /// Smallest of: stored, deflate at `level`, the original compressed bytes.
    Best,
    /// Always stored (EPUB / ODF `mimetype`).
    Stored,
}

/// Write `data` (the inflated content of source entry `i`) to `w`. With
/// [`EntryRule::Best`] the original compressed bytes are reused when they are not
/// larger than the new deflate output (never worse than the source).
pub fn copy_entry<R: Read + Seek, W: Write>(
    w: &mut ZipWriter<W>,
    src: &mut SourceArchive<R>,
    i: usize,
    data: &[u8],
    rule: EntryRule,
    level: u8,
) -> Result<WrittenEntry> {
    let opts = src.entry_options(i)?;
    let e = src.entries()[i].clone();
    let p = match rule {
        EntryRule::Stored => Prepared::new(data, Compression::Stored),
        EntryRule::Best => {
            let new = Prepared::new(data, Compression::Best(level));
            let same = data.len() as u64 == e.uncompressed_size && crate::crc32::crc32(data) == e.crc32;
            if same
                && (e.method == METHOD_DEFLATE || e.method == METHOD_STORED)
                && e.compressed_size <= new.compressed_size()
            {
                Prepared::raw(e.method, e.crc32, e.uncompressed_size, src.raw(i)?)
            } else {
                new
            }
        }
    };
    w.add_prepared(&opts, &p)
}

/// One entry the output must contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub name: String,
    pub crc32: u32,
    pub size: u64,
}

/// Verify an archive: it opens with the in-house reader, holds exactly `expected`
/// (same names in the same order), and every entry decodes with the expected CRC-32
/// and size (the reader checks the CRC of the decoded bytes against its directory).
pub fn verify_entries<R: Read + Seek>(r: R, expected: &[Expected], caps: Caps) -> Result<()> {
    let mut z = ZipArchive::new(r)?;
    let names: Vec<&str> = z.entries().iter().map(|e| e.name.as_str()).collect();
    let want: Vec<&str> = expected.iter().map(|e| e.name.as_str()).collect();
    if names != want {
        return Err(perr("the entry list changed"));
    }
    let mut total = 0u64;
    for (i, x) in expected.iter().enumerate() {
        let e = z.entries()[i].clone();
        if e.crc32 != x.crc32 || e.uncompressed_size != x.size {
            return Err(perr(format!("entry {} changed", x.name)));
        }
        let d = z.read(i, caps.max_entry.min(caps.max_total.saturating_sub(total)))?;
        total += d.len() as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn source_zip() -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        w.add("mimetype", b"application/epub+zip", 0, 0, 0).unwrap();
        w.add("a.xml", "<a>text text text</a>".repeat(200).as_bytes(), 1, 0, 0).unwrap();
        w.add("b.bin", &crate::zip::write::tests::noise(3000), 10, 0, 0).unwrap();
        w.finish().unwrap()
    }

    fn expected(src: &SourceArchive<Cursor<Vec<u8>>>) -> Vec<Expected> {
        src.entries()
            .iter()
            .map(|e| Expected { name: e.name.clone(), crc32: e.crc32, size: e.uncompressed_size })
            .collect()
    }

    #[test]
    fn copy_keeps_content_and_never_grows() {
        let bytes = source_zip();
        let mut src = SourceArchive::open_bytes(bytes.clone(), Caps::default()).unwrap();
        let mut w = ZipWriter::new(Vec::new());
        for i in 0..src.entries().len() {
            let d = src.read(i).unwrap();
            let rule = if i == 0 { EntryRule::Stored } else { EntryRule::Best };
            let out = copy_entry(&mut w, &mut src, i, &d, rule, 10).unwrap();
            assert!(out.compressed_size <= src.entries()[i].compressed_size.max(d.len() as u64));
        }
        let out = w.finish().unwrap();
        assert!(out.len() < bytes.len(), "level 1 -> 10 shrinks");
        verify_entries(Cursor::new(out.clone()), &expected(&src), Caps::default()).unwrap();
        let mut wrong = expected(&src);
        wrong[1].crc32 ^= 1;
        assert!(verify_entries(Cursor::new(out.clone()), &wrong, Caps::default()).is_err());
        wrong = expected(&src);
        wrong.swap(1, 2);
        assert!(verify_entries(Cursor::new(out), &wrong, Caps::default()).is_err());
    }

    #[test]
    fn caps_refuse_bombs() {
        let mut w = ZipWriter::new(Vec::new());
        w.add("zeros", &vec![0u8; 4 << 20], 10, 0, 0).unwrap();
        w.add("more", &vec![0u8; 4 << 20], 10, 0, 0).unwrap();
        let bytes = w.finish().unwrap();
        assert!(bytes.len() < 64 * 1024);
        let caps = Caps { max_entries: 10, max_entry: 8 << 20, max_total: 6 << 20 };
        assert!(SourceArchive::open_bytes(bytes.clone(), caps).is_err(), "declared total over the cap");
        let caps = Caps { max_entries: 1, ..Caps::default() };
        assert!(SourceArchive::open_bytes(bytes.clone(), caps).is_err(), "entry count over the cap");
        let caps = Caps { max_entries: 10, max_entry: 1 << 20, max_total: 64 << 20 };
        let mut s = SourceArchive::open_bytes(bytes, caps).unwrap();
        assert!(s.read(0).is_err(), "entry over the per-entry cap");
    }
}
