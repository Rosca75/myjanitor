//! MP3: ID3v2 tag (size, title/artist/album subset, cover art size), ID3v1 presence, and the first
//! MPEG audio frame (bitrate, sample rate, channels), with Xing/Info/VBRI frame counts for VBR files.

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::{be32, clean_text, Container, MediaInfo, Src};

const MAX_TAG_READ: u64 = 1024 * 1024;

fn synchsafe(b: &[u8]) -> Option<u32> {
    let b = b.get(..4)?;
    (b.iter().all(|x| x & 0x80 == 0)).then(|| b.iter().fold(0u32, |a, &x| a << 7 | u32::from(x)))
}

fn decode_text(enc: u8, b: &[u8]) -> String {
    match enc {
        1 | 2 => {
            let (le, b) = match b {
                [0xFF, 0xFE, r @ ..] => (true, r),
                [0xFE, 0xFF, r @ ..] => (false, r),
                r => (enc != 2, r),
            };
            let units: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(b).into_owned(),
        _ => b.iter().map(|&c| c as char).collect(),
    }
    .trim_end_matches('\0')
    .to_string()
}

/// Parse ID3v2 frames in `tag` (the bytes after the 10-byte header).
fn id3_frames(tag: &[u8], ver: u8, info: &mut MediaInfo) {
    let (idlen, hlen) = if ver == 2 { (3, 6) } else { (4, 10) };
    let mut p = 0usize;
    while p + hlen <= tag.len() {
        let id = &tag[p..p + idlen];
        if id[0] == 0 {
            break; // padding
        }
        let size = match (ver, idlen) {
            (2, _) => u32::from(tag[p + 3]) << 16 | u32::from(tag[p + 4]) << 8 | u32::from(tag[p + 5]),
            (4, _) => synchsafe(&tag[p + 4..]).unwrap_or(0),
            _ => be32(tag, p + 4).unwrap_or(0),
        } as usize;
        let body_start = p + hlen;
        let Some(body_end) = body_start.checked_add(size).filter(|&e| e <= tag.len()) else { break };
        let body = &tag[body_start..body_end];
        match id {
            b"APIC" | b"PIC" => *info.cover_art_bytes.get_or_insert(0) += size as u64,
            _ if !body.is_empty() => {
                let key = match id {
                    b"TIT2" | b"TT2" => Some("title"),
                    b"TPE1" | b"TP1" => Some("artist"),
                    b"TALB" | b"TAL" => Some("album"),
                    b"TDRC" | b"TYER" | b"TYE" => Some("date"),
                    b"TCON" | b"TCO" => Some("genre"),
                    b"TSSE" | b"TSS" => Some("encoder"),
                    _ => None,
                };
                if let Some(k) = key {
                    let text = clean_text(&decode_text(body[0], &body[1..]));
                    if !text.is_empty() {
                        if k == "encoder" {
                            info.encoder = Some(text);
                        } else {
                            info.tags.entry(k.into()).or_insert(text);
                        }
                    }
                }
            }
            _ => {}
        }
        p = body_end;
    }
}

const BR_V1_L1: [u32; 15] = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448];
const BR_V1_L2: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
const BR_V1_L3: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BR_V2_L1: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256];
const BR_V2_L23: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

struct Frame {
    layer: u8,
    mpeg1: bool,
    bitrate: u32,
    rate: u32,
    channels: u16,
    len: usize,
    samples: u32,
    header_len: usize,
}

fn parse_frame(h: &[u8]) -> Option<Frame> {
    let b = h.get(..4)?;
    if b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let ver = (b[1] >> 3) & 3; // 0 = 2.5, 1 reserved, 2 = MPEG2, 3 = MPEG1
    let layer = match (b[1] >> 1) & 3 {
        3 => 1,
        2 => 2,
        1 => 3,
        _ => return None,
    };
    let (br_idx, sr_idx) = ((b[2] >> 4) as usize, ((b[2] >> 2) & 3) as usize);
    if ver == 1 || br_idx == 0 || br_idx == 15 || sr_idx == 3 {
        return None;
    }
    let mpeg1 = ver == 3;
    let table = match (mpeg1, layer) {
        (true, 1) => &BR_V1_L1,
        (true, 2) => &BR_V1_L2,
        (true, _) => &BR_V1_L3,
        (false, 1) => &BR_V2_L1,
        (false, _) => &BR_V2_L23,
    };
    let bitrate = table[br_idx];
    let rate =
        [[11_025, 12_000, 8_000], [0, 0, 0], [22_050, 24_000, 16_000], [44_100, 48_000, 32_000]][ver as usize][sr_idx];
    let padding = u32::from((b[2] >> 1) & 1);
    let (len, samples) = match layer {
        1 => ((12 * bitrate * 1000 / rate + padding) * 4, 384),
        3 if !mpeg1 => (72 * bitrate * 1000 / rate + padding, 576),
        _ => (144 * bitrate * 1000 / rate + padding, 1152),
    };
    let channels = if (b[3] >> 6) == 3 { 1 } else { 2 };
    Some(Frame {
        layer,
        mpeg1,
        bitrate,
        rate,
        channels,
        len: len as usize,
        samples,
        header_len: if b[1] & 1 == 0 { 6 } else { 4 },
    })
}

pub(super) fn parse<R: Read + Seek>(s: &mut Src<R>) -> Result<MediaInfo> {
    let mut info = MediaInfo::new(Container::Mp3, s.len);
    let mut start = 0u64;
    let head = s.read_upto(0, 10)?;
    if head.len() == 10 && &head[..3] == b"ID3" {
        let size = synchsafe(&head[6..]).ok_or_else(|| Error::parse("MP3", "bad ID3v2 size"))?;
        let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
        let total = 10 + u64::from(size) + footer;
        if total > s.len {
            return Err(Error::parse("MP3", "ID3v2 tag exceeds the file"));
        }
        // Extended header (v2.3/2.4) is skipped when present.
        let mut tag = s.read_upto(10, u64::from(size).min(MAX_TAG_READ))?;
        if head[5] & 0x40 != 0 {
            let ext = if head[3] == 4 {
                synchsafe(&tag).unwrap_or(0) as usize
            } else {
                be32(&tag, 0).unwrap_or(0) as usize + 4
            };
            tag.drain(..ext.min(tag.len()));
        }
        id3_frames(&tag, head[3], &mut info);
        start = total;
        if s.read_upto(start, 4)? == b"fLaC" {
            return super::flac::parse(s, start);
        }
    }
    let window = s.read_upto(start, 64 * 1024)?;
    let (off, f) = (0..window.len().saturating_sub(4))
        .find_map(|i| {
            let f = parse_frame(&window[i..])?;
            // Confirm with the next frame header when it lies in the window.
            let next = window.get(i + f.len..i + f.len + 4);
            next.is_none_or(|n| parse_frame(n).is_some()).then_some((i, f))
        })
        .ok_or_else(|| Error::parse("MP3", "no MPEG audio frame found"))?;
    let audio_start = start + off as u64;
    let id3v1 = s.len >= 128 && s.read_at(s.len - 128, 3).is_ok_and(|t| t == b"TAG");
    let audio_bytes = s.len.saturating_sub(audio_start).saturating_sub(if id3v1 { 128 } else { 0 });
    info.audio_codec = Some(format!("mp{}", f.layer));
    info.audio_tracks = 1;
    info.sample_rate = Some(f.rate);
    info.channels = Some(f.channels);
    // VBR header: Xing/Info after the side information, or VBRI at a fixed offset.
    let side = match (f.mpeg1, f.channels) {
        (true, 1) => 17,
        (true, _) => 32,
        (false, 1) => 9,
        (false, _) => 17,
    };
    let frame = &window[off..];
    let xing = f.header_len + side;
    let frames = match frame.get(xing..xing + 8) {
        Some(t) if (t.starts_with(b"Xing") || t.starts_with(b"Info")) && be32(t, 4).is_some_and(|fl| fl & 1 != 0) => {
            be32(frame, xing + 8)
        }
        _ => frame.get(36..40).filter(|t| *t == b"VBRI").and_then(|_| be32(frame, 36 + 14)),
    };
    info.duration_ms = match frames {
        Some(n) if n > 0 => Some(u64::from(n) * u64::from(f.samples) * 1000 / u64::from(f.rate)),
        _ if f.bitrate > 0 => Some(audio_bytes * 8 / u64::from(f.bitrate)),
        _ => None,
    };
    let mut info = info.finish();
    if frames.is_none() {
        info.bitrate_kbps = Some(f.bitrate);
    }
    Ok(info)
}

/// Test-support builder: an ID3v2.3 tag (title, optional cover) plus `frames` CBR MPEG-1 Layer III
/// frames at `bitrate` kbit/s, 44.1 kHz, stereo.
#[doc(hidden)]
pub fn build_mp3(title: Option<&str>, cover: usize, frames: usize, bitrate_idx: u8) -> Vec<u8> {
    let mut tag = Vec::new();
    if let Some(t) = title {
        tag.extend_from_slice(b"TIT2");
        tag.extend_from_slice(&((t.len() + 1) as u32).to_be_bytes());
        tag.extend_from_slice(&[0, 0, 0]);
        tag.extend_from_slice(t.as_bytes());
    }
    if cover > 0 {
        tag.extend_from_slice(b"APIC");
        tag.extend_from_slice(&(cover as u32).to_be_bytes());
        tag.extend_from_slice(&[0, 0]);
        tag.extend(std::iter::repeat_n(0u8, cover));
    }
    let mut f = Vec::new();
    if !tag.is_empty() {
        f.extend_from_slice(b"ID3\x03\x00\x00");
        let n = tag.len() as u32;
        f.extend_from_slice(&[(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
        f.extend(tag);
    }
    let br = BR_V1_L3[bitrate_idx as usize];
    let len = (144 * br * 1000 / 44_100) as usize;
    for _ in 0..frames {
        let mut fr = vec![0u8; len];
        fr[..4].copy_from_slice(&[0xFF, 0xFB, bitrate_idx << 4, 0x00]);
        f.extend(fr);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::super::{probe_reader, Container};
    use super::build_mp3;
    use std::io::Cursor;

    fn probe(v: Vec<u8>) -> mj_core::Result<super::MediaInfo> {
        let n = v.len() as u64;
        probe_reader(Cursor::new(v), n)
    }

    #[test]
    fn id3_tag_and_cbr_estimate() {
        // 128 kbit/s: 417-byte frames; 3000 frames = 3000 * 1152 / 44100 = 78.4 s
        let i = probe(build_mp3(Some("Podcast"), 2000, 3000, 9)).unwrap();
        assert_eq!(i.container, Container::Mp3);
        assert_eq!((i.sample_rate, i.channels, i.bitrate_kbps), (Some(44_100), Some(2), Some(128)));
        let d = i.duration_ms.unwrap();
        assert!((78_000..79_500).contains(&d), "{d}");
        assert_eq!(i.tags.get("title").map(String::as_str), Some("Podcast"));
        assert_eq!(i.cover_art_bytes, Some(2000));
        assert_eq!(i.audio_codec.as_deref(), Some("mp3"));
    }

    #[test]
    fn bare_frames_without_tag() {
        let i = probe(build_mp3(None, 0, 50, 5)).unwrap();
        assert_eq!(i.bitrate_kbps, Some(64));
        assert!(i.tags.is_empty());
    }

    #[test]
    fn xing_frame_count_wins() {
        let mut f = build_mp3(None, 0, 10, 9);
        f[36..40].copy_from_slice(b"Xing");
        f[40..44].copy_from_slice(&1u32.to_be_bytes());
        f[44..48].copy_from_slice(&1000u32.to_be_bytes());
        let i = probe(f).unwrap();
        assert_eq!(i.duration_ms, Some(1000 * 1152 * 1000 / 44_100));
    }

    #[test]
    fn corrupt_tags_and_truncation_are_safe() {
        let f = build_mp3(Some("abc"), 100, 5, 9);
        for cut in 0..f.len().min(300) {
            let _ = probe(f[..cut].to_vec());
        }
        let mut bad = b"ID3\x03\x00\x00\x7f\x7f\x7f\x7f".to_vec();
        bad.extend_from_slice(&[0u8; 20]);
        assert!(probe(bad).is_err());
        assert!(probe(vec![0xFF, 0xFB, 0xF0, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
