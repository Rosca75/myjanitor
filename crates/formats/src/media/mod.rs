//! Media metadata without decoding: MP4/MOV boxes, WAV/AIFF headers, FLAC
//! STREAMINFO, MP3 (ID3v2 plus first-frame estimate) and Ogg Opus/Vorbis
//! identification headers. Owner: W3-C.
//!
//! The output is one [`MediaInfo`] record (duration, codecs, bitrate,
//! resolution, channels, a small tag subset) used by CLN-CNT-03 conditions,
//! by estimates (bitrate arithmetic) and by the screen-recording heuristics.
//!
//! Hardening: every read goes through [`Src`], which bounds a single read
//! (4 MiB) and the total read per file (32 MiB), checks offsets against the
//! file length and never trusts a declared size. Box and chunk walks have
//! depth and count guards. Malformed input returns `Error::Parse`; nothing
//! here panics.

mod flac;
mod mp3;
mod mp4;
mod ogg;
mod riff;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use mj_core::{Error, Result};
use serde::Serialize;

/// Test-support builders for hand-made media buffers (also used by `mj-optimise` tests).
#[doc(hidden)]
pub mod build {
    pub use super::flac::build_flac;
    pub use super::mp3::build_mp3;
    pub use super::mp4::build as mp4;
    pub use super::ogg::build_opus;
    pub use super::riff::build::{aiff, wav};
}

/// Container family of a probed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Mp4,
    Mov,
    Wav,
    Aiff,
    Flac,
    Mp3,
    Ogg,
}

/// What a header walk learned about a media file. Every field is optional: absence is not an error.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaInfo {
    pub container: Container,
    pub file_size: u64,
    pub duration_ms: Option<u64>,
    /// Lower-case codec name (`h264`, `hevc`, `av1`, `aac`, `opus`, `pcm`, `flac`, `mp3`...).
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f32>,
    pub channels: Option<u16>,
    pub sample_rate: Option<u32>,
    pub bits_per_sample: Option<u16>,
    /// Whole-file average in kbit/s (file size over duration), when the duration is known.
    pub bitrate_kbps: Option<u32>,
    pub video_tracks: u32,
    pub audio_tracks: u32,
    pub subtitle_tracks: u32,
    /// Subset of tags, lower-case keys: `title`, `artist`, `album`, `date`, `genre`, `comment`.
    pub tags: BTreeMap<String, String>,
    /// Bytes of embedded cover art (ID3 APIC, FLAC PICTURE, MP4 `covr`).
    pub cover_art_bytes: Option<u64>,
    /// Writing application (`©too`, WAV `ISFT`...), used for screen-recorder detection.
    pub encoder: Option<String>,
}

impl MediaInfo {
    pub(crate) fn new(container: Container, file_size: u64) -> Self {
        MediaInfo {
            container,
            file_size,
            duration_ms: None,
            video_codec: None,
            audio_codec: None,
            width: None,
            height: None,
            frame_rate: None,
            channels: None,
            sample_rate: None,
            bits_per_sample: None,
            bitrate_kbps: None,
            video_tracks: 0,
            audio_tracks: 0,
            subtitle_tracks: 0,
            tags: BTreeMap::new(),
            cover_art_bytes: None,
            encoder: None,
        }
    }

    /// Whole-file average bitrate from size and duration.
    pub(crate) fn finish(mut self) -> Self {
        if let Some(ms) = self.duration_ms.filter(|&m| m > 0) {
            let kbps = (u128::from(self.file_size) * 8 * 1000 / u128::from(ms) / 1000).min(u128::from(u32::MAX));
            self.bitrate_kbps = Some(kbps as u32);
        }
        self
    }

    /// Duration in seconds.
    pub fn duration_s(&self) -> Option<f64> {
        self.duration_ms.map(|m| m as f64 / 1000.0)
    }
}

/// Bounded random-access reader over any `Read + Seek`.
pub(crate) struct Src<R> {
    r: R,
    pub len: u64,
    budget: u64,
}

impl<R: Read + Seek> Src<R> {
    const MAX_READ: u64 = 4 * 1024 * 1024;
    const BUDGET: u64 = 32 * 1024 * 1024;

    pub fn new(r: R, len: u64) -> Self {
        Src { r, len, budget: Self::BUDGET }
    }

    /// Read exactly `n` bytes at `pos`; an error when out of range or over budget.
    pub fn read_at(&mut self, pos: u64, n: u64) -> Result<Vec<u8>> {
        let end = pos
            .checked_add(n)
            .filter(|&e| e <= self.len)
            .ok_or_else(|| Error::parse("media", "read past end of file"))?;
        if n > Self::MAX_READ || n > self.budget {
            return Err(Error::parse("media", "read budget exceeded"));
        }
        self.budget -= n;
        let io = |e: std::io::Error| Error::Io { path: None, kind: "read", message: e.to_string() };
        self.r.seek(SeekFrom::Start(pos)).map_err(io)?;
        let mut v = vec![0u8; n as usize];
        self.r.read_exact(&mut v).map_err(io)?;
        let _ = end;
        Ok(v)
    }

    /// Like [`Src::read_at`] but shortens the read at the end of the file.
    pub fn read_upto(&mut self, pos: u64, n: u64) -> Result<Vec<u8>> {
        let n = n.min(self.len.saturating_sub(pos));
        self.read_at(pos, n)
    }
}

pub(crate) fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
pub(crate) fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
pub(crate) fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}
pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
pub(crate) fn le64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// Printable, trimmed, length-bounded copy of tag text.
pub(crate) fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect::<String>().trim().to_string()
}

/// Probe a file on disk.
pub fn probe(path: &Path) -> Result<MediaInfo> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let len = f.metadata().map_err(|e| Error::io(path, e))?.len();
    probe_reader(f, len)
}

/// Probe any seekable source of `len` bytes. The container is detected from content, not the name.
pub fn probe_reader<R: Read + Seek>(r: R, len: u64) -> Result<MediaInfo> {
    let mut s = Src::new(r, len);
    let head = s.read_upto(0, 16)?;
    let unsupported = || Error::parse("media", "not a supported media container");
    if head.len() < 12 {
        return Err(unsupported());
    }
    match &head[..4] {
        b"RIFF" | b"RF64" if &head[8..12] == b"WAVE" => riff::wav(&mut s),
        b"FORM" if matches!(&head[8..12], b"AIFF" | b"AIFC") => riff::aiff(&mut s),
        b"fLaC" => flac::parse(&mut s, 0),
        b"OggS" => ogg::parse(&mut s),
        b"ID3\x02" | b"ID3\x03" | b"ID3\x04" => mp3::parse(&mut s),
        _ if &head[4..8] == b"ftyp" => mp4::parse(&mut s),
        _ if head[0] == 0xFF && head[1] & 0xE0 == 0xE0 => mp3::parse(&mut s),
        _ => Err(unsupported()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn garbage_and_truncation_never_panic() {
        assert!(probe_reader(Cursor::new(b"hello".to_vec()), 5).is_err());
        assert!(probe_reader(Cursor::new(Vec::new()), 0).is_err());
        let mut x: u32 = 7;
        for heads in [
            &b"RIFF\0\0\0\0WAVE"[..],
            b"FORM\0\0\0\0AIFF",
            b"fLaC\0\0\0\0\0\0\0\0",
            b"OggS\0\0\0\0\0\0\0\0",
            b"ID3\x03\0\0\0\0\0\0\0\0",
            b"\0\0\0\x18ftypmp42\0\0\0\0",
            &[0xFF, 0xFB, 0x90, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ] {
            for _ in 0..200 {
                let mut v = heads.to_vec();
                for _ in 0..400 {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    v.push((x >> 16) as u8);
                }
                let n = v.len() as u64;
                let _ = probe_reader(Cursor::new(v), n);
            }
        }
    }
}
