//! Parsing of `FSCTL_ENUM_USN_DATA` output and path reconstruction from the
//! records (SCAN-03). Pure functions over bytes, so they are unit-tested on
//! every platform with hand-built buffers. Owner: W2-A.
//!
//! Output buffer of `FSCTL_ENUM_USN_DATA`: a `u64` (the next
//! `StartFileReferenceNumber`) followed by back-to-back `USN_RECORD`s, each
//! 8-byte aligned and `RecordLength` long. Layouts (winioctl.h):
//!
//! ```text
//! USN_RECORD_V2                         USN_RECORD_V3
//!  0 u32 RecordLength                    0 u32 RecordLength
//!  4 u16 MajorVersion (2)                4 u16 MajorVersion (3)
//!  6 u16 MinorVersion                    6 u16 MinorVersion
//!  8 u64 FileReferenceNumber             8 [u8;16] FileReferenceNumber
//! 16 u64 ParentFileReferenceNumber      24 [u8;16] ParentFileReferenceNumber
//! 24 i64 Usn                            40 i64 Usn
//! 32 i64 TimeStamp                      48 i64 TimeStamp
//! 40 u32 Reason                         56 u32 Reason
//! 44 u32 SourceInfo                     60 u32 SourceInfo
//! 48 u32 SecurityId                     64 u32 SecurityId
//! 52 u32 FileAttributes                 68 u32 FileAttributes
//! 56 u16 FileNameLength (bytes)         72 u16 FileNameLength (bytes)
//! 58 u16 FileNameOffset                 74 u16 FileNameOffset
//! 60 WCHAR FileName[]                   76 WCHAR FileName[]
//! ```

use std::collections::{HashMap, HashSet};

use mj_core::{Error, Result};

/// One parsed USN record (what MFT enumeration needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnRecord {
    pub major: u16,
    /// File reference (V2: 64-bit, zero-extended).
    pub file_ref: u128,
    pub parent_ref: u128,
    pub attributes: u32,
    /// File name, UTF-16 code units.
    pub name: Vec<u16>,
}

/// Smallest valid V2 record (header + one WCHAR, rounded to 8).
const MIN_V2: usize = 64;
const MIN_V3: usize = 80;

fn rd_u16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn rd_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn rd_u64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}
fn rd_u128(b: &[u8], o: usize) -> Option<u128> {
    Some(u128::from_le_bytes(b.get(o..o + 16)?.try_into().ok()?))
}

fn bad(msg: impl Into<String>) -> Error {
    Error::parse("USN record", msg)
}

/// Parse one record that starts at `rec[0]` and is exactly `rec.len()` bytes.
/// Returns `Ok(None)` for record versions we do not use (e.g. V4 range records).
pub fn parse_record(rec: &[u8]) -> Result<Option<UsnRecord>> {
    let major = rd_u16(rec, 4).ok_or_else(|| bad("truncated header"))?;
    let (refs, attrs_at, name_len_at, min) = match major {
        2 => (8usize, 52usize, 56usize, MIN_V2),
        3 => (8, 68, 72, MIN_V3),
        _ => return Ok(None),
    };
    if rec.len() < min {
        return Err(bad(format!("V{major} record of {} bytes is too short", rec.len())));
    }
    let (file_ref, parent_ref) = if major == 2 {
        (u128::from(rd_u64(rec, refs).unwrap_or(0)), u128::from(rd_u64(rec, refs + 8).unwrap_or(0)))
    } else {
        (rd_u128(rec, refs).unwrap_or(0), rd_u128(rec, refs + 16).unwrap_or(0))
    };
    let attributes = rd_u32(rec, attrs_at).ok_or_else(|| bad("truncated"))?;
    let name_len = usize::from(rd_u16(rec, name_len_at).ok_or_else(|| bad("truncated"))?);
    let name_off = usize::from(rd_u16(rec, name_len_at + 2).ok_or_else(|| bad("truncated"))?);
    if !name_len.is_multiple_of(2) || name_off < name_len_at + 4 || name_off + name_len > rec.len() {
        return Err(bad("file name outside the record"));
    }
    let name = rec[name_off..name_off + name_len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Ok(Some(UsnRecord { major, file_ref, parent_ref, attributes, name }))
}

/// Parse one `FSCTL_ENUM_USN_DATA` output buffer (`len` = bytes returned).
/// Returns the next start reference and the records. Malformed data is an
/// error, never a panic or an endless loop.
pub fn parse_enum_buffer(buf: &[u8]) -> Result<(u64, Vec<UsnRecord>)> {
    let next = rd_u64(buf, 0).ok_or_else(|| bad("buffer shorter than 8 bytes"))?;
    let mut out = Vec::new();
    let mut pos = 8usize;
    while pos < buf.len() {
        let len = rd_u32(buf, pos).ok_or_else(|| bad("truncated record length"))? as usize;
        if len < MIN_V2 || !len.is_multiple_of(8) || pos + len > buf.len() {
            return Err(bad(format!("record length {len} at offset {pos} is invalid")));
        }
        if let Some(r) = parse_record(&buf[pos..pos + len])? {
            out.push(r);
        }
        pos += len;
    }
    Ok((next, out))
}

/// One entry reachable from the scan root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Index into the record slice.
    pub record: usize,
    /// Path components below the root, root excluded.
    pub components: Vec<usize>,
}

/// Every record below `root_ref`, breadth first, with its chain of record
/// indexes from the root (so callers build paths without recursion).
/// `descend(i)` says whether record `i` (a directory) may be entered: links
/// and reparse points are listed but never entered. Cycles and duplicate
/// references are ignored; `max_entries` caps the output.
pub fn resolve_below(
    records: &[UsnRecord],
    root_ref: u128,
    descend: &dyn Fn(usize) -> bool,
    max_entries: usize,
) -> Vec<Resolved> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let mut children: HashMap<u128, Vec<usize>> = HashMap::new();
    let mut seen_ref: HashSet<u128> = HashSet::new();
    for (i, r) in records.iter().enumerate() {
        // The volume root's own record names itself as parent; never a child.
        if r.file_ref == r.parent_ref || !seen_ref.insert(r.file_ref) {
            continue;
        }
        children.entry(r.parent_ref).or_default().push(i);
    }
    let mut out: Vec<Resolved> = Vec::new();
    let mut visited: HashSet<u128> = HashSet::from([root_ref]);
    let mut queue: std::collections::VecDeque<(u128, Vec<usize>)> = std::collections::VecDeque::new();
    queue.push_back((root_ref, Vec::new()));
    while let Some((dir_ref, chain)) = queue.pop_front() {
        let Some(kids) = children.get(&dir_ref) else { continue };
        for &k in kids {
            if out.len() >= max_entries {
                return out;
            }
            let mut c = chain.clone();
            c.push(k);
            let r = &records[k];
            if r.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && descend(k) && visited.insert(r.file_ref) {
                queue.push_back((r.file_ref, c.clone()));
            }
            out.push(Resolved { record: k, components: c });
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a V2 record (tests only).
    pub fn v2(file_ref: u64, parent: u64, attrs: u32, name: &str) -> Vec<u8> {
        let name: Vec<u16> = name.encode_utf16().collect();
        let len = (60 + name.len() * 2).div_ceil(8) * 8;
        let mut b = vec![0u8; len];
        b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        b[4..6].copy_from_slice(&2u16.to_le_bytes());
        b[8..16].copy_from_slice(&file_ref.to_le_bytes());
        b[16..24].copy_from_slice(&parent.to_le_bytes());
        b[52..56].copy_from_slice(&attrs.to_le_bytes());
        b[56..58].copy_from_slice(&((name.len() * 2) as u16).to_le_bytes());
        b[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (i, c) in name.iter().enumerate() {
            b[60 + i * 2..62 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        b
    }

    fn v3(file_ref: u128, parent: u128, attrs: u32, name: &str) -> Vec<u8> {
        let name: Vec<u16> = name.encode_utf16().collect();
        let len = (76 + name.len() * 2).div_ceil(8) * 8;
        let mut b = vec![0u8; len];
        b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        b[4..6].copy_from_slice(&3u16.to_le_bytes());
        b[8..24].copy_from_slice(&file_ref.to_le_bytes());
        b[24..40].copy_from_slice(&parent.to_le_bytes());
        b[68..72].copy_from_slice(&attrs.to_le_bytes());
        b[72..74].copy_from_slice(&((name.len() * 2) as u16).to_le_bytes());
        b[74..76].copy_from_slice(&76u16.to_le_bytes());
        for (i, c) in name.iter().enumerate() {
            b[76 + i * 2..78 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        b
    }

    pub fn buffer(next: u64, recs: &[Vec<u8>]) -> Vec<u8> {
        let mut b = next.to_le_bytes().to_vec();
        for r in recs {
            b.extend_from_slice(r);
        }
        b
    }

    fn name(r: &UsnRecord) -> String {
        String::from_utf16_lossy(&r.name)
    }

    #[test]
    fn parses_v2_and_v3_records() {
        let buf = buffer(99, &[v2(0x0001_0000_0000_0010, 5, 0x20, "a.txt"), v3(1 << 70, 7, 0x10, "Dossier é")]);
        let (next, recs) = parse_enum_buffer(&buf).unwrap();
        assert_eq!(next, 99);
        assert_eq!(recs.len(), 2);
        assert_eq!((recs[0].major, recs[0].file_ref, recs[0].parent_ref), (2, 0x0001_0000_0000_0010, 5));
        assert_eq!((recs[0].attributes, name(&recs[0])), (0x20, "a.txt".to_string()));
        assert_eq!((recs[1].major, recs[1].file_ref, recs[1].parent_ref), (3, 1 << 70, 7));
        assert_eq!(name(&recs[1]), "Dossier é");
        // An empty enumeration result is just the next reference.
        assert_eq!(parse_enum_buffer(&7u64.to_le_bytes()).unwrap(), (7, vec![]));
    }

    #[test]
    fn rejects_malformed_buffers_without_panicking() {
        assert!(parse_enum_buffer(&[1, 2, 3]).is_err());
        let good = v2(16, 5, 0, "x");
        // Zero length (would loop forever), unaligned, overrunning.
        for len in [0u32, 13, 4096] {
            let mut r = good.clone();
            r[0..4].copy_from_slice(&len.to_le_bytes());
            assert!(parse_enum_buffer(&buffer(0, &[r])).is_err(), "len {len}");
        }
        // Name outside the record.
        let mut r = good.clone();
        r[56..58].copy_from_slice(&200u16.to_le_bytes());
        assert!(parse_enum_buffer(&buffer(0, &[r])).is_err());
        // Odd name length.
        let mut r = good.clone();
        r[56..58].copy_from_slice(&1u16.to_le_bytes());
        assert!(parse_enum_buffer(&buffer(0, &[r])).is_err());
        // Truncated buffer.
        let b = buffer(0, std::slice::from_ref(&good));
        assert!(parse_enum_buffer(&b[..b.len() - 4]).is_err());
        // Unknown version: skipped, not an error.
        let mut r = good;
        r[4..6].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(parse_enum_buffer(&buffer(0, &[r])).unwrap().1.len(), 0);
    }

    #[test]
    fn resolves_paths_below_root_without_entering_links() {
        // 5 = volume root; 10 = Users (root of scan); 11 = me; 12 = file; 13 = junction; 14 = under junction;
        // 20 = elsewhere; 21 = a cycle pair.
        let recs: Vec<UsnRecord> = [
            v2(5, 5, 0x10, "."),
            v2(10, 5, 0x10, "Users"),
            v2(11, 10, 0x10, "me"),
            v2(12, 11, 0x20, "f.txt"),
            v2(13, 11, 0x10 | 0x400, "link"),
            v2(14, 13, 0x20, "hidden-behind-link"),
            v2(20, 5, 0x10, "Other"),
            v2(21, 22, 0x10, "c1"),
            v2(22, 21, 0x10, "c2"),
            v2(12, 11, 0x20, "f.txt"),
        ]
        .iter()
        .map(|b| parse_record(b).unwrap().unwrap())
        .collect();
        let descend = |i: usize| recs[i].attributes & 0x400 == 0;
        let got = resolve_below(&recs, 10, &descend, usize::MAX);
        let paths: Vec<String> =
            got.iter().map(|r| r.components.iter().map(|&i| name(&recs[i])).collect::<Vec<_>>().join("/")).collect();
        assert_eq!(paths, vec!["me", "me/f.txt", "me/link"]);
        assert_eq!(resolve_below(&recs, 10, &descend, 2).len(), 2);
        // The whole volume: the root's self-parent record is not a child, cycles are unreachable.
        let all = resolve_below(&recs, 5, &|_| true, usize::MAX);
        assert_eq!(all.len(), 6);
    }
}
