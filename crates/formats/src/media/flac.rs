//! FLAC: STREAMINFO (rate, channels, bits, total samples), VORBIS_COMMENT subset and PICTURE sizes.

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::{be32, clean_text, le32, Container, MediaInfo, Src};

const MAX_BLOCKS: u32 = 1_000;

/// Parse a FLAC stream starting at `base` (the `fLaC` marker).
pub(super) fn parse<R: Read + Seek>(s: &mut Src<R>, base: u64) -> Result<MediaInfo> {
    let mut info = MediaInfo::new(Container::Flac, s.len);
    let mut pos = base + 4;
    let mut seen_streaminfo = false;
    for _ in 0..MAX_BLOCKS {
        let Ok(h) = s.read_at(pos, 4) else { break };
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7F;
        let size = u64::from(h[1]) << 16 | u64::from(h[2]) << 8 | u64::from(h[3]);
        let body = pos + 4;
        match kind {
            0 if size >= 34 => {
                let b = s.read_at(body, 34)?;
                let rate = u32::from(b[10]) << 12 | u32::from(b[11]) << 4 | u32::from(b[12] >> 4);
                let channels = ((b[12] >> 1) & 7) + 1;
                let bits = (u16::from(b[12] & 1) << 4 | u16::from(b[13] >> 4)) + 1;
                let total = u64::from(b[13] & 0x0F) << 32 | u64::from(be32(&b, 14).unwrap_or(0));
                info.sample_rate = Some(rate).filter(|&r| r > 0);
                info.channels = Some(u16::from(channels));
                info.bits_per_sample = Some(bits);
                info.audio_codec = Some("flac".into());
                info.audio_tracks = 1;
                if rate > 0 && total > 0 {
                    info.duration_ms = Some((u128::from(total) * 1000 / u128::from(rate)) as u64);
                }
                seen_streaminfo = true;
            }
            4 if size <= 1024 * 1024 => vorbis_comment(&s.read_at(body, size)?, &mut info),
            6 => *info.cover_art_bytes.get_or_insert(0) += size,
            _ => {}
        }
        pos = body.saturating_add(size);
        if last {
            break;
        }
    }
    if !seen_streaminfo {
        return Err(Error::parse("FLAC", "no STREAMINFO block"));
    }
    Ok(info.finish())
}

fn vorbis_comment(b: &[u8], info: &mut MediaInfo) {
    let Some(vendor) = le32(b, 0) else { return };
    let mut p = 4 + vendor as usize;
    let Some(count) = le32(b, p) else { return };
    p += 4;
    for _ in 0..count.min(512) {
        let Some(n) = le32(b, p) else { return };
        let Some(raw) = b.get(p + 4..p + 4 + n as usize) else { return };
        p += 4 + n as usize;
        let text = String::from_utf8_lossy(raw);
        if let Some((k, v)) = text.split_once('=') {
            let key = match k.to_ascii_lowercase().as_str() {
                "title" => "title",
                "artist" => "artist",
                "album" => "album",
                "date" => "date",
                "genre" => "genre",
                "comment" => "comment",
                _ => continue,
            };
            let v = clean_text(v);
            if !v.is_empty() {
                info.tags.insert(key.into(), v);
            }
        }
    }
}

/// Test-support builder.
#[doc(hidden)]
pub fn build_flac(rate: u32, channels: u8, bits: u8, total: u64, title: Option<&str>, picture: usize) -> Vec<u8> {
    let mut si = vec![0u8; 34];
    si[10] = (rate >> 12) as u8;
    si[11] = (rate >> 4) as u8;
    si[12] = ((rate & 0xF) << 4) as u8 | ((channels - 1) << 1) | ((bits - 1) >> 4);
    si[13] = (((bits - 1) & 0xF) << 4) | ((total >> 32) as u8 & 0x0F);
    si[14..18].copy_from_slice(&(total as u32).to_be_bytes());
    let mut f = b"fLaC".to_vec();
    let mut blocks: Vec<(u8, Vec<u8>)> = vec![(0, si)];
    if let Some(t) = title {
        let mut vc = Vec::new();
        vc.extend_from_slice(&4u32.to_le_bytes());
        vc.extend_from_slice(b"test");
        vc.extend_from_slice(&1u32.to_le_bytes());
        let c = format!("TITLE={t}");
        vc.extend_from_slice(&(c.len() as u32).to_le_bytes());
        vc.extend_from_slice(c.as_bytes());
        blocks.push((4, vc));
    }
    if picture > 0 {
        blocks.push((6, vec![0u8; picture]));
    }
    let n = blocks.len();
    for (i, (kind, body)) in blocks.into_iter().enumerate() {
        f.push(kind | if i + 1 == n { 0x80 } else { 0 });
        f.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        f.extend(body);
    }
    f.extend(std::iter::repeat_n(0u8, 1000));
    f
}

#[cfg(test)]
mod tests {
    use super::super::{probe_reader, Container};
    use super::build_flac;
    use std::io::Cursor;

    fn probe(v: Vec<u8>) -> mj_core::Result<super::MediaInfo> {
        let n = v.len() as u64;
        probe_reader(Cursor::new(v), n)
    }

    #[test]
    fn streaminfo_tags_and_picture() {
        let i = probe(build_flac(44_100, 2, 16, 441_000, Some("Song"), 300)).unwrap();
        assert_eq!(i.container, Container::Flac);
        assert_eq!((i.sample_rate, i.channels, i.bits_per_sample), (Some(44_100), Some(2), Some(16)));
        assert_eq!(i.duration_ms, Some(10_000));
        assert_eq!(i.cover_art_bytes, Some(300));
        assert_eq!(i.tags.get("title").map(String::as_str), Some("Song"));
        let hi = probe(build_flac(96_000, 6, 24, 96_000 * 5, None, 0)).unwrap();
        assert_eq!(
            (hi.sample_rate, hi.channels, hi.bits_per_sample, hi.duration_ms),
            (Some(96_000), Some(6), Some(24), Some(5000))
        );
    }

    #[test]
    fn truncation_is_safe() {
        let f = build_flac(48_000, 1, 16, 48_000, Some("x"), 10);
        for cut in 0..f.len().min(200) {
            let _ = probe(f[..cut].to_vec());
        }
        assert!(probe(b"fLaC\x00\x00\x00\x00abcdefgh".to_vec()).is_err());
    }
}
