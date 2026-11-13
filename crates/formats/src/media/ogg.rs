//! Ogg: Opus (`OpusHead`) and Vorbis identification headers, duration from the last page's granule.

use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::{le16, le32, le64, Container, MediaInfo, Src};

pub(super) fn parse<R: Read + Seek>(s: &mut Src<R>) -> Result<MediaInfo> {
    let mut info = MediaInfo::new(Container::Ogg, s.len);
    let first = s.read_upto(0, 4096)?;
    let nseg = *first.get(26).ok_or_else(|| Error::parse("Ogg", "truncated first page"))? as usize;
    let payload_start = 27 + nseg;
    let payload = first.get(payload_start..).unwrap_or(&[]);
    let (codec, pre_skip, rate) = if payload.starts_with(b"OpusHead") && payload.len() >= 19 {
        info.channels = Some(u16::from(payload[9]));
        (Some("opus"), u64::from(le16(payload, 10).unwrap_or(0)), 48_000u32)
    } else if payload.starts_with(b"\x01vorbis") && payload.len() >= 16 {
        info.channels = Some(u16::from(payload[11]));
        let r = le32(payload, 12).unwrap_or(0);
        (Some("vorbis"), 0, r)
    } else if payload.starts_with(b"\x7fFLAC") {
        (Some("flac"), 0, 0)
    } else {
        return Err(Error::parse("Ogg", "unsupported Ogg stream (not Opus or Vorbis)"));
    };
    info.audio_codec = codec.map(String::from);
    info.audio_tracks = 1;
    if rate > 0 {
        info.sample_rate =
            Some(if codec == Some("opus") { le32(payload, 12).filter(|&r| r > 0).unwrap_or(rate) } else { rate });
    }
    // Duration: granule position of the last page.
    let tail_len = s.len.min(64 * 1024);
    let tail = s.read_at(s.len - tail_len, tail_len)?;
    if let Some(at) = tail.windows(4).rposition(|w| w == b"OggS") {
        if let Some(g) = le64(&tail, at + 6).map(|g| g as i64).filter(|&g| g > 0 && rate > 0) {
            let samples = (g as u64).saturating_sub(pre_skip);
            info.duration_ms = Some(samples * 1000 / u64::from(rate));
        }
    }
    Ok(info.finish())
}

/// Test-support builder: an Opus stream with identification page and a final page with `granule`.
#[doc(hidden)]
pub fn build_opus(channels: u8, granule: u64, padding: usize) -> Vec<u8> {
    let page = |flags: u8, gp: u64, seq: u32, payload: &[u8]| {
        let mut p = b"OggS".to_vec();
        p.extend_from_slice(&[0, flags]);
        p.extend_from_slice(&gp.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]);
        let mut segs = Vec::new();
        let mut left = payload.len();
        while left >= 255 {
            segs.push(255u8);
            left -= 255;
        }
        segs.push(left as u8);
        p.push(segs.len() as u8);
        p.extend(segs);
        p.extend_from_slice(payload);
        p
    };
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, channels]);
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut f = page(2, 0, 0, &head);
    f.extend(page(0, 0, 1, b"OpusTags\x00\x00\x00\x00\x00\x00\x00\x00"));
    f.extend(vec![0u8; padding]);
    f.extend(page(4, granule, 2, &[0u8; 20]));
    f
}

#[cfg(test)]
mod tests {
    use super::super::{probe_reader, Container};
    use super::build_opus;
    use std::io::Cursor;

    fn probe(v: Vec<u8>) -> mj_core::Result<super::MediaInfo> {
        let n = v.len() as u64;
        probe_reader(Cursor::new(v), n)
    }

    #[test]
    fn opus_identification_and_duration() {
        let i = probe(build_opus(1, 312 + 48_000 * 90, 5000)).unwrap();
        assert_eq!(i.container, Container::Ogg);
        assert_eq!((i.audio_codec.as_deref(), i.channels, i.sample_rate), (Some("opus"), Some(1), Some(48_000)));
        assert_eq!(i.duration_ms, Some(90_000));
    }

    #[test]
    fn truncation_is_safe() {
        let f = build_opus(2, 96_000, 100);
        for cut in 0..f.len() {
            let _ = probe(f[..cut].to_vec());
        }
        assert!(probe(b"OggS\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\x05hello".to_vec()).is_err());
    }
}
