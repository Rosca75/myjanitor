//! ISO base media (MP4, M4A, MOV) box walker: ftyp, moov/mvhd, trak/tkhd, mdia/mdhd+hdlr,
//! stsd codec, stts frame rate and the `ilst` tag subset. Handles 64-bit box sizes and a `moov`
//! at the end of the file by seeking past `mdat` (never reading it).

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::{be16, be32, be64, clean_text, Container, MediaInfo, Src};

const MAX_DEPTH: u32 = 12;
const MAX_BOXES: u32 = 100_000;
const MAX_TRACKS: u32 = 64;
/// Cap on `stts` entries read (8 bytes each).
const MAX_STTS: u64 = 65_536;

#[derive(Default, Clone)]
struct Track {
    handler: [u8; 4],
    tkhd_w: u32,
    tkhd_h: u32,
    w: u32,
    h: u32,
    timescale: u32,
    duration: u64,
    samples: Option<u64>,
    codec: Option<String>,
    channels: u16,
    rate: u32,
    bits: u16,
}

struct Walker<'a, R> {
    s: &'a mut Src<R>,
    info: MediaInfo,
    boxes: u32,
    tracks: Vec<Track>,
    cur: Option<Track>,
    mvhd: Option<(u32, u64)>,
}

/// Human codec name for a sample-entry fourcc.
fn codec_name(cc: &[u8]) -> String {
    match cc {
        b"avc1" | b"avc3" => "h264".into(),
        b"hvc1" | b"hev1" => "hevc".into(),
        b"av01" => "av1".into(),
        b"vp09" => "vp9".into(),
        b"vp08" => "vp8".into(),
        b"mp4v" => "mpeg4".into(),
        b"jpeg" | b"mjpa" => "mjpeg".into(),
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" => "prores".into(),
        b"mp4a" => "aac".into(),
        b"ac-3" => "ac3".into(),
        b"ec-3" => "eac3".into(),
        b"Opus" => "opus".into(),
        b"fLaC" => "flac".into(),
        b"alac" => "alac".into(),
        b"sowt" | b"twos" | b"lpcm" | b"in24" | b"in32" | b"fl32" => "pcm".into(),
        other => clean_text(&String::from_utf8_lossy(other)).to_lowercase(),
    }
}

pub(super) fn parse<R: Read + Seek>(s: &mut Src<R>) -> Result<MediaInfo> {
    let len = s.len;
    let mut w =
        Walker { s, info: MediaInfo::new(Container::Mp4, len), boxes: 0, tracks: Vec::new(), cur: None, mvhd: None };
    w.walk(0, len, 0, *b"    ")?;
    w.finish()
}

impl<R: Read + Seek> Walker<'_, R> {
    fn walk(&mut self, start: u64, end: u64, depth: u32, parent: [u8; 4]) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error::parse("MP4", "boxes nested too deeply"));
        }
        let mut pos = start;
        while pos + 8 <= end {
            self.boxes += 1;
            if self.boxes > MAX_BOXES {
                return Err(Error::parse("MP4", "too many boxes"));
            }
            let h = self.s.read_at(pos, 8)?;
            let (size32, kind) = (u64::from(be32(&h, 0).unwrap_or(0)), [h[4], h[5], h[6], h[7]]);
            let (hdr, size) = match size32 {
                1 => (16u64, be64(&self.s.read_at(pos + 8, 8)?, 0).unwrap_or(0)),
                0 => (8, end - pos),
                n => (8, n),
            };
            if size < hdr {
                return Err(Error::parse("MP4", "box smaller than its header"));
            }
            // A box running past its parent is clamped for mdat (truncated files) and an error otherwise.
            let box_end = pos.saturating_add(size);
            let box_end = if box_end > end {
                if &kind == b"mdat" || depth == 0 {
                    end
                } else {
                    return Err(Error::parse("MP4", "box exceeds its parent"));
                }
            } else {
                box_end
            };
            let body = pos + hdr;
            self.visit(kind, parent, body, box_end, depth)?;
            if box_end <= pos {
                return Err(Error::parse("MP4", "box walk does not advance"));
            }
            pos = box_end;
        }
        Ok(())
    }

    fn visit(&mut self, kind: [u8; 4], parent: [u8; 4], body: u64, end: u64, depth: u32) -> Result<()> {
        match &kind {
            b"ftyp" => {
                let b = self.s.read_upto(body, 4)?;
                if b == b"qt  " {
                    self.info.container = Container::Mov;
                }
            }
            b"moov" | b"mdia" | b"minf" | b"stbl" | b"udta" => self.walk(body, end, depth + 1, kind)?,
            b"trak" => {
                if self.tracks.len() as u32 >= MAX_TRACKS {
                    return Err(Error::parse("MP4", "too many tracks"));
                }
                self.cur = Some(Track::default());
                self.walk(body, end, depth + 1, kind)?;
                if let Some(t) = self.cur.take() {
                    self.tracks.push(t);
                }
            }
            b"meta" => {
                // ISO `meta` is a full box (4 version/flags bytes); QuickTime's has none.
                let peek = self.s.read_upto(body + 4, 8)?;
                let start = if peek.len() >= 8 && &peek[4..8] == b"hdlr" { body + 4 } else { body };
                self.walk(start, end, depth + 1, kind)?;
            }
            b"ilst" => self.ilst(body, end)?,
            b"mvhd" if parent == *b"moov" => {
                let b = self.s.read_upto(body, 32)?;
                self.mvhd = match b.first() {
                    Some(0) => be32(&b, 12).zip(be32(&b, 16)).map(|(t, d)| (t, u64::from(d))),
                    Some(1) => be32(&b, 20).zip(be64(&b, 24)),
                    _ => None,
                };
            }
            b"tkhd" => {
                let b = self.s.read_upto(body, 104)?;
                let off = if b.first() == Some(&1) { 96 } else { 84 };
                if let (Some(t), Some(w), Some(h)) = (self.cur.as_mut(), be32(&b, off), be32(&b, off + 4)) {
                    (t.tkhd_w, t.tkhd_h) = (w >> 16, h >> 16);
                }
            }
            b"mdhd" => {
                let b = self.s.read_upto(body, 32)?;
                let v = match b.first() {
                    Some(0) => be32(&b, 12).zip(be32(&b, 16)).map(|(t, d)| (t, u64::from(d))),
                    Some(1) => be32(&b, 20).zip(be64(&b, 24)),
                    _ => None,
                };
                if let (Some(t), Some((ts, d))) = (self.cur.as_mut(), v) {
                    (t.timescale, t.duration) = (ts, d);
                }
            }
            b"hdlr" if parent == *b"mdia" => {
                let b = self.s.read_upto(body, 12)?;
                if let (Some(t), Some(h)) = (self.cur.as_mut(), b.get(8..12)) {
                    t.handler.copy_from_slice(h);
                }
            }
            b"stsd" => self.stsd(body, end)?,
            b"stts" => {
                let b = self.s.read_upto(body, 8)?;
                let n = u64::from(be32(&b, 4).unwrap_or(0));
                if n > 0 && n <= MAX_STTS && end.saturating_sub(body) >= 8 + n * 8 {
                    let e = self.s.read_at(body + 8, n * 8)?;
                    let total: u64 = (0..n as usize).map(|i| u64::from(be32(&e, i * 8).unwrap_or(0))).sum();
                    if let Some(t) = self.cur.as_mut() {
                        t.samples = Some(total);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn stsd(&mut self, body: u64, end: u64) -> Result<()> {
        let b = self.s.read_upto(body, (end.saturating_sub(body)).min(8 + 40))?;
        // [ver/flags 4][count 4][entry: size 4, fourcc 4, reserved 6, dref 2, ...]
        let Some(cc) = b.get(12..16) else { return Ok(()) };
        let Some(t) = self.cur.as_mut() else { return Ok(()) };
        t.codec = Some(codec_name(cc));
        let e = &b[8..];
        match &t.handler {
            b"vide" => {
                t.w = u32::from(be16(e, 32).unwrap_or(0));
                t.h = u32::from(be16(e, 34).unwrap_or(0));
            }
            b"soun" => {
                t.channels = be16(e, 24).unwrap_or(0);
                t.bits = be16(e, 26).unwrap_or(0);
                t.rate = u32::from(be16(e, 32).unwrap_or(0));
            }
            _ => {}
        }
        Ok(())
    }

    /// Tags of an `ilst` box: each child holds a `data` box (8 header bytes, then the value).
    fn ilst(&mut self, body: u64, end: u64) -> Result<()> {
        let mut pos = body;
        let mut n = 0;
        while pos + 8 <= end && n < 256 {
            n += 1;
            let h = self.s.read_at(pos, 8)?;
            let size = u64::from(be32(&h, 0).unwrap_or(0));
            if size < 16 || pos + size > end {
                break;
            }
            let key = h[4..8].to_vec();
            let data_hdr = self.s.read_at(pos + 8, 8)?;
            if &data_hdr[4..8] == b"data" {
                let dsize = u64::from(be32(&data_hdr, 0).unwrap_or(0));
                let vlen = dsize.saturating_sub(16).min(size.saturating_sub(24));
                if key == b"covr" {
                    *self.info.cover_art_bytes.get_or_insert(0) += vlen;
                } else if vlen > 0 && vlen <= 4096 {
                    let v = self.s.read_at(pos + 24, vlen)?;
                    let text = clean_text(&String::from_utf8_lossy(&v));
                    let name = match key.as_slice() {
                        [0xA9, b'n', b'a', b'm'] => Some("title"),
                        [0xA9, b'A', b'R', b'T'] => Some("artist"),
                        [0xA9, b'a', b'l', b'b'] => Some("album"),
                        [0xA9, b'd', b'a', b'y'] => Some("date"),
                        [0xA9, b'g', b'e', b'n'] => Some("genre"),
                        [0xA9, b'c', b'm', b't'] => Some("comment"),
                        [0xA9, b't', b'o', b'o'] => {
                            self.info.encoder = Some(text.clone());
                            None
                        }
                        _ => None,
                    };
                    if let (Some(k), false) = (name, text.is_empty()) {
                        self.info.tags.insert(k.to_string(), text);
                    }
                }
            }
            pos += size;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<MediaInfo> {
        if self.tracks.is_empty() && self.mvhd.is_none() {
            return Err(Error::parse("MP4", "no moov box (not a complete MP4/MOV file)"));
        }
        let secs = |d: u64, ts: u32| (ts > 0).then(|| d as f64 / f64::from(ts));
        let mut dur = self.mvhd.and_then(|(ts, d)| secs(d, ts));
        let vid = self.tracks.iter().find(|t| &t.handler == b"vide").cloned();
        let aud = self.tracks.iter().find(|t| &t.handler == b"soun").cloned();
        if dur.is_none() {
            dur = self
                .tracks
                .iter()
                .filter_map(|t| secs(t.duration, t.timescale))
                .fold(None, |a, b| Some(a.map_or(b, |a: f64| a.max(b))));
        }
        self.info.duration_ms = dur.map(|d| (d * 1000.0).round() as u64);
        self.info.video_tracks = self.tracks.iter().filter(|t| &t.handler == b"vide").count() as u32;
        self.info.audio_tracks = self.tracks.iter().filter(|t| &t.handler == b"soun").count() as u32;
        self.info.subtitle_tracks =
            self.tracks.iter().filter(|t| matches!(&t.handler, b"sbtl" | b"text" | b"subt" | b"clcp")).count() as u32;
        if let Some(v) = vid {
            self.info.video_codec = v.codec.clone();
            self.info.width = Some(if v.w > 0 { v.w } else { v.tkhd_w }).filter(|&w| w > 0);
            self.info.height = Some(if v.h > 0 { v.h } else { v.tkhd_h }).filter(|&h| h > 0);
            let vd = secs(v.duration, v.timescale).filter(|&d| d > 0.0);
            self.info.frame_rate = v
                .samples
                .zip(vd)
                .map(|(n, d)| (n as f64 / d) as f32)
                .filter(|f| f.is_finite() && *f > 0.0 && *f < 1000.0);
        }
        if let Some(a) = aud {
            self.info.audio_codec = a.codec.clone();
            self.info.channels = Some(a.channels).filter(|&c| c > 0);
            self.info.sample_rate = Some(a.rate).filter(|&r| r > 0);
            self.info.bits_per_sample = Some(a.bits).filter(|&b| b > 0);
        }
        Ok(self.info.finish())
    }
}

/// Test-support builders for MP4 box structures.
#[doc(hidden)]
pub mod build {
    /// A box: size, type, body.
    pub fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// A box with a 64-bit size field.
    pub fn boxed64(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(&((body.len() + 16) as u64).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    /// One track: `handler` is `vide` or `soun`.
    pub fn trak(
        handler: &[u8; 4],
        codec: &[u8; 4],
        w: u16,
        h: u16,
        timescale: u32,
        duration: u32,
        frames: u32,
    ) -> Vec<u8> {
        let mut tkhd = vec![0u8; 84];
        tkhd.extend_from_slice(&(u32::from(w) << 16).to_be_bytes());
        tkhd.extend_from_slice(&(u32::from(h) << 16).to_be_bytes());
        let mut mdhd = vec![0u8; 12];
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&duration.to_be_bytes());
        mdhd.extend_from_slice(&[0, 0, 0, 0]);
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0u8; 13]);
        let mut entry = vec![0u8; 6];
        entry.extend_from_slice(&[0, 1]);
        if handler == b"vide" {
            entry.extend_from_slice(&[0u8; 16]);
            entry.extend_from_slice(&w.to_be_bytes());
            entry.extend_from_slice(&h.to_be_bytes());
        } else {
            entry.extend_from_slice(&[0u8; 8]);
            entry.extend_from_slice(&2u16.to_be_bytes());
            entry.extend_from_slice(&16u16.to_be_bytes());
            entry.extend_from_slice(&[0u8; 4]);
            entry.extend_from_slice(&(48_000u32 << 16).to_be_bytes());
        }
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(boxed(codec, &entry));
        let mut stts = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stts.extend_from_slice(&frames.to_be_bytes());
        stts.extend_from_slice(&1u32.to_be_bytes());
        let stbl = boxed(b"stbl", &cat(&[boxed(b"stsd", &stsd), boxed(b"stts", &stts)]));
        let mdia = boxed(b"mdia", &cat(&[boxed(b"mdhd", &mdhd), boxed(b"hdlr", &hdlr), boxed(b"minf", &stbl)]));
        boxed(b"trak", &cat(&[boxed(b"tkhd", &tkhd), mdia]))
    }

    /// A `moov` with an mvhd (timescale 1000) and the given children.
    pub fn moov(duration_ms: u32, children: &[Vec<u8>]) -> Vec<u8> {
        let mut mvhd = vec![0u8; 12];
        mvhd.extend_from_slice(&1000u32.to_be_bytes());
        mvhd.extend_from_slice(&duration_ms.to_be_bytes());
        mvhd.extend_from_slice(&[0u8; 80]);
        let mut parts = vec![boxed(b"mvhd", &mvhd)];
        parts.extend_from_slice(children);
        boxed(b"moov", &cat(&parts))
    }

    /// `ilst` item with a text value.
    pub fn ilst_text(key: [u8; 4], text: &str) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
        data.extend_from_slice(text.as_bytes());
        boxed(&key, &boxed(b"data", &data))
    }

    /// `udta/meta/ilst` wrapper (ISO full-box meta).
    pub fn udta_ilst(items: &[Vec<u8>]) -> Vec<u8> {
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(boxed(b"hdlr", &[0u8; 24]));
        meta.extend(boxed(b"ilst", &items.concat()));
        boxed(b"udta", &boxed(b"meta", &meta))
    }

    /// `ftyp` + `mdat` (of `mdat_len` zero bytes) + the given tail boxes.
    pub fn file(brand: &[u8; 4], mdat_len: usize, tail: &[Vec<u8>]) -> Vec<u8> {
        let mut f = boxed(b"ftyp", &[brand.as_slice(), &[0, 0, 0, 0], brand.as_slice()].concat());
        f.extend(boxed(b"mdat", &vec![0u8; mdat_len]));
        for t in tail {
            f.extend_from_slice(t);
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::super::probe_reader;
    use super::build::*;
    use std::io::Cursor;

    fn probe(v: Vec<u8>) -> mj_core::Result<super::MediaInfo> {
        let n = v.len() as u64;
        probe_reader(Cursor::new(v), n)
    }

    #[test]
    fn moov_at_end_with_video_and_audio() {
        let tail = moov(
            60_000,
            &[
                trak(b"vide", b"avc1", 1920, 1080, 30_000, 1_800_000, 1800),
                trak(b"soun", b"mp4a", 0, 0, 48_000, 2_880_000, 2812),
                udta_ilst(&[ilst_text(*b"\xA9nam", "Weekly sync"), ilst_text(*b"\xA9too", "OBS Studio 30.1")]),
            ],
        );
        let f = file(b"isom", 5_000_000, &[tail]);
        let i = probe(f).unwrap();
        assert_eq!(i.duration_ms, Some(60_000));
        assert_eq!((i.width, i.height), (Some(1920), Some(1080)));
        assert_eq!(i.video_codec.as_deref(), Some("h264"));
        assert_eq!(i.audio_codec.as_deref(), Some("aac"));
        assert_eq!((i.channels, i.sample_rate, i.bits_per_sample), (Some(2), Some(48_000), Some(16)));
        assert!((i.frame_rate.unwrap() - 30.0).abs() < 0.01);
        assert_eq!((i.video_tracks, i.audio_tracks), (1, 1));
        assert_eq!(i.tags.get("title").map(String::as_str), Some("Weekly sync"));
        assert_eq!(i.encoder.as_deref(), Some("OBS Studio 30.1"));
        assert!(i.bitrate_kbps.unwrap() > 600);
    }

    #[test]
    fn large_size_boxes_and_quicktime_brand() {
        let mut f = boxed(b"ftyp", b"qt  \0\0\0\0qt  ");
        f.extend(boxed64(b"mdat", &[0u8; 1000]));
        f.extend(moov(2_000, &[trak(b"vide", b"hvc1", 640, 360, 600, 1200, 48)]));
        let i = probe(f).unwrap();
        assert_eq!(i.container, super::Container::Mov);
        assert_eq!(i.video_codec.as_deref(), Some("hevc"));
        assert_eq!(i.duration_ms, Some(2_000));
    }

    #[test]
    fn missing_moov_truncation_and_bad_sizes_are_errors_not_panics() {
        assert!(probe(file(b"isom", 100, &[])).is_err());
        let f = file(b"isom", 100, &[moov(1000, &[trak(b"soun", b"mp4a", 0, 0, 44_100, 44_100, 10)])]);
        for cut in 0..f.len() {
            let _ = probe(f[..cut].to_vec());
        }
        // A box that claims to be smaller than its header.
        let mut bad = boxed(b"ftyp", b"isom\0\0\0\0");
        bad.extend_from_slice(&[0, 0, 0, 4, b'm', b'o', b'o', b'v']);
        assert!(probe(bad).is_err());
        // Deep nesting is refused.
        let mut deep = boxed(b"trak", &[]);
        for _ in 0..40 {
            deep = boxed(b"trak", &deep);
        }
        let _ = probe([boxed(b"ftyp", b"isom\0\0\0\0"), boxed(b"moov", &deep)].concat());
    }
}
