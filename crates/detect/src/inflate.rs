//! A minimal DEFLATE / gzip *validator* (no output, no window): decodes every block of a gzip
//! member to prove the stream is structurally complete, then checks the trailer's length field.
//! Used by the broken-file detector to find truncated `.gz` files without a decompression crate.
//!
//! It is a port of the well-known `puff` reference decoder's structure (canonical Huffman decoding
//! bit by bit). Because no output is kept, a back-reference is only required to point inside the
//! bytes produced so far; the CRC-32 is not recomputed (the ISIZE comparison already catches
//! truncation and most corruption). Hostile input never panics: every read is bounds-checked and
//! returns `None`.

const MAXBITS: usize = 15;
const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Result of validating a gzip file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GzipVerdict {
    /// Header, every deflate block and the trailer are consistent.
    Complete,
    /// Not a gzip header at all.
    BadHeader,
    /// The stream is truncated or corrupt, with a short reason.
    Broken(&'static str),
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Bits { data, pos, buf: 0, cnt: 0 }
    }

    fn bits(&mut self, need: u32) -> Option<u32> {
        let mut val = self.buf;
        while self.cnt < need {
            let b = *self.data.get(self.pos)? as u32;
            self.pos += 1;
            val |= b << self.cnt;
            self.cnt += 8;
        }
        self.buf = if need >= 32 { 0 } else { val >> need };
        self.cnt -= need;
        Some(if need >= 32 { val } else { val & ((1u32 << need) - 1) })
    }

    /// Drop the bits left in the current byte.
    fn align(&mut self) {
        self.buf = 0;
        self.cnt = 0;
    }
}

struct Huff {
    count: [u16; MAXBITS + 1],
    symbol: Vec<u16>,
}

/// Canonical Huffman table from code lengths. `None` when over-subscribed; incomplete is allowed.
fn build(lengths: &[u8]) -> Option<Huff> {
    let mut count = [0u16; MAXBITS + 1];
    for &l in lengths {
        count[l as usize] += 1;
    }
    let mut left: i32 = 1;
    for &c in count.iter().skip(1) {
        left = (left << 1) - c as i32;
        if left < 0 {
            return None;
        }
    }
    let mut offs = [0u16; MAXBITS + 2];
    for len in 1..=MAXBITS {
        offs[len + 1] = offs[len] + count[len];
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (sym, &l) in lengths.iter().enumerate() {
        if l != 0 {
            symbol[offs[l as usize] as usize] = sym as u16;
            offs[l as usize] += 1;
        }
    }
    Some(Huff { count, symbol })
}

fn decode(b: &mut Bits<'_>, h: &Huff) -> Option<u16> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for len in 1..=MAXBITS {
        code |= b.bits(1)? as i32;
        let count = h.count[len] as i32;
        if code - count < first {
            return h.symbol.get((index + (code - first)) as usize).copied();
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    None
}

fn fixed_tables() -> (Huff, Huff) {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (build(&l).expect("fixed literal table"), build(&[5u8; 30]).expect("fixed distance table"))
}

fn dynamic_tables(b: &mut Bits<'_>) -> Option<(Huff, Huff)> {
    let nlen = b.bits(5)? as usize + 257;
    let ndist = b.bits(5)? as usize + 1;
    let ncode = b.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return None;
    }
    let mut cl = [0u8; 19];
    for &o in CL_ORDER.iter().take(ncode) {
        cl[o] = b.bits(3)? as u8;
    }
    let clh = build(&cl)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = decode(b, &clh)?;
        if sym < 16 {
            lengths[i] = sym as u8;
            i += 1;
            continue;
        }
        let (val, rep) = match sym {
            16 => (*lengths.get(i.checked_sub(1)?)?, 3 + b.bits(2)? as usize),
            17 => (0, 3 + b.bits(3)? as usize),
            _ => (0, 11 + b.bits(7)? as usize),
        };
        if i + rep > nlen + ndist {
            return None;
        }
        lengths[i..i + rep].fill(val);
        i += rep;
    }
    if lengths[256] == 0 {
        return None; // no end-of-block code
    }
    Some((build(&lengths[..nlen])?, build(&lengths[nlen..])?))
}

/// Decode one codes block; adds to `out`.
fn codes(b: &mut Bits<'_>, lit: &Huff, dist: &Huff, out: &mut u64) -> Option<()> {
    loop {
        let sym = decode(b, lit)? as usize;
        match sym {
            0..=255 => *out += 1,
            256 => return Some(()),
            257..=285 => {
                let s = sym - 257;
                let len = LEN_BASE[s] as u64 + b.bits(LEN_EXTRA[s] as u32)? as u64;
                let ds = decode(b, dist)? as usize;
                if ds >= 30 {
                    return None;
                }
                let d = DIST_BASE[ds] as u64 + b.bits(DIST_EXTRA[ds] as u32)? as u64;
                if d > *out {
                    return None; // reaches before the start of the data
                }
                *out += len;
            }
            _ => return None,
        }
    }
}

/// Decode all blocks of one deflate stream starting at `pos`; returns (end position, bytes produced).
fn inflate(data: &[u8], pos: usize) -> Option<(usize, u64)> {
    let mut b = Bits::new(data, pos);
    let mut out = 0u64;
    let fixed = fixed_tables();
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.align();
                let len = b.bits(16)?;
                let nlen = b.bits(16)?;
                if len != (!nlen & 0xffff) {
                    return None;
                }
                let end = b.pos.checked_add(len as usize)?;
                if end > data.len() {
                    return None;
                }
                b.pos = end;
                out += len as u64;
            }
            1 => codes(&mut b, &fixed.0, &fixed.1, &mut out)?,
            2 => {
                let (l, d) = dynamic_tables(&mut b)?;
                codes(&mut b, &l, &d, &mut out)?;
            }
            _ => return None,
        }
        if last == 1 {
            b.align();
            return Some((b.pos, out));
        }
    }
}

/// Skip the gzip header; returns the offset of the deflate data.
fn header_len(d: &[u8]) -> Option<usize> {
    if d.len() < 10 || d[0] != 0x1f || d[1] != 0x8b || d[2] != 8 {
        return None;
    }
    let flg = d[3];
    let mut p = 10usize;
    if flg & 0x04 != 0 {
        let xlen = u16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]) as usize;
        p = p.checked_add(2 + xlen)?;
    }
    for bit in [0x08u8, 0x10] {
        if flg & bit != 0 {
            while *d.get(p)? != 0 {
                p += 1;
            }
            p += 1;
        }
    }
    if flg & 0x02 != 0 {
        p += 2;
    }
    (p <= d.len()).then_some(p)
}

/// Validate a whole gzip file held in memory (all concatenated members).
pub fn check_gzip(data: &[u8]) -> GzipVerdict {
    let mut at = 0usize;
    let mut members = 0;
    loop {
        let Some(h) = header_len(&data[at..]) else {
            return if members == 0 { GzipVerdict::BadHeader } else { GzipVerdict::Complete };
        };
        let Some((end, produced)) = inflate(data, at + h) else {
            return GzipVerdict::Broken("compressed data ends early or is corrupt");
        };
        let Some(trailer) = data.get(end..end + 8) else {
            return GzipVerdict::Broken("the end-of-file checksum is missing");
        };
        let isize = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        if isize as u64 != produced & 0xffff_ffff {
            return GzipVerdict::Broken("stored length does not match the data");
        }
        members += 1;
        at = end + 8;
        if at >= data.len() {
            return GzipVerdict::Complete;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const FIXED: [u8; 29] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40,
        0x27, 0xb9, 0x00, 0x00, 0x88, 0x59, 0x0b, 0x18, 0x00, 0x00, 0x00,
    ];
    /// 1369 bytes of text compressed with a dynamic Huffman block (made with Python's gzip).
    pub const DYNAMIC: [u8; 395] = [
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0x75, 0x54, 0x01, 0x6e, 0xc3, 0x30, 0x08, 0xfc,
        0x4a, 0xbe, 0x66, 0xaf, 0x51, 0x63, 0x2d, 0x4e, 0xa2, 0x36, 0x96, 0xa2, 0xbc, 0x7e, 0xc0, 0x81, 0x0d, 0xdd,
        0x26, 0x35, 0x8d, 0x0d, 0x1c, 0x77, 0x18, 0x9c, 0x73, 0x99, 0xcf, 0x34, 0x9d, 0xa9, 0x4d, 0xef, 0xf2, 0xac,
        0x69, 0x9a, 0x8f, 0x77, 0x59, 0xf7, 0x6d, 0xaa, 0x6d, 0x6a, 0xba, 0x3c, 0x8a, 0xf8, 0x9f, 0xa9, 0x92, 0xdf,
        0x8c, 0x69, 0x3d, 0x96, 0xc4, 0xae, 0xb2, 0x13, 0x1e, 0xd8, 0x53, 0x72, 0xf1, 0x43, 0x76, 0x98, 0xf0, 0x4f,
        0xdb, 0xad, 0xf5, 0xd4, 0x1a, 0xa6, 0xbb, 0xd7, 0xb2, 0xb3, 0x13, 0xf9, 0xc0, 0x71, 0x9b, 0xa4, 0xcc, 0x8b,
        0xef, 0x74, 0x1c, 0x49, 0xfd, 0x42, 0x46, 0xd9, 0x4c, 0x05, 0x01, 0x2f, 0xc9, 0xcd, 0xd1, 0x7b, 0x2d, 0x5f,
        0x2f, 0x32, 0xba, 0x12, 0x1e, 0xf3, 0x4a, 0x80, 0xec, 0xf9, 0x08, 0xcd, 0x5b, 0xc9, 0x44, 0x58, 0x64, 0xa7,
        0x85, 0xea, 0xe0, 0x44, 0xd5, 0x0e, 0x83, 0x37, 0x17, 0xaa, 0x87, 0xe8, 0x35, 0xd5, 0xfc, 0x08, 0x62, 0x4c,
        0xc9, 0xed, 0xdc, 0x03, 0xcc, 0x0f, 0x34, 0x58, 0x49, 0x02, 0x02, 0x29, 0x1c, 0x28, 0x99, 0x54, 0xd1, 0x0f,
        0xeb, 0x6a, 0x67, 0x4d, 0xd4, 0x73, 0x38, 0xee, 0x2e, 0x96, 0x7e, 0xae, 0x34, 0xd3, 0x60, 0x6f, 0x31, 0x6a,
        0x2d, 0x41, 0x92, 0x90, 0x73, 0xa1, 0xa8, 0xe6, 0xd7, 0xf1, 0x82, 0x16, 0x99, 0x2d, 0x19, 0x90, 0xd9, 0x3a,
        0xdb, 0x8f, 0xcc, 0xfc, 0x92, 0x73, 0xf6, 0xcc, 0x4a, 0xa9, 0xaf, 0x3a, 0x1a, 0x4f, 0x92, 0xb6, 0xd1, 0x26,
        0x3d, 0xf0, 0xc8, 0x13, 0xb9, 0x39, 0xc4, 0x1a, 0x05, 0xc9, 0xfd, 0x04, 0x7a, 0x29, 0xb0, 0x00, 0x1d, 0xda,
        0xa3, 0x8d, 0x0b, 0x36, 0xc7, 0xc7, 0xbe, 0xbf, 0x64, 0x9b, 0xbc, 0xda, 0xfc, 0x1d, 0xb0, 0xc1, 0x43, 0x9a,
        0x3e, 0x9b, 0xd8, 0x56, 0x6d, 0xab, 0x41, 0x21, 0x89, 0x83, 0x0c, 0xae, 0xb9, 0x65, 0x48, 0x28, 0xda, 0x0f,
        0xcb, 0x27, 0x09, 0xc0, 0x9b, 0xcd, 0xcd, 0x20, 0x34, 0xb1, 0x08, 0x18, 0x5d, 0x14, 0x54, 0x18, 0xcf, 0x5b,
        0x1b, 0x85, 0x0c, 0xf1, 0x48, 0x63, 0x73, 0x00, 0x33, 0xdd, 0x82, 0xc3, 0x0c, 0xa8, 0xdf, 0xa6, 0xd6, 0x22,
        0x1c, 0x97, 0x9b, 0x3f, 0xd6, 0xe2, 0xe8, 0x19, 0x70, 0x77, 0x65, 0x6a, 0x8c, 0x5f, 0x8f, 0xda, 0x42, 0x49,
        0x7d, 0xa8, 0x7a, 0xc3, 0x5d, 0x17, 0xfc, 0x25, 0xd5, 0x0b, 0x99, 0xb5, 0x40, 0xc3, 0x8f, 0x56, 0xb8, 0x6f,
        0x03, 0xe7, 0xba, 0x4a, 0xf8, 0x34, 0x65, 0x5f, 0xed, 0x3f, 0xc3, 0x83, 0x48, 0xc8, 0x65, 0x3a, 0x7f, 0x57,
        0x5d, 0x96, 0x50, 0xb8, 0x2a, 0xcb, 0x5e, 0xc7, 0x0f, 0x03, 0x82, 0xff, 0x37, 0x59, 0x05, 0x00, 0x00,
    ];

    #[test]
    fn complete_streams_pass() {
        assert_eq!(check_gzip(&FIXED), GzipVerdict::Complete);
        assert_eq!(check_gzip(&DYNAMIC), GzipVerdict::Complete);
        // Two concatenated members are one valid file.
        let mut two = FIXED.to_vec();
        two.extend_from_slice(&FIXED);
        assert_eq!(check_gzip(&two), GzipVerdict::Complete);
    }

    #[test]
    fn every_truncation_is_detected() {
        for data in [&FIXED[..], &DYNAMIC[..]] {
            for cut in 10..data.len() {
                assert!(matches!(check_gzip(&data[..cut]), GzipVerdict::Broken(_)), "cut at {cut}");
            }
        }
    }

    #[test]
    fn wrong_length_field_and_garbage_are_rejected() {
        let mut bad = FIXED;
        bad[25] ^= 1; // ISIZE
        assert!(matches!(check_gzip(&bad), GzipVerdict::Broken(_)));
        assert_eq!(check_gzip(b"PK\x03\x04 not gzip at all"), GzipVerdict::BadHeader);
        assert_eq!(check_gzip(&[]), GzipVerdict::BadHeader);
    }

    #[test]
    fn hostile_input_never_panics() {
        // Deterministic pseudo-random garbage behind a valid header, and bit flips of a real stream.
        let mut x = 0x1234_5678u32;
        for round in 0..300 {
            let mut v = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
            for _ in 0..(round % 90) {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                v.push((x >> 24) as u8);
            }
            let _ = check_gzip(&v);
        }
        for i in 10..DYNAMIC.len() {
            let mut d = DYNAMIC;
            d[i] ^= 0x55;
            let _ = check_gzip(&d);
        }
    }
}
