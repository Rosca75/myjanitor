//! NTFS FILE record parsing (`FSCTL_GET_NTFS_FILE_RECORD` output), used by the
//! MFT fast path to get sizes and times without opening each file (SCAN-03).
//! Pure functions over bytes, unit-tested on every platform. Owner: W2-A.
//!
//! ```text
//! FILE record header                    attribute header (common)
//!  0 "FILE"                              0 u32 type (0xFFFFFFFF = end)
//!  4 u16 update sequence offset          4 u32 length
//!  6 u16 update sequence count           8 u8  non-resident
//! 16 u16 sequence number                 9 u8  name length (WCHARs)
//! 18 u16 hard-link count                12 u16 flags (0x1 compressed, 0x4000 encrypted, 0x8000 sparse)
//! 20 u16 first attribute offset         resident:     16 u32 value length, 20 u16 value offset
//! 22 u16 flags (0x1 in use, 0x2 dir)    non-resident: 16 u64 lowest VCN, 34 u16 compression unit,
//! 24 u32 bytes in use                                 40 u64 allocated, 48 u64 data size,
//! 32 u64 base record (0 = base)                       64 u64 compressed size (when compressed/sparse)
//! ```
//!
//! `$STANDARD_INFORMATION` (0x10): 0 created, 8 modified, 16 MFT changed,
//! 24 accessed (FILETIME), 32 u32 file attributes. Unnamed `$DATA` (0x80) holds
//! the file size. `$REPARSE_POINT` (0xC0) starts with the u32 tag.
//! `$ATTRIBUTE_LIST` (0x20) means some attributes live in other records; when
//! the unnamed `$DATA` is not in the base record the caller falls back to a
//! per-path stat.

use mj_core::{Error, Result};

const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;
const SECTOR: usize = 512;

/// What one FILE record says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NtfsFileInfo {
    pub in_use: bool,
    pub is_dir: bool,
    pub link_count: u16,
    /// Non-zero for extension records.
    pub base_record: u64,
    /// `$STANDARD_INFORMATION` times (FILETIME ticks) and attributes.
    pub created: Option<u64>,
    pub modified: Option<u64>,
    pub accessed: Option<u64>,
    pub attributes: Option<u32>,
    /// Unnamed `$DATA`: logical size, allocated size, and the on-disk size of a
    /// compressed or sparse stream. `None` when not in this record.
    pub data_size: Option<u64>,
    pub allocated: Option<u64>,
    pub compressed: Option<u64>,
    /// The unnamed `$DATA` is resident (stored inside the MFT record: no clusters).
    pub resident: bool,
    pub reparse_tag: Option<u32>,
    pub has_attribute_list: bool,
}

fn rd_u16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn rd_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn rd_u64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn bad(msg: impl Into<String>) -> Error {
    Error::parse("NTFS file record", msg)
}

/// Apply the update-sequence fixups in place. Records returned by
/// `FSCTL_GET_NTFS_FILE_RECORD` may already be fixed up: fixups are applied
/// only when every protected sector end carries the update sequence number,
/// and a record where some do and some do not is rejected as torn.
pub fn apply_fixups(rec: &mut [u8]) -> Result<()> {
    let off = usize::from(rd_u16(rec, 4).ok_or_else(|| bad("truncated header"))?);
    let count = usize::from(rd_u16(rec, 6).ok_or_else(|| bad("truncated header"))?);
    if count < 2 {
        return Ok(());
    }
    let sectors = count - 1;
    if off + count * 2 > rec.len() || sectors * SECTOR > rec.len() {
        return Err(bad("update sequence array out of range"));
    }
    let usn = [rec[off], rec[off + 1]];
    let ends: Vec<usize> = (0..sectors).map(|i| (i + 1) * SECTOR - 2).collect();
    let matching = ends.iter().filter(|&&e| rec[e..e + 2] == usn).count();
    if matching == 0 {
        return Ok(()); // already fixed up
    }
    if matching != sectors {
        return Err(bad("torn record (update sequence mismatch)"));
    }
    for (i, &e) in ends.iter().enumerate() {
        let src = off + 2 + i * 2;
        let orig = [rec[src], rec[src + 1]];
        rec[e..e + 2].copy_from_slice(&orig);
    }
    Ok(())
}

/// Parse a fixed-up FILE record. Errors on a bad signature or attribute chain;
/// never panics and never loops (every attribute must advance).
pub fn parse_file_record(rec: &[u8]) -> Result<NtfsFileInfo> {
    if rec.get(0..4) != Some(b"FILE") {
        return Err(bad("missing FILE signature"));
    }
    let flags = rd_u16(rec, 22).ok_or_else(|| bad("truncated header"))?;
    let mut info = NtfsFileInfo {
        in_use: flags & 0x1 != 0,
        is_dir: flags & 0x2 != 0,
        link_count: rd_u16(rec, 18).unwrap_or(0),
        base_record: rd_u64(rec, 32).ok_or_else(|| bad("truncated header"))?,
        ..Default::default()
    };
    let mut pos = usize::from(rd_u16(rec, 20).ok_or_else(|| bad("truncated header"))?);
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 512 {
            return Err(bad("too many attributes"));
        }
        let ty = rd_u32(rec, pos).ok_or_else(|| bad("attribute header out of range"))?;
        if ty == ATTR_END {
            break;
        }
        let len = rd_u32(rec, pos + 4).ok_or_else(|| bad("attribute header out of range"))? as usize;
        if len < 16 || pos + len > rec.len() {
            return Err(bad(format!("attribute 0x{ty:x} length {len} is invalid")));
        }
        let a = &rec[pos..pos + len];
        let non_resident = a[8] != 0;
        let name_len = a[9];
        let aflags = rd_u16(a, 12).unwrap_or(0);
        let value = |a: &[u8]| -> Option<(usize, usize)> {
            let vlen = rd_u32(a, 16)? as usize;
            let voff = usize::from(rd_u16(a, 20)?);
            (voff + vlen <= a.len()).then_some((voff, vlen))
        };
        match ty {
            ATTR_STANDARD_INFORMATION if !non_resident => {
                if let Some((o, l)) = value(a).filter(|&(_, l)| l >= 36) {
                    let v = &a[o..o + l];
                    info.created = rd_u64(v, 0);
                    info.modified = rd_u64(v, 8);
                    info.accessed = rd_u64(v, 24);
                    info.attributes = rd_u32(v, 32);
                }
            }
            ATTR_ATTRIBUTE_LIST => info.has_attribute_list = true,
            ATTR_DATA if name_len == 0 => {
                if non_resident {
                    // Only the first extent (lowest VCN 0) carries the sizes.
                    if rd_u64(a, 16) == Some(0) && a.len() >= 56 {
                        info.allocated = rd_u64(a, 40);
                        info.data_size = rd_u64(a, 48);
                        let packed = aflags & (0x0001 | 0x8000) != 0;
                        if packed && a.len() >= 72 {
                            info.compressed = rd_u64(a, 64);
                        }
                        info.resident = false;
                    }
                } else if let Some((_, l)) = value(a) {
                    info.data_size = Some(l as u64);
                    info.allocated = Some(0);
                    info.resident = true;
                }
            }
            // A non-resident reparse buffer leaves the tag unknown (the caller treats it as a link).
            ATTR_REPARSE_POINT if !non_resident => info.reparse_tag = value(a).and_then(|(o, _)| rd_u32(a, o)),
            _ => {}
        }
        pos += len;
    }
    Ok(info)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builder for hand-made FILE records (tests only).
    pub struct Rec {
        attrs: Vec<Vec<u8>>,
        flags: u16,
        links: u16,
        base: u64,
    }

    impl Rec {
        pub fn new(flags: u16) -> Rec {
            Rec { attrs: Vec::new(), flags, links: 1, base: 0 }
        }
        fn attr(mut self, ty: u32, non_res: bool, name_len: u8, aflags: u16, body: &[u8]) -> Rec {
            let mut a = vec![0u8; 16];
            a[0..4].copy_from_slice(&ty.to_le_bytes());
            a[8] = non_res as u8;
            a[9] = name_len;
            a[12..14].copy_from_slice(&aflags.to_le_bytes());
            a.extend_from_slice(body);
            while !a.len().is_multiple_of(8) {
                a.push(0);
            }
            let l = a.len() as u32;
            a[4..8].copy_from_slice(&l.to_le_bytes());
            self.attrs.push(a);
            self
        }
        fn resident(self, ty: u32, name_len: u8, value: &[u8]) -> Rec {
            // value length (16..20), value offset (20..22) = 24, then padding to 24.
            let mut body = vec![0u8; 8];
            body[0..4].copy_from_slice(&(value.len() as u32).to_le_bytes());
            body[4..6].copy_from_slice(&24u16.to_le_bytes());
            body.extend_from_slice(value);
            self.attr(ty, false, name_len, 0, &body)
        }
        pub fn std_info(self, created: u64, modified: u64, accessed: u64, attrs: u32) -> Rec {
            let mut v = vec![0u8; 48];
            v[0..8].copy_from_slice(&created.to_le_bytes());
            v[8..16].copy_from_slice(&modified.to_le_bytes());
            v[24..32].copy_from_slice(&accessed.to_le_bytes());
            v[32..36].copy_from_slice(&attrs.to_le_bytes());
            self.resident(0x10, 0, &v)
        }
        pub fn resident_data(self, len: usize) -> Rec {
            self.resident(0x80, 0, &vec![b'x'; len])
        }
        pub fn nonresident_data(self, allocated: u64, size: u64, compressed: Option<u64>) -> Rec {
            // Body starts at attribute offset 16: lowest VCN (16), highest VCN (24), runs offset (32),
            // compression unit (34), pad (36), allocated (40), size (48), initialized (56), compressed (64).
            let mut body = vec![0u8; if compressed.is_some() { 56 } else { 48 }];
            body[24..32].copy_from_slice(&allocated.to_le_bytes());
            body[32..40].copy_from_slice(&size.to_le_bytes());
            body[40..48].copy_from_slice(&size.to_le_bytes());
            if let Some(c) = compressed {
                body[48..56].copy_from_slice(&c.to_le_bytes());
            }
            let f = if compressed.is_some() { 0x8000 } else { 0 };
            self.attr(0x80, true, 0, f, &body)
        }
        pub fn named_data(self) -> Rec {
            self.resident(0x80, 4, b"stream")
        }
        pub fn reparse(self, tag: u32) -> Rec {
            self.resident(0xC0, 0, &tag.to_le_bytes())
        }
        pub fn attr_list(self) -> Rec {
            self.resident(0x20, 0, &[0u8; 8])
        }
        pub fn links(mut self, n: u16) -> Rec {
            self.links = n;
            self
        }
        pub fn base(mut self, b: u64) -> Rec {
            self.base = b;
            self
        }
        /// 1024-byte record with a valid update sequence (fixups NOT yet applied).
        pub fn build(self) -> Vec<u8> {
            let mut r = vec![0u8; 1024];
            r[0..4].copy_from_slice(b"FILE");
            r[4..6].copy_from_slice(&48u16.to_le_bytes());
            r[6..8].copy_from_slice(&3u16.to_le_bytes());
            r[18..20].copy_from_slice(&self.links.to_le_bytes());
            r[20..22].copy_from_slice(&56u16.to_le_bytes());
            r[22..24].copy_from_slice(&self.flags.to_le_bytes());
            r[32..40].copy_from_slice(&self.base.to_le_bytes());
            let mut pos = 56;
            for a in &self.attrs {
                r[pos..pos + a.len()].copy_from_slice(a);
                pos += a.len();
            }
            r[pos..pos + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            pos += 8;
            r[24..28].copy_from_slice(&(pos as u32).to_le_bytes());
            r[28..32].copy_from_slice(&1024u32.to_le_bytes());
            // Update sequence: number 0x0042, save sector ends, stamp them.
            r[48..50].copy_from_slice(&0x0042u16.to_le_bytes());
            for i in 0..2 {
                let end = (i + 1) * 512 - 2;
                let (a, b) = (r[end], r[end + 1]);
                r[50 + i * 2] = a;
                r[51 + i * 2] = b;
                r[end..end + 2].copy_from_slice(&0x0042u16.to_le_bytes());
            }
            r
        }
    }

    fn parse(mut r: Vec<u8>) -> NtfsFileInfo {
        apply_fixups(&mut r).unwrap();
        parse_file_record(&r).unwrap()
    }

    #[test]
    fn resident_file_with_standard_information() {
        let info = parse(Rec::new(0x1).std_info(10, 20, 30, 0x22).resident_data(300).links(2).build());
        assert!(info.in_use && !info.is_dir);
        assert_eq!(
            (info.created, info.modified, info.accessed, info.attributes),
            (Some(10), Some(20), Some(30), Some(0x22))
        );
        assert_eq!((info.data_size, info.allocated, info.resident), (Some(300), Some(0), true));
        assert_eq!(info.link_count, 2);
    }

    #[test]
    fn non_resident_compressed_data_and_named_streams() {
        let info = parse(
            Rec::new(0x1)
                .std_info(1, 2, 3, 0x820)
                .named_data()
                .nonresident_data(1 << 20, 1_000_000, Some(65_536))
                .build(),
        );
        assert_eq!((info.data_size, info.allocated, info.compressed), (Some(1_000_000), Some(1 << 20), Some(65_536)));
        assert!(!info.resident);
        let plain = parse(Rec::new(0x1).nonresident_data(8192, 5000, None).build());
        assert_eq!((plain.data_size, plain.allocated, plain.compressed), (Some(5000), Some(8192), None));
    }

    #[test]
    fn directories_reparse_points_and_attribute_lists() {
        let d = parse(Rec::new(0x3).std_info(0, 0, 0, 0x410).reparse(0xA000_0003).build());
        assert!(d.is_dir);
        assert_eq!(d.reparse_tag, Some(0xA000_0003));
        assert_eq!(d.data_size, None);
        let l = parse(Rec::new(0x1).attr_list().base(0).build());
        assert!(l.has_attribute_list);
        let ext = parse(Rec::new(0x1).base(0x0001_0000_0000_0042).build());
        assert_eq!(ext.base_record, 0x0001_0000_0000_0042);
    }

    #[test]
    fn fixups_are_applied_and_torn_records_rejected() {
        let mut r = Rec::new(0x1).resident_data(900).build();
        // Sector ends carry the USN before fixup.
        assert_eq!(&r[510..512], &0x0042u16.to_le_bytes());
        apply_fixups(&mut r).unwrap();
        assert_eq!(&r[510..512], b"xx");
        // Applying again is a no-op (already fixed up).
        let copy = r.clone();
        apply_fixups(&mut r).unwrap();
        assert_eq!(r, copy);
        let mut torn = Rec::new(0x1).resident_data(900).build();
        torn[1022..1024].copy_from_slice(&[1, 2]);
        assert!(apply_fixups(&mut torn).is_err());
    }

    #[test]
    fn malformed_records_are_errors_not_panics() {
        assert!(parse_file_record(b"FILX").is_err());
        assert!(parse_file_record(b"FILE").is_err());
        let mut r = Rec::new(0x1).resident_data(10).build();
        apply_fixups(&mut r).unwrap();
        // Zero-length attribute (would loop forever).
        let mut z = r.clone();
        z[60..64].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_file_record(&z).is_err());
        // Attribute running off the record.
        let mut o = r.clone();
        o[60..64].copy_from_slice(&5000u32.to_le_bytes());
        assert!(parse_file_record(&o).is_err());
        // First attribute offset beyond the record.
        let mut f = r;
        f[20..22].copy_from_slice(&2000u16.to_le_bytes());
        assert!(parse_file_record(&f).is_err());
        for n in 0..64 {
            let _ = parse_file_record(&Rec::new(0x1).std_info(1, 2, 3, 4).build()[..n]);
        }
    }
}
