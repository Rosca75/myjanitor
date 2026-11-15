//! Content-type detection from the first bytes of a file (CLN-CNT-02,
//! OPT-ENG-02). Owner: W2-C.
//!
//! [`sniff`] is a pure function over a byte prefix: it never reads a file, never
//! panics on any input (every access is bounds-checked) and recognises the
//! formats MyJanitor cares about by their magic numbers. ZIP containers are
//! refined into OOXML / ODF / EPUB / JAR by looking at the entry names inside
//! the prefix, which is cheap and needs no decompression.
//!
//! [`check_extension`] answers "does this header fit the file's extension?"
//! (the CLN-CNT-02 true-type-versus-extension check) and the `read_prefix` /
//! `check_file` helpers read a *bounded* prefix from disk for callers that only
//! have a path.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Bytes callers should read for [`sniff`] (enough for everything but ISO images).
pub const SNIFF_LEN: usize = 4096;

/// Bytes needed to recognise an ISO 9660 image (the `CD001` marker sits at 0x8001..0x9001).
pub const ISO_SNIFF_LEN: usize = 0x9006;

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
    /// ZIP container that is none of the refined kinds below (plain archive, APK...).
    Zip,
    /// OOXML Word document (`word/` part).
    Docx,
    /// OOXML Excel workbook (`xl/` part).
    Xlsx,
    /// OOXML PowerPoint presentation (`ppt/` part).
    Pptx,
    /// OpenDocument (text, sheet, presentation, drawing).
    Odf,
    /// EPUB book.
    Epub,
    /// Java archive (`META-INF/`).
    Jar,
    /// OLE compound file (legacy Office, encrypted OOXML, MSI).
    Cfb,
    SevenZip,
    Rar,
    Gzip,
    Zstd,
    Xz,
    Bzip2,
    Tar,
    Mp3,
    Flac,
    Ogg,
    Wav,
    Aiff,
    Avi,
    /// ISO BMFF (MP4, MOV, M4A, 3GP by brand).
    IsoBmff,
    Matroska,
    Exe,
    Elf,
    Sqlite,
    /// ISO 9660 disc image.
    Iso,
    Ttf,
    Otf,
    Woff,
    Woff2,
}

impl FileKind {
    /// Short human label for the UI ("PNG image", "Word document").
    pub fn label(self) -> &'static str {
        use FileKind::*;
        match self {
            Jpeg => "JPEG image",
            Png => "PNG image",
            Gif => "GIF image",
            Bmp => "BMP image",
            Tiff => "TIFF image",
            Webp => "WebP image",
            Heif => "HEIF/AVIF image",
            Pdf => "PDF document",
            Zip => "ZIP archive",
            Docx => "Word document",
            Xlsx => "Excel workbook",
            Pptx => "PowerPoint presentation",
            Odf => "OpenDocument file",
            Epub => "EPUB book",
            Jar => "Java archive",
            Cfb => "OLE compound file",
            SevenZip => "7-Zip archive",
            Rar => "RAR archive",
            Gzip => "gzip file",
            Zstd => "Zstandard file",
            Xz => "XZ file",
            Bzip2 => "bzip2 file",
            Tar => "TAR archive",
            Mp3 => "MP3 audio",
            Flac => "FLAC audio",
            Ogg => "Ogg media",
            Wav => "WAV audio",
            Aiff => "AIFF audio",
            Avi => "AVI video",
            IsoBmff => "MP4/QuickTime media",
            Matroska => "Matroska/WebM video",
            Exe => "Windows executable",
            Elf => "ELF executable",
            Sqlite => "SQLite database",
            Iso => "ISO disc image",
            Ttf => "TrueType font",
            Otf => "OpenType font",
            Woff => "WOFF font",
            Woff2 => "WOFF2 font",
        }
    }
}

/// Detect the kind from a file header; `None` when unknown. Never panics.
pub fn sniff(h: &[u8]) -> Option<FileKind> {
    let starts = |m: &[u8]| h.starts_with(m);
    let at = |off: usize, m: &[u8]| h.get(off..off + m.len()) == Some(m);
    Some(match () {
        _ if starts(&[0xFF, 0xD8, 0xFF]) => FileKind::Jpeg,
        _ if starts(b"\x89PNG\r\n\x1a\n") => FileKind::Png,
        _ if starts(b"GIF87a") || starts(b"GIF89a") => FileKind::Gif,
        _ if is_bmp(h) => FileKind::Bmp,
        _ if starts(b"II*\0") || starts(b"MM\0*") => FileKind::Tiff,
        _ if starts(b"RIFF") && at(8, b"WEBP") => FileKind::Webp,
        _ if starts(b"RIFF") && at(8, b"WAVE") => FileKind::Wav,
        _ if starts(b"RIFF") && at(8, b"AVI ") => FileKind::Avi,
        _ if starts(b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) => FileKind::Aiff,
        _ if at(4, b"ftyp") => match h.get(8..12) {
            Some(b"heic") | Some(b"heix") | Some(b"heim") | Some(b"heis") | Some(b"hevc") | Some(b"hevx")
            | Some(b"mif1") | Some(b"msf1") | Some(b"avif") | Some(b"avis") => FileKind::Heif,
            _ => FileKind::IsoBmff,
        },
        // Old QuickTime files start with a bare atom instead of `ftyp`.
        _ if at(4, b"moov") || at(4, b"mdat") || at(4, b"wide") => FileKind::IsoBmff,
        _ if starts(b"%PDF-") => FileKind::Pdf,
        _ if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") || starts(b"PK\x07\x08") => refine_zip(h),
        _ if starts(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) => FileKind::Cfb,
        _ if starts(b"7z\xBC\xAF\x27\x1C") => FileKind::SevenZip,
        _ if starts(b"Rar!\x1A\x07") => FileKind::Rar,
        _ if starts(&[0x1F, 0x8B, 0x08]) => FileKind::Gzip,
        _ if starts(&[0x28, 0xB5, 0x2F, 0xFD]) => FileKind::Zstd,
        _ if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0]) => FileKind::Xz,
        _ if starts(b"BZh") && matches!(h.get(3), Some(b'1'..=b'9')) => FileKind::Bzip2,
        _ if at(257, b"ustar") => FileKind::Tar,
        _ if starts(b"SQLite format 3\0") => FileKind::Sqlite,
        _ if starts(b"ID3") || is_mp3_frame(h) => FileKind::Mp3,
        _ if starts(b"fLaC") => FileKind::Flac,
        _ if starts(b"OggS") => FileKind::Ogg,
        _ if starts(&[0x1A, 0x45, 0xDF, 0xA3]) => FileKind::Matroska,
        _ if starts(b"MZ") => FileKind::Exe,
        _ if starts(b"\x7FELF") => FileKind::Elf,
        _ if starts(b"wOFF") => FileKind::Woff,
        _ if starts(b"wOF2") => FileKind::Woff2,
        _ if starts(b"OTTO") => FileKind::Otf,
        _ if starts(b"ttcf") || starts(b"true") || is_truetype(h) => FileKind::Ttf,
        _ if at(0x8001, b"CD001") || at(0x8801, b"CD001") || at(0x9001, b"CD001") => FileKind::Iso,
        _ => return None,
    })
}

/// BMP: `BM`, reserved words zero, a known DIB header size. Stricter than the
/// two-byte magic alone, which matches plenty of text files.
fn is_bmp(h: &[u8]) -> bool {
    if !h.starts_with(b"BM") || h.len() < 18 || h[6..10] != [0, 0, 0, 0] {
        return false;
    }
    let dib = u32::from_le_bytes([h[14], h[15], h[16], h[17]]);
    matches!(dib, 12 | 16 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// An MPEG audio frame header: 11 sync bits, plausible version/layer/bitrate/rate.
fn is_mp3_frame(h: &[u8]) -> bool {
    h.len() >= 4
        && h[0] == 0xFF
        && h[1] & 0xE0 == 0xE0
        && (h[1] >> 3) & 3 != 1 // version "reserved"
        && (h[1] >> 1) & 3 != 0 // layer "reserved" (also excludes AAC/ADTS)
        && h[2] >> 4 != 0xF // bitrate "bad"
        && (h[2] >> 2) & 3 != 3 // sample rate "reserved"
}

/// TrueType: version 1.0 then a sane table count.
fn is_truetype(h: &[u8]) -> bool {
    h.starts_with(&[0, 1, 0, 0]) && h.len() >= 12 && (1..=100).contains(&u16::from_be_bytes([h[4], h[5]]))
}

/// Walk the ZIP local-file headers that fit in the prefix and decide what the
/// container is. Entry names are the only thing inspected; nothing is inflated.
fn refine_zip(h: &[u8]) -> FileKind {
    // ODF and EPUB store a `mimetype` entry first, uncompressed, at offset 30.
    if h.get(30..38) == Some(b"mimetype") {
        let rest = &h[38..];
        if rest.starts_with(b"application/epub+zip") {
            return FileKind::Epub;
        }
        if rest.starts_with(b"application/vnd.oasis.opendocument.") {
            return FileKind::Odf;
        }
    }
    let mut jar = false;
    let mut pos = 0usize;
    while pos + 30 <= h.len() {
        // Find the next local file header signature.
        let Some(rel) = h[pos..].windows(4).position(|w| w == b"PK\x03\x04") else { break };
        let at = pos + rel;
        pos = at + 4;
        let Some(hdr) = h.get(at..at + 30) else { break };
        let name_len = u16::from_le_bytes([hdr[26], hdr[27]]) as usize;
        let Some(name) = h.get(at + 30..at + 30 + name_len) else { continue };
        if name.starts_with(b"word/") {
            return FileKind::Docx;
        }
        if name.starts_with(b"xl/") {
            return FileKind::Xlsx;
        }
        if name.starts_with(b"ppt/") {
            return FileKind::Pptx;
        }
        if name.starts_with(b"META-INF/") {
            jar = true;
        }
    }
    if jar {
        FileKind::Jar
    } else {
        FileKind::Zip
    }
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
        "zip" | "cbz" | "apk" | "xpi" | "msix" | "appx" | "vsix" | "nupkg" => &[Zip, Docx, Xlsx, Pptx, Odf, Epub, Jar],
        "docx" | "docm" | "dotx" => &[Docx, Zip],
        "xlsx" | "xlsm" | "xlsb" | "xltx" => &[Xlsx, Zip],
        "pptx" | "pptm" | "ppsx" | "potx" => &[Pptx, Zip],
        "odt" | "ods" | "odp" | "odg" => &[Odf, Zip],
        "epub" => &[Epub, Zip],
        "jar" | "war" => &[Jar, Zip],
        "doc" | "xls" | "ppt" | "msi" => &[Cfb],
        "7z" => &[SevenZip],
        "rar" => &[Rar],
        "gz" | "tgz" => &[Gzip],
        "zst" => &[Zstd],
        "xz" => &[Xz],
        "bz2" => &[Bzip2],
        "tar" => &[Tar],
        "mp3" => &[Mp3],
        "flac" => &[Flac],
        "ogg" | "opus" | "oga" => &[Ogg],
        "wav" => &[Wav],
        "aif" | "aiff" => &[Aiff],
        "avi" => &[Avi],
        "mp4" | "m4a" | "m4v" | "mov" | "3gp" => &[IsoBmff],
        "mkv" | "webm" | "mka" => &[Matroska],
        "exe" | "dll" | "sys" | "ocx" | "scr" => &[Exe],
        "sqlite" | "sqlite3" => &[Sqlite],
        "iso" => &[Iso],
        "ttf" => &[Ttf],
        "otf" => &[Otf, Ttf],
        "woff" => &[Woff],
        "woff2" => &[Woff2],
        _ => &[],
    }
}

/// How many header bytes are needed to judge `ext` (more for ISO images).
pub fn sniff_len_for_ext(ext: &str) -> usize {
    if ext == "iso" {
        ISO_SNIFF_LEN
    } else {
        SNIFF_LEN
    }
}

/// Outcome of comparing a header with an extension (CLN-CNT-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The extension is not checked, the file is empty or the header is too short to judge.
    Unchecked,
    /// The content fits the extension.
    Match,
    /// The content is something else (`actual` is `None` when it is unrecognised).
    Mismatch { expected: &'static [FileKind], actual: Option<FileKind> },
}

impl Verdict {
    /// True for [`Verdict::Mismatch`].
    pub fn is_mismatch(self) -> bool {
        matches!(self, Verdict::Mismatch { .. })
    }
}

/// Compare a header with a lower-case extension.
///
/// Unrecognised content under a checked extension is a mismatch (a `.jpg` that
/// is really an HTML error page is the typical case), except for `.mp3`, whose
/// files may legally start with junk before the first frame.
pub fn check_extension(ext: &str, header: &[u8]) -> Verdict {
    let expected = expected_kinds(ext);
    if expected.is_empty() || header.is_empty() {
        return Verdict::Unchecked;
    }
    match sniff(header) {
        Some(k) if expected.contains(&k) => Verdict::Match,
        Some(k) => Verdict::Mismatch { expected, actual: Some(k) },
        None if expected.contains(&FileKind::Iso) && header.len() < ISO_SNIFF_LEN => Verdict::Unchecked,
        None if ext == "mp3" || header.len() < 4 => Verdict::Unchecked,
        None => Verdict::Mismatch { expected, actual: None },
    }
}

/// Read at most `len` bytes from the start of a file (a bounded read: never the whole file).
pub fn read_prefix(path: &Path, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(len.min(SNIFF_LEN));
    File::open(path)?.take(len as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Sniff a file on disk by reading a bounded prefix.
pub fn sniff_file(path: &Path) -> io::Result<Option<FileKind>> {
    Ok(sniff(&read_prefix(path, SNIFF_LEN)?))
}

/// [`check_extension`] for a file on disk, reading only as much as that extension needs.
pub fn check_file(path: &Path, ext: &str) -> io::Result<Verdict> {
    if expected_kinds(ext).is_empty() {
        return Ok(Verdict::Unchecked);
    }
    Ok(check_extension(ext, &read_prefix(path, sniff_len_for_ext(ext))?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    /// A minimal ZIP local header for `name` followed by no data.
    fn zip_entry(name: &str) -> Vec<u8> {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(name.as_bytes());
        v
    }

    fn bmp() -> Vec<u8> {
        let mut v = b"BM".to_vec();
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&[0; 20]);
        v
    }

    #[test]
    fn detects_common_kinds() {
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some(FileKind::Jpeg));
        assert_eq!(sniff(b"%PDF-1.7"), Some(FileKind::Pdf));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), Some(FileKind::Heif));
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), Some(FileKind::IsoBmff));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(expected_kinds("docx"), &[FileKind::Docx, FileKind::Zip]);
        assert!(expected_kinds("txt").is_empty());
    }

    #[test]
    fn images_audio_video_and_archives() {
        let cases: Vec<(Vec<u8>, FileKind)> = vec![
            (b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec(), FileKind::Png),
            (b"GIF89a\x01\0".to_vec(), FileKind::Gif),
            (bmp(), FileKind::Bmp),
            (b"II*\0\x08\0\0\0".to_vec(), FileKind::Tiff),
            (b"MM\0*\0\0\0\x08".to_vec(), FileKind::Tiff),
            (b"RIFF\x24\0\0\0WEBPVP8 ".to_vec(), FileKind::Webp),
            (b"RIFF\x24\0\0\0WAVEfmt ".to_vec(), FileKind::Wav),
            (b"RIFF\x24\0\0\0AVI LIST".to_vec(), FileKind::Avi),
            (b"FORM\0\0\0\x24AIFFCOMM".to_vec(), FileKind::Aiff),
            (b"ID3\x04\0\0\0\0\0\0".to_vec(), FileKind::Mp3),
            (vec![0xFF, 0xFB, 0x90, 0x64], FileKind::Mp3),
            (b"fLaC\0\0\0\x22".to_vec(), FileKind::Flac),
            (b"OggS\0\x02".to_vec(), FileKind::Ogg),
            (b"\0\0\0\x20ftypM4A \0\0".to_vec(), FileKind::IsoBmff),
            (b"\0\0\0\x14ftypqt  \0\0\0\0".to_vec(), FileKind::IsoBmff),
            (b"\0\0\0\x1cftypavif\0\0".to_vec(), FileKind::Heif),
            (vec![0x1A, 0x45, 0xDF, 0xA3, 0x93, 0x42, 0x82, 0x88, b'm', b'a', b't', b'r'], FileKind::Matroska),
            (b"7z\xBC\xAF\x27\x1C\0\x04".to_vec(), FileKind::SevenZip),
            (b"Rar!\x1A\x07\x01\0".to_vec(), FileKind::Rar),
            (vec![0x1F, 0x8B, 0x08, 0, 0, 0], FileKind::Gzip),
            (b"BZh91AY&SY".to_vec(), FileKind::Bzip2),
            (vec![0xFD, b'7', b'z', b'X', b'Z', 0, 0, 4], FileKind::Xz),
            (vec![0x28, 0xB5, 0x2F, 0xFD, 0x04], FileKind::Zstd),
            (b"SQLite format 3\0\x10\0".to_vec(), FileKind::Sqlite),
            (b"MZ\x90\0\x03\0\0\0".to_vec(), FileKind::Exe),
            (b"\x7FELF\x02\x01\x01".to_vec(), FileKind::Elf),
            (vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0], FileKind::Cfb),
            (b"wOFF\0\x01\0\0".to_vec(), FileKind::Woff),
            (b"wOF2\0\x01\0\0".to_vec(), FileKind::Woff2),
            (b"OTTO\0\x0c\0\x80\0\x03\0\x30".to_vec(), FileKind::Otf),
            (vec![0, 1, 0, 0, 0, 12, 0, 0x80, 0, 3, 0, 0x30], FileKind::Ttf),
        ];
        for (bytes, kind) in cases {
            assert_eq!(sniff(&bytes), Some(kind), "{kind:?}");
        }
    }

    #[test]
    fn tar_and_iso_need_their_offsets() {
        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff(&tar), Some(FileKind::Tar));
        let mut iso = vec![0u8; ISO_SNIFF_LEN];
        iso[0x8001..0x8006].copy_from_slice(b"CD001");
        assert_eq!(sniff(&iso), Some(FileKind::Iso));
        assert_eq!(sniff(&iso[..4096]), None);
        assert_eq!(check_extension("iso", &iso[..4096]), Verdict::Unchecked);
        assert_eq!(check_extension("iso", &iso), Verdict::Match);
    }

    #[test]
    fn zip_refinement() {
        let mut docx = zip_entry("[Content_Types].xml");
        docx.extend(zip_entry("word/document.xml"));
        assert_eq!(sniff(&docx), Some(FileKind::Docx));
        let mut xlsx = zip_entry("[Content_Types].xml");
        xlsx.extend(zip_entry("xl/workbook.xml"));
        assert_eq!(sniff(&xlsx), Some(FileKind::Xlsx));
        let mut pptx = zip_entry("ppt/presentation.xml");
        pptx.extend(zip_entry("x"));
        assert_eq!(sniff(&pptx), Some(FileKind::Pptx));
        let mut jar = zip_entry("META-INF/MANIFEST.MF");
        jar.extend(zip_entry("A.class"));
        assert_eq!(sniff(&jar), Some(FileKind::Jar));
        assert_eq!(sniff(&zip_entry("readme.txt")), Some(FileKind::Zip));
        let mut odt = zip_entry("mimetype");
        odt.extend_from_slice(b"application/vnd.oasis.opendocument.text");
        assert_eq!(sniff(&odt), Some(FileKind::Odf));
        let mut epub = zip_entry("mimetype");
        epub.extend_from_slice(b"application/epub+zip");
        assert_eq!(sniff(&epub), Some(FileKind::Epub));
        assert_eq!(sniff(b"PK\x05\x06\0\0\0\0"), Some(FileKind::Zip));
    }

    #[test]
    fn weak_magics_are_not_enough() {
        assert_eq!(sniff(b"BM is a text file starting with BM"), None);
        assert_eq!(sniff(&[0xFF, 0xE0, 0, 0]), None);
        assert_eq!(sniff(b"BZhello"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn extension_verdicts() {
        assert_eq!(check_extension("png", b"\x89PNG\r\n\x1a\n"), Verdict::Match);
        assert!(check_extension("jpg", b"\x89PNG\r\n\x1a\n").is_mismatch());
        // A .jpg that is really an HTML error page.
        assert_eq!(
            check_extension("jpg", b"<html><body>404</body></html>"),
            Verdict::Mismatch { expected: &[FileKind::Jpeg], actual: None }
        );
        // Plain zip accepted for .docx (cannot tell), a JAR is not.
        assert_eq!(check_extension("docx", &zip_entry("a.txt")), Verdict::Match);
        let mut jar = zip_entry("META-INF/MANIFEST.MF");
        jar.extend(zip_entry("A.class"));
        assert!(check_extension("docx", &jar).is_mismatch());
        assert_eq!(check_extension("txt", b"anything"), Verdict::Unchecked);
        assert_eq!(check_extension("png", b""), Verdict::Unchecked);
        assert_eq!(check_extension("mp3", b"junk junk junk"), Verdict::Unchecked);
    }

    #[test]
    fn reads_a_bounded_prefix_from_disk() {
        let dir = TempDir::new("sniff").unwrap();
        let big = [b"%PDF-1.4\n".as_slice(), &vec![b'x'; 100_000]].concat();
        let p = dir.write("a.pdf", &big).unwrap();
        assert_eq!(read_prefix(&p, 10).unwrap().len(), 10);
        assert_eq!(read_prefix(&p, 1 << 20).unwrap().len(), big.len());
        assert_eq!(sniff_file(&p).unwrap(), Some(FileKind::Pdf));
        assert_eq!(check_file(&p, "pdf").unwrap(), Verdict::Match);
        let q = dir.write("b.png", &big).unwrap();
        assert!(check_file(&q, "png").unwrap().is_mismatch());
        assert!(check_file(&dir.join("missing.png"), "png").is_err());
    }

    /// Deterministic fuzz: random bytes, with and without real magic prefixes,
    /// must never panic and `check_extension` must stay total.
    #[test]
    fn fuzz_random_bytes_never_panic() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let magics: [&[u8]; 6] = [b"PK\x03\x04", b"RIFF", b"\0\0\0\x18ftyp", b"BM", b"\x89PNG", b"MZ"];
        for i in 0..4000 {
            let len = (next() % 300) as usize;
            let mut buf: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if i % 2 == 0 {
                let m = magics[(next() % magics.len() as u64) as usize];
                let mut v = m.to_vec();
                v.extend_from_slice(&buf);
                buf = v;
            }
            let _ = sniff(&buf);
            for ext in ["png", "docx", "iso", "mp3", "zip"] {
                let _ = check_extension(ext, &buf);
            }
        }
    }
}
