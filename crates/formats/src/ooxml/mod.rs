//! OOXML package helpers (content types, relationships, parts, custom
//! properties incl. sensitivity labels `MSIP_Label_*`, digital signature parts
//! `_xmlsignatures/`). Owner: W3-A (W2-E reads labels/signatures through
//! `zip::read` directly in Wave 2).
//!
//! - [`tree`]: flat element index with spans and namespaces for small parts;
//! - [`rels`]: relationships parts and part-name arithmetic;
//! - [`content_types`]: the `Content_Types` part;
//! - [`package`]: the editable package model (reachability, safe removal, GC);
//! - [`verify`]: structural verification of a written package;
//! - [`inspect`]: protection / label / macro detection (SAF-07/08/09).

pub mod content_types;
pub mod package;
pub mod rels;
pub mod tree;
pub mod verify;

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use mj_core::{Error, Result};

pub use package::{Edge, Package};

/// What [`inspect`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Inspection {
    /// Not a ZIP package at all but an encrypted (password / RMS) CFB container,
    /// or a ZIP with encrypted entries.
    pub encrypted: bool,
    /// Digital signature parts (`_xmlsignatures/`) or a signature-origin relationship.
    pub signed: bool,
    /// Sensitivity label (`MSIP_Label_` custom property or `docMetadata/LabelInfo.xml`).
    pub labelled: bool,
    /// VBA project present.
    pub macros: bool,
    /// The file is a ZIP container at all.
    pub zip: bool,
}

/// Inspect a file for SAF-07/08/09 (reads the header, the ZIP directory and at most
/// the custom-properties and root relationships parts).
pub fn inspect(path: &Path) -> Result<Inspection> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    inspect_reader(std::io::BufReader::new(f))
}

/// [`inspect`] over any reader.
pub fn inspect_reader<R: Read + Seek>(mut r: R) -> Result<Inspection> {
    let mut head = [0u8; 8];
    let n = r.read(&mut head).map_err(|e| Error::parse("file", e.to_string()))?;
    r.seek(SeekFrom::Start(0)).map_err(|e| Error::parse("file", e.to_string()))?;
    let mut out = Inspection::default();
    if crate::cfb::is_compound_file(&head[..n]) {
        let mut cf = crate::cfb::CompoundFile::open(r)?;
        out.encrypted = cf.protection()?.is_some();
        return Ok(out);
    }
    let mut z = crate::zip::ZipArchive::new(r)?;
    out.zip = true;
    out.encrypted = z.any_encrypted();
    let names: Vec<String> = z.entries().iter().map(|e| e.name.to_ascii_lowercase()).collect();
    out.signed = names.iter().any(|n| n.starts_with("_xmlsignatures/"));
    out.macros = names.iter().any(|n| n.ends_with("vbaproject.bin"));
    out.labelled = names.iter().any(|n| n == "docmetadata/labelinfo.xml");
    if let Some(i) = names.iter().position(|n| n == "docprops/custom.xml") {
        if let Ok(d) = z.read(i, 16 << 20) {
            out.labelled |= d.windows(11).any(|w| w == b"MSIP_Label_");
        }
    }
    if let Some(i) = names.iter().position(|n| n == "_rels/.rels") {
        if let Ok(d) = z.read(i, 16 << 20) {
            if let Ok(list) = rels::parse(&d) {
                out.signed |= list.iter().any(|r| r.rel_type.ends_with("/digital-signature/origin"));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::ZipWriter;
    use std::io::Cursor;

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        for (n, d) in entries {
            w.add(n, d, 6, 0, 0).unwrap();
        }
        w.finish().unwrap()
    }

    #[test]
    fn detects_signatures_labels_macros_and_encryption() {
        let plain = inspect_reader(Cursor::new(zip(&[("[Content_Types].xml", b"<T/>")]))).unwrap();
        assert_eq!(plain, Inspection { zip: true, ..Default::default() });
        let s = inspect_reader(Cursor::new(zip(&[("_xmlsignatures/sig1.xml", b"<s/>")]))).unwrap();
        assert!(s.signed);
        let l = inspect_reader(Cursor::new(zip(&[(
            "docProps/custom.xml",
            b"<Properties><property name=\"MSIP_Label_1234_Enabled\"/></Properties>",
        )])))
        .unwrap();
        assert!(l.labelled);
        let l2 = inspect_reader(Cursor::new(zip(&[("docMetadata/LabelInfo.xml", b"<l/>")]))).unwrap();
        assert!(l2.labelled);
        let m = inspect_reader(Cursor::new(zip(&[("xl/vbaProject.bin", b"x")]))).unwrap();
        assert!(m.macros);
        let cfb = crate::cfb::build_test_file(&[("EncryptionInfo", b"x"), ("EncryptedPackage", b"y")], &[]);
        let e = inspect_reader(Cursor::new(cfb)).unwrap();
        assert!(e.encrypted && !e.zip);
        assert!(inspect_reader(Cursor::new(b"garbage!".to_vec())).is_err());
    }
}
