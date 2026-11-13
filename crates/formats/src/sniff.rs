//! Content-type detection from the first bytes of a file (CLN-CNT-02,
//! OPT-ENG-02). Owner: W2-C. Wave 1 ships a small magic-number table;
//! W2-C extends it (OOXML vs plain ZIP, MP4 brands, text heuristics...).

use serde::{Deserialize, Serialize};

/// Bytes callers should read for [`sniff`].
pub const SNIFF_LEN: usize = 4096;

/// Detected content kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Tiff,
    Webp,
    Heif,
    Pdf,
    /// ZIP container (OOXML, ODF, EPUB, JAR... refine with `zip`).
    Zip,
    /// OLE compound file (legacy Office, encrypted OOXML, MSI).
    Cfb,
    SevenZip,
    Rar,
    Gzip,
    Zstd,
    Xz,
    Bzip2,
    Mp3,
    Flac,
    Ogg,
    Wav,
    /// ISO BMFF (MP4, MOV, M4A, HEIC by brand).
    IsoBmff,
    Matroska,
    Exe,
    Elf,
}

/// Detect the kind from a file header; `None` when unknown.
pub fn sniff(h: &[u8]) -> Option<FileKind> {
    let starts = |m: &[u8]| h.starts_with(m);
    Some(match () {
        _ if starts(&[0xFF, 0xD8, 0xFF]) => FileKind::Jpeg,
        _ if starts(b"\x89PNG\r\n\x1a\n") => FileKind::Png,
        _ if starts(b"GIF87a") || starts(b"GIF89a") => FileKind::Gif,
        _ if starts(b"BM") && h.len() >= 14 => FileKind::Bmp,
        _ if starts(b"II*\0") || starts(b"MM\0*") => FileKind::Tiff,
        _ if starts(b"RIFF") && h.get(8..12) == Some(b"WEBP") => FileKind::Webp,
        _ if starts(b"RIFF") && h.get(8..12) == Some(b"WAVE") => FileKind::Wav,
        _ if h.get(4..8) == Some(b"ftyp") => match h.get(8..12) {
            Some(b"heic") | Some(b"heix") | Some(b"mif1") | Some(b"msf1") | Some(b"avif") => FileKind::Heif,
            _ => FileKind::IsoBmff,
        },
        _ if starts(b"%PDF-") => FileKind::Pdf,
        _ if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") => FileKind::Zip,
        _ if starts(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) => FileKind::Cfb,
        _ if starts(b"7z\xBC\xAF\x27\x1C") => FileKind::SevenZip,
        _ if starts(b"Rar!\x1A\x07") => FileKind::Rar,
        _ if starts(&[0x1F, 0x8B]) => FileKind::Gzip,
        _ if starts(&[0x28, 0xB5, 0x2F, 0xFD]) => FileKind::Zstd,
        _ if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0]) => FileKind::Xz,
        _ if starts(b"BZh") => FileKind::Bzip2,
        _ if starts(b"ID3") || (h.len() >= 2 && h[0] == 0xFF && h[1] & 0xE0 == 0xE0) => FileKind::Mp3,
        _ if starts(b"fLaC") => FileKind::Flac,
        _ if starts(b"OggS") => FileKind::Ogg,
        _ if starts(&[0x1A, 0x45, 0xDF, 0xA3]) => FileKind::Matroska,
        _ if starts(b"MZ") => FileKind::Exe,
        _ if starts(b"\x7FELF") => FileKind::Elf,
        _ => return None,
    })
}

/// Kinds a lower-case extension is expected to contain; empty when the extension is not checked.
pub fn expected_kinds(ext: &str) -> &'static [FileKind] {
    use FileKind::*;
    match ext {
        "jpg" | "jpeg" | "jfif" => &[Jpeg],
        "png" => &[Png],
        "gif" => &[Gif],
        "bmp" => &[Bmp],
        "tif" | "tiff" => &[Tiff],
        "webp" => &[Webp],
        "heic" | "heif" | "avif" => &[Heif],
        "pdf" => &[Pdf],
        "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "apk" | "cbz" => &[Zip],
        "doc" | "xls" | "ppt" | "msi" => &[Cfb],
        "7z" => &[SevenZip],
        "rar" => &[Rar],
        "gz" | "tgz" => &[Gzip],
        "zst" => &[Zstd],
        "xz" => &[Xz],
        "bz2" => &[Bzip2],
        "mp3" => &[Mp3],
        "flac" => &[Flac],
        "ogg" | "opus" => &[Ogg],
        "wav" => &[Wav],
        "mp4" | "m4a" | "m4v" | "mov" => &[IsoBmff],
        "mkv" | "webm" => &[Matroska],
        "exe" | "dll" => &[Exe],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_kinds() {
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some(FileKind::Jpeg));
        assert_eq!(sniff(b"%PDF-1.7"), Some(FileKind::Pdf));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), Some(FileKind::Heif));
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), Some(FileKind::IsoBmff));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(expected_kinds("docx"), &[FileKind::Zip]);
        assert!(expected_kinds("txt").is_empty());
    }
}
