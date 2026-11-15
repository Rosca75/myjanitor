//! xxHash64, streaming and one-shot. Reference: the xxHash specification
//! (<https://github.com/Cyan4973/xxHash/blob/dev/doc/xxhash_spec.md>).

const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline]
fn round(acc: u64, lane: u64) -> u64 {
    acc.wrapping_add(lane.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

#[inline]
fn merge(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

#[inline]
fn read_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[inline]
fn read_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// One-shot xxHash64 of `data` with `seed`.
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    let mut h = Xxh64::new(seed);
    h.update(data);
    h.finish()
}

/// Streaming xxHash64 state.
#[derive(Clone, Debug)]
pub struct Xxh64 {
    seed: u64,
    acc: [u64; 4],
    buf: [u8; 32],
    buf_len: usize,
    total: u64,
}

impl Xxh64 {
    /// New state with the given seed (MyJanitor uses seed 0 everywhere).
    pub fn new(seed: u64) -> Self {
        Xxh64 {
            seed,
            acc: [seed.wrapping_add(P1).wrapping_add(P2), seed.wrapping_add(P2), seed, seed.wrapping_sub(P1)],
            buf: [0; 32],
            buf_len: 0,
            total: 0,
        }
    }

    fn stripe(acc: &mut [u64; 4], s: &[u8]) {
        acc[0] = round(acc[0], read_u64(&s[0..]));
        acc[1] = round(acc[1], read_u64(&s[8..]));
        acc[2] = round(acc[2], read_u64(&s[16..]));
        acc[3] = round(acc[3], read_u64(&s[24..]));
    }

    /// Feed more bytes.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        if self.buf_len > 0 {
            let take = (32 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < 32 {
                return;
            }
            let buf = self.buf;
            Self::stripe(&mut self.acc, &buf);
            self.buf_len = 0;
        }
        while data.len() >= 32 {
            Self::stripe(&mut self.acc, &data[..32]);
            data = &data[32..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buf_len = data.len();
    }

    /// Final hash (the state can keep being updated afterwards).
    pub fn finish(&self) -> u64 {
        let mut h = if self.total >= 32 {
            let a = self.acc;
            let mut h = a[0]
                .rotate_left(1)
                .wrapping_add(a[1].rotate_left(7))
                .wrapping_add(a[2].rotate_left(12))
                .wrapping_add(a[3].rotate_left(18));
            for v in a {
                h = merge(h, v);
            }
            h
        } else {
            self.seed.wrapping_add(P5)
        };
        h = h.wrapping_add(self.total);
        let mut rest = &self.buf[..self.buf_len];
        while rest.len() >= 8 {
            h ^= round(0, read_u64(rest));
            h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
            rest = &rest[8..];
        }
        if rest.len() >= 4 {
            h ^= (read_u32(rest) as u64).wrapping_mul(P1);
            h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
            rest = &rest[4..];
        }
        for &b in rest {
            h ^= (b as u64).wrapping_mul(P5);
            h = h.rotate_left(11).wrapping_mul(P1);
        }
        h ^= h >> 33;
        h = h.wrapping_mul(P2);
        h ^= h >> 29;
        h = h.wrapping_mul(P3);
        h ^= h >> 32;
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity buffer from the reference implementation's test suite
    /// (`xxhsum` sanity checks): byte i = (gen >> 56), gen = gen * PRIME32 ...
    fn sanity_buffer(len: usize) -> Vec<u8> {
        const PRIME32: u64 = 2_654_435_761;
        const PRIME64: u64 = 11_400_714_785_074_694_797;
        let mut gen = PRIME32;
        let mut v = Vec::with_capacity(len);
        for _ in 0..len {
            v.push((gen >> 56) as u8);
            gen = gen.wrapping_mul(PRIME64);
        }
        v
    }

    #[test]
    fn official_vectors() {
        // Empty input: the published xxh64("") value.
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"a", 0), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
        assert_eq!(xxh64(b"Nobody inspects the spammish repetition", 0), 0xFBCE_A83C_8A37_8BF1);
        // xxhsum sanity-check table (XXH64 column).
        let buf = sanity_buffer(222);
        assert_eq!(xxh64(&buf[..0], 2_654_435_761), 0xAC75_FDA2_929B_17EF);
        assert_eq!(xxh64(&buf[..1], 0), 0xE934_A84A_DB05_2768);
        assert_eq!(xxh64(&buf[..1], 2_654_435_761), 0x5014_6076_43A9_B4C3);
        assert_eq!(xxh64(&buf[..14], 0), 0x8282_DCC4_994E_35C8);
        assert_eq!(xxh64(&buf[..14], 2_654_435_761), 0xC3BD_6BF6_3DEB_6DF0);
        assert_eq!(xxh64(&buf[..222], 0), 0xB641_AE8C_B691_C174);
        assert_eq!(xxh64(&buf[..222], 2_654_435_761), 0x20CB_8AB7_AE10_C14A);
    }

    #[test]
    fn streaming_matches_one_shot_at_every_split() {
        let data: Vec<u8> = (0..300u32).map(|i| (i * 7 + 3) as u8).collect();
        let want = xxh64(&data, 42);
        for split in 0..data.len() {
            let mut h = Xxh64::new(42);
            h.update(&data[..split]);
            h.update(&data[split..]);
            assert_eq!(h.finish(), want, "split {split}");
        }
        let mut h = Xxh64::new(42);
        for b in &data {
            h.update(std::slice::from_ref(b));
        }
        assert_eq!(h.finish(), want);
    }
}
