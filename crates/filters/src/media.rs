//! Media-metadata conditions (CLN-CNT-03). Owner: W2-C.
//!
//! This module only *matches*: the caller supplies a [`MediaMeta`] record per
//! file (extracted by `mj-formats` or an external tool, other workstreams'
//! job) through a [`MediaMetaProvider`]. A condition on a field the record
//! does not carry never matches, and files without any record are counted, not
//! silently dropped. The conditions are serialisable so the UI's "field picker and
//! condition" can store them, but they are not part of the frozen `Rule`
//! tree: the engine applies them as a second pass over an already selected
//! set with [`filter_by_media`], so metadata is extracted only for files that
//! survived every cheaper test.

use mj_core::{CancelToken, Cmp, DateValue, Error, FileEntry, FileIndex, Result, TextMode, Timestamp};
use serde::{Deserialize, Serialize};

use crate::eval::FilterContext;
use crate::matcher::TextMatcher;
use crate::timeutil;

/// Metadata of one media file; every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    /// EXIF capture time.
    pub date_taken: Option<Timestamp>,
    /// `Some(false)` means "checked, no GPS"; `None` means unknown.
    pub has_gps: Option<bool>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
}

/// Numeric fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumField {
    Width,
    Height,
    /// `width * height`.
    Pixels,
    DurationMs,
    BitrateKbps,
    Year,
}

/// Text fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    /// Video or audio codec.
    Codec,
    VideoCodec,
    AudioCodec,
    CameraMake,
    CameraModel,
    Title,
    Artist,
    Album,
    Genre,
}

/// One media condition. All conditions of a [`MediaMatcher`] must hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaCondition {
    Number {
        field: NumField,
        cmp: Cmp,
        value: u64,
    },
    Text {
        field: TextField,
        mode: TextMode,
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    /// Date taken, absolute or relative.
    DateTaken {
        cmp: Cmp,
        value: DateValue,
    },
    /// GPS coordinates present (`true`) or absent (`false`).
    Gps {
        present: bool,
    },
}

fn number(m: &MediaMeta, f: NumField) -> Option<u64> {
    match f {
        NumField::Width => m.width.map(u64::from),
        NumField::Height => m.height.map(u64::from),
        NumField::Pixels => Some(u64::from(m.width?) * u64::from(m.height?)),
        NumField::DurationMs => m.duration_ms,
        NumField::BitrateKbps => m.bitrate_kbps.map(u64::from),
        NumField::Year => m.year.map(u64::from),
    }
}

fn text_values(m: &MediaMeta, f: TextField) -> Vec<&str> {
    fn one(o: &Option<String>) -> Vec<&str> {
        o.as_deref().into_iter().collect()
    }
    match f {
        TextField::Codec => m.video_codec.iter().chain(m.audio_codec.iter()).map(String::as_str).collect(),
        TextField::VideoCodec => one(&m.video_codec),
        TextField::AudioCodec => one(&m.audio_codec),
        TextField::CameraMake => one(&m.camera_make),
        TextField::CameraModel => one(&m.camera_model),
        TextField::Title => one(&m.title),
        TextField::Artist => one(&m.artist),
        TextField::Album => one(&m.album),
        TextField::Genre => one(&m.genre),
    }
}

enum Compiled {
    Number(NumField, Cmp, u64),
    Text(TextField, TextMatcher),
    Date(Cmp, Timestamp),
    Gps(bool),
}

/// Compiled media conditions (AND).
pub struct MediaMatcher {
    conds: Vec<Compiled>,
}

impl std::fmt::Debug for MediaMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MediaMatcher({} conditions)", self.conds.len())
    }
}

impl MediaMatcher {
    /// Compile conditions; invalid patterns and out-of-range dates are `InvalidInput`.
    pub fn compile(conds: &[MediaCondition], ctx: &FilterContext<'_>) -> Result<MediaMatcher> {
        let mut out = Vec::with_capacity(conds.len());
        for c in conds {
            out.push(match c {
                MediaCondition::Number { field, cmp, value } => Compiled::Number(*field, *cmp, *value),
                MediaCondition::Text { field, mode, pattern, case_sensitive } => Compiled::Text(
                    *field,
                    TextMatcher::new(*mode, pattern, *case_sensitive, false)
                        .map_err(|e| Error::InvalidInput(format!("media condition '{pattern}': {e}")))?,
                ),
                MediaCondition::DateTaken { cmp, value } => {
                    let at = match value {
                        DateValue::At(t) => *t,
                        DateValue::Ago(s) => timeutil::span_before(*s, ctx.now)
                            .map_err(|e| Error::InvalidInput(format!("date taken condition: {e}")))?,
                    };
                    Compiled::Date(*cmp, at)
                }
                MediaCondition::Gps { present } => Compiled::Gps(*present),
            });
        }
        Ok(MediaMatcher { conds: out })
    }

    /// True when every condition holds. A missing field fails its condition.
    pub fn matches(&self, m: &MediaMeta) -> bool {
        self.conds.iter().all(|c| match c {
            Compiled::Number(f, cmp, v) => number(m, *f).is_some_and(|x| cmp.test(x, *v)),
            Compiled::Text(f, tm) => text_values(m, *f).into_iter().any(|t| tm.is_match(t)),
            Compiled::Date(cmp, at) => m.date_taken.is_some_and(|t| cmp.test(t.0, at.0)),
            Compiled::Gps(want) => m.has_gps == Some(*want),
        })
    }
}

/// Supplies media metadata for a file (`None`: not a media file or unreadable).
pub trait MediaMetaProvider: Send + Sync {
    /// Metadata of `entry`.
    fn media_meta(&self, entry: &FileEntry) -> Option<MediaMeta>;
}

/// Result of [`filter_by_media`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MediaFilterResult {
    /// Surviving positions (index order).
    pub positions: Vec<usize>,
    /// Files the provider had no metadata for (they cannot match; reported, not hidden).
    pub without_metadata: u64,
}

/// Second pass: keep the `positions` whose media metadata satisfies `matcher`.
pub fn filter_by_media(
    index: &FileIndex,
    positions: &[usize],
    provider: &dyn MediaMetaProvider,
    matcher: &MediaMatcher,
    cancel: Option<&CancelToken>,
) -> Result<MediaFilterResult> {
    let mut out = MediaFilterResult::default();
    for &p in positions {
        if let Some(c) = cancel {
            c.checkpoint()?;
        }
        match provider.media_meta(index.entry(p)) {
            Some(m) if matcher.matches(&m) => out.positions.push(p),
            Some(_) => {}
            None => out.without_metadata += 1,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::time::{RelativeSpan, SpanUnit};
    use mj_core::{EntryKind, ScanStats};
    use std::collections::BTreeMap;

    fn ctx(g: &BTreeMap<String, Vec<String>>) -> FilterContext<'_> {
        FilterContext { now: Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0), type_groups: g }
    }

    fn meta() -> MediaMeta {
        MediaMeta {
            width: Some(4000),
            height: Some(3000),
            video_codec: Some("H.264".into()),
            audio_codec: Some("AAC".into()),
            bitrate_kbps: Some(8000),
            duration_ms: Some(95_000),
            camera_make: Some("Canon".into()),
            date_taken: Some(Timestamp::from_ymd_hms(2020, 5, 1, 0, 0, 0)),
            has_gps: Some(false),
            ..MediaMeta::default()
        }
    }

    #[test]
    fn numbers_text_dates_and_gps() {
        let g = BTreeMap::new();
        let c = ctx(&g);
        let m = |conds: Vec<MediaCondition>| MediaMatcher::compile(&conds, &c).unwrap();
        let n = |field, cmp, value| MediaCondition::Number { field, cmp, value };
        assert!(m(vec![n(NumField::Pixels, Cmp::Ge, 12_000_000)]).matches(&meta()));
        assert!(!m(vec![n(NumField::Width, Cmp::Lt, 1920)]).matches(&meta()));
        assert!(!m(vec![n(NumField::Year, Cmp::Gt, 0)]).matches(&meta()), "missing field never matches");
        let t = |field, p: &str| MediaCondition::Text {
            field,
            mode: TextMode::Contains,
            pattern: p.into(),
            case_sensitive: false,
        };
        assert!(m(vec![t(TextField::Codec, "aac")]).matches(&meta()), "codec checks video and audio");
        assert!(m(vec![t(TextField::CameraMake, "canon")]).matches(&meta()));
        assert!(!m(vec![t(TextField::Artist, "x")]).matches(&meta()));
        let old = MediaCondition::DateTaken {
            cmp: Cmp::Lt,
            value: DateValue::Ago(RelativeSpan { amount: 2, unit: SpanUnit::Years }),
        };
        assert!(m(vec![old]).matches(&meta()));
        assert!(m(vec![MediaCondition::Gps { present: false }]).matches(&meta()));
        assert!(!m(vec![MediaCondition::Gps { present: true }]).matches(&meta()));
        assert!(!m(vec![MediaCondition::Gps { present: false }]).matches(&MediaMeta::default()), "unknown GPS");
        assert!(MediaMatcher::compile(&[], &c).unwrap().matches(&MediaMeta::default()));
    }

    #[test]
    fn bad_pattern_and_json_shape() {
        let g = BTreeMap::new();
        let bad = MediaCondition::Text {
            field: TextField::Title,
            mode: TextMode::Regex,
            pattern: "(".into(),
            case_sensitive: false,
        };
        assert!(MediaMatcher::compile(&[bad], &ctx(&g)).is_err());
        let json = r#"{"kind":"number","field":"duration_ms","cmp":"gt","value":60000}"#;
        let c: MediaCondition = serde_json::from_str(json).unwrap();
        assert_eq!(c, MediaCondition::Number { field: NumField::DurationMs, cmp: Cmp::Gt, value: 60000 });
    }

    struct Provider;
    impl MediaMetaProvider for Provider {
        fn media_meta(&self, e: &FileEntry) -> Option<MediaMeta> {
            if e.name() == "a.mp4" {
                Some(meta())
            } else if e.name() == "b.mp4" {
                Some(MediaMeta { width: Some(640), ..MediaMeta::default() })
            } else {
                None
            }
        }
    }

    #[test]
    fn second_pass_reports_files_without_metadata() {
        let g = BTreeMap::new();
        let entries =
            ["/m/a.mp4", "/m/b.mp4", "/m/c.mp4"].iter().map(|p| FileEntry::new(*p, EntryKind::File, 5)).collect();
        let index = FileIndex::new(vec!["/m".into()], entries, ScanStats::default(), Timestamp::default());
        let matcher = MediaMatcher::compile(
            &[MediaCondition::Number { field: NumField::Width, cmp: Cmp::Ge, value: 1920 }],
            &ctx(&g),
        )
        .unwrap();
        let r = filter_by_media(&index, &[0, 1, 2], &Provider, &matcher, None).unwrap();
        assert_eq!(r, MediaFilterResult { positions: vec![0], without_metadata: 1 });
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(filter_by_media(&index, &[0], &Provider, &matcher, Some(&cancel)).is_err());
    }
}
