//! WAV (RIFF/RF64) and AIFF/AIFC headers: format, channels, sample rate, bits, duration.

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::{be16, be32, clean_text, le16, le32, le64, Container, MediaInfo, Src};

const MAX_CHUNKS: u32 = 2_000;

fn wav_codec(tag: u16, ext: Option<u16>) -> String {
    match if tag == 0xFFFE { ext.unwrap_or(tag) } else { tag } {
        1 => "pcm",
        3 => "pcm_float",
        6 => "alaw",
        7 => "mulaw",
        0x11 => "adpcm_ima",
        0x55 => "mp3",
        0x2 => "adpcm_ms",
        0xFFFE => "extensible",
        _ => "unknown",
    }
    .into()
}

pub(super) fn wav<R: Read + Seek>(s: &mut Src<R>) -> Result<MediaInfo> {
    let mut info = MediaInfo::new(Container::Wav, s.len);
    let head = s.read_at(0, 12)?;
    let rf64 = &head[..4] == b"RF64";
    let (mut pos, mut byte_rate, mut data_len, mut ds64_data) = (12u64, 0u64, None::<u64>, None::<u64>);
    let mut chunks = 0;
    let mut fmt_seen = false;
    while pos + 8 <= s.len && chunks < MAX_CHUNKS {
        chunks += 1;
        let h = s.read_at(pos, 8)?;
        let (id, size) = ([h[0], h[1], h[2], h[3]], u64::from(le32(&h, 4).unwrap_or(0)));
        let body = pos + 8;
        let avail = s.len.saturating_sub(body);
        match &id {
            b"ds64" if rf64 => {
                let b = s.read_upto(body, 24)?;
                ds64_data = le64(&b, 8);
            }
            b"fmt " => {
                let b = s.read_upto(body, size.min(40))?;
                if b.len() < 16 {
                    return Err(Error::parse("WAV", "fmt chunk too short"));
                }
                let tag = le16(&b, 0).unwrap_or(0);
                info.channels = le16(&b, 2).filter(|&c| c > 0);
                info.sample_rate = le32(&b, 4).filter(|&r| r > 0);
                byte_rate = u64::from(le32(&b, 8).unwrap_or(0));
                info.bits_per_sample = le16(&b, 14).filter(|&b| b > 0);
                info.audio_codec = Some(wav_codec(tag, le16(&b, 24)));
                if byte_rate == 0 {
                    byte_rate = u64::from(le16(&b, 12).unwrap_or(0)) * u64::from(info.sample_rate.unwrap_or(0));
                }
                fmt_seen = true;
            }
            b"data" => {
                let declared = if size == 0xFFFF_FFFF { ds64_data.unwrap_or(avail) } else { size };
                data_len = Some(declared.min(avail));
            }
            b"LIST" if (4..=65_536).contains(&size) => {
                let b = s.read_upto(body, size)?;
                if b.get(..4) == Some(b"INFO") {
                    let mut p = 4usize;
                    while p + 8 <= b.len() {
                        let sz = le32(&b, p + 4).unwrap_or(0) as usize;
                        let v = b.get(p + 8..p + 8 + sz).unwrap_or(&[]);
                        let text = clean_text(&String::from_utf8_lossy(v));
                        let key = match &b[p..p + 4] {
                            b"INAM" => Some("title"),
                            b"IART" => Some("artist"),
                            b"IPRD" => Some("album"),
                            b"ICMT" => Some("comment"),
                            b"ICRD" => Some("date"),
                            b"IGNR" => Some("genre"),
                            b"ISFT" => {
                                info.encoder = Some(text.clone());
                                None
                            }
                            _ => None,
                        };
                        if let (Some(k), false) = (key, text.is_empty()) {
                            info.tags.insert(k.into(), text);
                        }
                        p += 8 + sz + (sz & 1);
                    }
                }
            }
            _ => {}
        }
        let skip = if &id == b"data" { data_len.unwrap_or(size) } else { size };
        pos = body.saturating_add(skip).saturating_add(skip & 1);
    }
    if !fmt_seen {
        return Err(Error::parse("WAV", "no fmt chunk"));
    }
    info.audio_tracks = 1;
    if let (Some(d), true) = (data_len, byte_rate > 0) {
        info.duration_ms = Some((u128::from(d) * 1000 / u128::from(byte_rate)) as u64);
    }
    Ok(info.finish())
}

/// 80-bit IEEE extended float (AIFF sample rate).
fn extended80(b: &[u8]) -> Option<f64> {
    let exp = i32::from(be16(b, 0)? & 0x7FFF);
    let mant = u64::from(be32(b, 2)?) << 32 | u64::from(be32(b, 6)?);
    if exp == 0 && mant == 0 {
        return Some(0.0);
    }
    if exp == 0x7FFF {
        return None;
    }
    Some(mant as f64 * 2f64.powi(exp - 16383 - 63))
}

pub(super) fn aiff<R: Read + Seek>(s: &mut Src<R>) -> Result<MediaInfo> {
    let mut info = MediaInfo::new(Container::Aiff, s.len);
    let aifc = s.read_at(8, 4)? == b"AIFC";
    let (mut pos, mut chunks, mut comm) = (12u64, 0, false);
    while pos + 8 <= s.len && chunks < MAX_CHUNKS {
        chunks += 1;
        let h = s.read_at(pos, 8)?;
        let (id, size) = ([h[0], h[1], h[2], h[3]], u64::from(be32(&h, 4).unwrap_or(0)));
        let body = pos + 8;
        match &id {
            b"COMM" => {
                let b = s.read_upto(body, size.min(30))?;
                if b.len() < 18 {
                    return Err(Error::parse("AIFF", "COMM chunk too short"));
                }
                info.channels = be16(&b, 0).filter(|&c| c > 0);
                let frames = u64::from(be32(&b, 2).unwrap_or(0));
                info.bits_per_sample = be16(&b, 6).filter(|&b| b > 0);
                let rate = extended80(&b[8..18]).filter(|r| r.is_finite() && *r > 0.0 && *r < 4e6);
                info.sample_rate = rate.map(|r| r.round() as u32);
                let comp = b.get(18..22);
                info.audio_codec = Some(
                    match comp {
                        Some(b"fl32" | b"fl64") => "pcm_float",
                        Some(b"ulaw") => "mulaw",
                        Some(b"alaw") => "alaw",
                        Some(c) if aifc && c != b"NONE" && c != b"sowt" && c != b"twos" => "compressed",
                        _ => "pcm",
                    }
                    .into(),
                );
                info.duration_ms = rate.map(|r| (frames as f64 / r * 1000.0).round() as u64);
                comm = true;
            }
            b"NAME" | b"AUTH" if size <= 4096 => {
                let v = clean_text(&String::from_utf8_lossy(&s.read_upto(body, size)?));
                if !v.is_empty() {
                    info.tags.insert(if &id == b"NAME" { "title" } else { "artist" }.into(), v);
                }
            }
            _ => {}
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    if !comm {
        return Err(Error::parse("AIFF", "no COMM chunk"));
    }
    info.audio_tracks = 1;
    Ok(info.finish())
}

/// Test-support builders.
#[doc(hidden)]
pub mod build {
    /// A PCM WAV of `seconds` of silence (16-bit) with an optional `LIST/INFO` title.
    pub fn wav(rate: u32, channels: u16, seconds: u32, title: Option<&str>) -> Vec<u8> {
        let block = channels * 2;
        let data_len = rate * u32::from(block) * seconds;
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        fmt.extend_from_slice(&block.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend(fmt);
        if let Some(t) = title {
            let mut info = b"INFO".to_vec();
            info.extend_from_slice(b"INAM");
            info.extend_from_slice(&(t.len() as u32).to_le_bytes());
            info.extend_from_slice(t.as_bytes());
            if t.len() % 2 == 1 {
                info.push(0);
            }
            body.extend_from_slice(b"LIST");
            body.extend_from_slice(&(info.len() as u32).to_le_bytes());
            body.extend(info);
        }
        body.extend_from_slice(b"data");
        body.extend_from_slice(&data_len.to_le_bytes());
        body.extend(std::iter::repeat_n(0u8, data_len as usize));
        let mut f = b"RIFF".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_le_bytes());
        f.extend(body);
        f
    }

    /// An AIFF of `frames` frames, 16-bit, at `rate` Hz.
    pub fn aiff(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        // 80-bit extended: exponent = floor(log2(rate)) + 16383, mantissa normalised (bit 63 set).
        let log2 = 31 - rate.leading_zeros();
        let mant = u64::from(rate) << (63 - log2);
        let mut comm = Vec::new();
        comm.extend_from_slice(&channels.to_be_bytes());
        comm.extend_from_slice(&frames.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&((log2 + 16383) as u16).to_be_bytes());
        comm.extend_from_slice(&mant.to_be_bytes());
        let mut body = b"AIFF".to_vec();
        body.extend_from_slice(b"COMM");
        body.extend_from_slice(&18u32.to_be_bytes());
        body.extend(comm);
        let ssnd = (frames * u32::from(channels) * 2) as usize;
        body.extend_from_slice(b"SSND");
        body.extend_from_slice(&((ssnd + 8) as u32).to_be_bytes());
        body.extend_from_slice(&[0u8; 8]);
        body.extend(std::iter::repeat_n(0u8, ssnd));
        let mut f = b"FORM".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend(body);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::super::{probe_reader, Container};
    use super::build::*;
    use std::io::Cursor;

    fn probe(v: Vec<u8>) -> mj_core::Result<super::MediaInfo> {
        let n = v.len() as u64;
        probe_reader(Cursor::new(v), n)
    }

    #[test]
    fn wav_header_and_duration() {
        let i = probe(wav(8000, 1, 3, Some("Standup"))).unwrap();
        assert_eq!(i.container, Container::Wav);
        assert_eq!((i.channels, i.sample_rate, i.bits_per_sample), (Some(1), Some(8000), Some(16)));
        assert_eq!(i.duration_ms, Some(3000));
        assert_eq!(i.audio_codec.as_deref(), Some("pcm"));
        assert_eq!(i.tags.get("title").map(String::as_str), Some("Standup"));
        assert!((127..=129).contains(&i.bitrate_kbps.unwrap()));
    }

    #[test]
    fn wav_with_lying_data_size_is_clamped() {
        let mut f = wav(8000, 1, 1, None);
        let n = f.len();
        f.truncate(n - 8000); // half the data missing
        let i = probe(f).unwrap();
        assert_eq!(i.duration_ms, Some(500));
    }

    #[test]
    fn aiff_header() {
        let i = probe(aiff(44_100, 2, 88_200)).unwrap();
        assert_eq!(i.container, Container::Aiff);
        assert_eq!((i.channels, i.sample_rate), (Some(2), Some(44_100)));
        assert_eq!(i.duration_ms, Some(2000));
    }

    #[test]
    fn truncated_and_corrupt_do_not_panic() {
        for f in [wav(8000, 1, 1, Some("x")), aiff(48_000, 1, 480)] {
            for cut in 0..f.len().min(400) {
                let _ = probe(f[..cut].to_vec());
            }
        }
        let mut f = wav(8000, 1, 1, None);
        f[16..20].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes()); // absurd fmt size
        let _ = probe(f);
    }
}
