//! Similar images (CLN-DET-02): fingerprint stability and distinctness, the
//! EXIF thumbnail fast path, PNG decoding, skip reporting and match modes.

mod common;

use common::*;
use mj_core::fingerprint::FingerprintFlags;
use mj_core::testutil::TempDir;
use mj_core::FingerprintStore;
use mj_dupes::decode::{self, png};
use mj_dupes::perceptual::{self, hamming};
use mj_dupes::similar_images::SimilarImages;
use mj_formats::exif::build::{app1, insert_app1, tiff, Value};

const FIX: &[(&str, &[u8])] = &[
    ("scene_a_q92.jpg", include_bytes!("fixtures/scene_a_q92.jpg")),
    ("scene_a_q60.jpg", include_bytes!("fixtures/scene_a_q60.jpg")),
    ("scene_a_small.jpg", include_bytes!("fixtures/scene_a_small.jpg")),
    ("scene_a_progressive.jpg", include_bytes!("fixtures/scene_a_progressive.jpg")),
    ("scene_b.jpg", include_bytes!("fixtures/scene_b.jpg")),
    ("scene_c_grey.jpg", include_bytes!("fixtures/scene_c_grey.jpg")),
];
const THUMB_A: &[u8] = include_bytes!("fixtures/thumb_a.jpg");
const SCENE_B: &[u8] = include_bytes!("fixtures/scene_b.jpg");

fn fixture(name: &str) -> &'static [u8] {
    FIX.iter().find(|(n, _)| *n == name).unwrap().1
}

#[test]
fn jpeg_fingerprints_are_stable_under_recompress_resize_and_progressive() {
    let a = decode::jpeg(fixture("scene_a_q92.jpg")).unwrap();
    assert_eq!((a.width, a.height), (320, 240));
    for v in ["scene_a_q60.jpg", "scene_a_small.jpg", "scene_a_progressive.jpg"] {
        let d = decode::jpeg(fixture(v)).unwrap();
        assert!(hamming(a.dhash, d.dhash) <= 6, "{v} dhash {}", hamming(a.dhash, d.dhash));
        assert!(hamming(a.phash, d.phash) <= 6, "{v} phash {}", hamming(a.phash, d.phash));
    }
    let t = decode::jpeg(THUMB_A).unwrap();
    assert!(hamming(a.dhash, t.dhash) <= 6);
    for other in ["scene_b.jpg", "scene_c_grey.jpg"] {
        let d = decode::jpeg(fixture(other)).unwrap();
        assert!(hamming(a.dhash, d.dhash) > 11, "{other} dhash {}", hamming(a.dhash, d.dhash));
        assert!(hamming(a.phash, d.phash) > 11, "{other} phash {}", hamming(a.phash, d.phash));
    }
}

fn write_fixtures(t: &TempDir) {
    for (n, b) in FIX {
        t.write(format!("photos/{n}"), b).unwrap();
    }
}

#[test]
fn groups_variants_and_leaves_unrelated_images_alone() {
    let t = TempDir::new("similar").unwrap();
    write_fixtures(&t);
    for mode in ["dhash", "phash", "both"] {
        let h = Harness::new(index_of(t.path())).text("match_mode", mode);
        let out = h.run(&SimilarImages).unwrap();
        assert_eq!(
            group_names(&out),
            vec![vec!["scene_a_progressive.jpg", "scene_a_q60.jpg", "scene_a_q92.jpg", "scene_a_small.jpg"]],
            "mode {mode}"
        );
        assert_one_keeper_per_group(&out);
        // Highest resolution is kept by default: never the 160x120 copy.
        assert_ne!(keeper_of(&out, "scene_a_small.jpg"), "scene_a_small.jpg");
        let g = &out.groups[0];
        assert!(g.confidence > 80.0 && g.confidence <= 100.0);
        assert_eq!(out.diagnostics.unreadable, 0);
        let small = out.candidates.iter().find(|c| c.path.ends_with("scene_a_small.jpg")).unwrap();
        assert_eq!(small.columns.get("resolution").map(String::as_str), Some("160x120"));
    }
    // Both fingerprints and the dimensions were cached from one decode.
    let h = Harness::new(index_of(t.path()));
    h.run(&SimilarImages).unwrap();
    let e = h.index.get(&t.join("photos/scene_b.jpg")).unwrap();
    let fp = h.store.get(&e.path, e.stamp()).unwrap();
    assert!(fp.dhash.is_some() && fp.phash.is_some());
    assert_eq!((fp.width, fp.height), (Some(320), Some(240)));
    assert!(fp.exif.is_some(), "absence of EXIF is a captured fact");
}

#[test]
fn threshold_zero_and_cap() {
    let t = TempDir::new("threshold").unwrap();
    write_fixtures(&t);
    let out = Harness::new(index_of(t.path())).num("threshold_percent", 0.0).run(&SimilarImages).unwrap();
    for g in &out.groups {
        assert_eq!(g.confidence, 100.0, "only identical fingerprints at 0 bits");
    }
    // 90 % is capped at 18 % (11 bits): unrelated scenes still stay apart.
    let out = Harness::new(index_of(t.path())).num("threshold_percent", 90.0).run(&SimilarImages).unwrap();
    assert!(group_names(&out).iter().all(|g| g.iter().all(|n| n.starts_with("scene_a"))), "{:?}", group_names(&out));
}

/// Scene B (320x240) carrying scene A's 160x120 thumbnail: what an editor that
/// copies EXIF verbatim leaves behind after a retouch.
fn b_with_thumb_of_a() -> Vec<u8> {
    let exif = tiff(
        &[(0x010F, Value::Ascii("TestCam".into())), (0x0112, Value::Short(1))],
        Some(&[(0, Value::Thumbnail(THUMB_A.to_vec()))]),
    );
    insert_app1(SCENE_B, &app1(&exif))
}

#[test]
fn embedded_thumbnail_is_used_before_a_full_decode() {
    let t = TempDir::new("thumb").unwrap();
    // Alone, the file is fingerprinted from its thumbnail (no full decode), while
    // the dimensions stay the main frame's.
    t.write("b_with_thumb_of_a.jpg", &b_with_thumb_of_a()).unwrap();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&SimilarImages).unwrap();
    assert!(out.groups.is_empty());
    let e = h.index.get(&t.join("b_with_thumb_of_a.jpg")).unwrap();
    let fp = h.store.get(&e.path, e.stamp()).unwrap();
    assert!(fp.flags.contains(FingerprintFlags::FROM_THUMBNAIL));
    assert_eq!((fp.width, fp.height), (Some(320), Some(240)));
    assert_eq!(fp.exif.as_ref().and_then(|x| x.camera.clone()).as_deref(), Some("TestCam"));
}

#[test]
fn a_stale_thumbnail_never_makes_a_different_picture_a_match() {
    // DUP-1: the thumbnail says scene A, the picture is scene B. A thumbnail match
    // is only a hint: the main image is decoded before anything is grouped.
    let t = TempDir::new("stalethumb").unwrap();
    // a.jpg carries its own (correct) thumbnail, so that both files have a
    // thumbnail fingerprint even when full decodes are off: the check of a
    // proposed match decodes the main pictures whatever that setting says.
    let exif = tiff(&[(0x0112, Value::Short(1))], Some(&[(0, Value::Thumbnail(THUMB_A.to_vec()))]));
    t.write("a.jpg", &insert_app1(fixture("scene_a_q92.jpg"), &app1(&exif))).unwrap();
    t.write("b_with_thumb_of_a.jpg", &b_with_thumb_of_a()).unwrap();
    for decode_full in [true, false] {
        let h = Harness::new(index_of(t.path())).flag("decode_full", decode_full);
        let out = h.run(&SimilarImages).unwrap();
        assert!(out.groups.is_empty(), "decode_full={decode_full}: {:?}", group_names(&out));
        assert!(out.candidates.iter().all(|c| !c.default_selected));
        // The checked fingerprint replaces the thumbnail's in the cache.
        let e = h.index.get(&t.join("b_with_thumb_of_a.jpg")).unwrap();
        let fp = h.store.get(&e.path, e.stamp()).unwrap();
        assert!(fp.flags.contains(FingerprintFlags::MAIN_DECODED), "{:?}", fp.flags);
        assert_eq!(fp.dhash, Some(decode::jpeg(SCENE_B).unwrap().dhash));
        // A second scan from the cache gives the same answer without a decode.
        assert!(h.run(&SimilarImages).unwrap().groups.is_empty());
    }
}

#[test]
fn a_matching_thumbnail_still_groups_after_the_check() {
    let t = TempDir::new("goodthumb").unwrap();
    let exif = tiff(&[(0x0112, Value::Short(1))], Some(&[(0, Value::Thumbnail(THUMB_A.to_vec()))]));
    t.write("a_with_thumb.jpg", &insert_app1(fixture("scene_a_q92.jpg"), &app1(&exif))).unwrap();
    t.write("a_q60.jpg", fixture("scene_a_q60.jpg")).unwrap();
    let out = Harness::new(index_of(t.path())).run(&SimilarImages).unwrap();
    assert_eq!(group_names(&out), vec![vec!["a_q60.jpg", "a_with_thumb.jpg"]]);
    assert_eq!(out.candidates.iter().filter(|c| c.default_selected).count(), 1);
}

#[test]
fn a_thumbnail_of_another_shape_is_not_trusted() {
    // The main frame claims 320x320 while the thumbnail is 4:3 (a crop that kept
    // the old EXIF): the thumbnail is not used as the image's fingerprint.
    let t = TempDir::new("cropthumb").unwrap();
    let mut b = b_with_thumb_of_a();
    let sof = (2..b.len() - 9).rev().find(|&i| b[i] == 0xFF && b[i + 1] == 0xC0).unwrap();
    b[sof + 5..sof + 7].copy_from_slice(&320u16.to_be_bytes());
    t.write("cropped.jpg", &b).unwrap();
    let h = Harness::new(index_of(t.path()));
    h.run(&SimilarImages).unwrap();
    let e = h.index.get(&t.join("cropped.jpg")).unwrap();
    let fp = h.store.get(&e.path, e.stamp()).unwrap();
    assert!(!fp.flags.contains(FingerprintFlags::FROM_THUMBNAIL), "{:?}", fp.flags);
}

#[test]
fn no_thumbnail_and_no_full_decode_is_reported_as_skipped() {
    let t = TempDir::new("nothumb").unwrap();
    write_fixtures(&t);
    let h = Harness::new(index_of(t.path())).flag("decode_full", false);
    let out = h.run(&SimilarImages).unwrap();
    assert!(out.groups.is_empty());
    assert_eq!(out.diagnostics.skipped.get("no_thumbnail"), Some(&6));
    let e = h.index.get(&t.join("photos/scene_b.jpg")).unwrap();
    assert!(h.store.get(&e.path, e.stamp()).unwrap().flags.contains(FingerprintFlags::NO_THUMBNAIL));
    // Turning full decodes back on recomputes them (the flag is not a permanent verdict).
    let mut h2 = Harness::new(index_of(t.path()));
    h2.store = h.store;
    assert_eq!(h2.run(&SimilarImages).unwrap().groups.len(), 1);
}

#[test]
fn corrupt_images_are_skipped_and_the_failure_is_cached() {
    let t = TempDir::new("corrupt").unwrap();
    write_fixtures(&t);
    let mut bad = fixture("scene_b.jpg")[..600].to_vec();
    bad.extend(noise(1, 400));
    t.write("photos/broken.jpg", &bad).unwrap();
    t.write("photos/not_an_image.png", b"plain text pretending to be a png").unwrap();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&SimilarImages).unwrap();
    assert_eq!(out.groups.len(), 1);
    let skipped: u64 = out.diagnostics.skipped.values().sum();
    assert!(skipped >= 2, "{:?}", out.diagnostics);
    let again = h.run(&SimilarImages).unwrap();
    assert_eq!(again.diagnostics.skipped, out.diagnostics.skipped, "cached verdicts give the same report");
}

#[test]
fn exact_copies_are_reported_once() {
    let t = TempDir::new("exactcopies").unwrap();
    write_fixtures(&t);
    t.write("backup/scene_a_q92.jpg", fixture("scene_a_q92.jpg")).unwrap();
    let out = Harness::new(index_of(t.path())).run(&SimilarImages).unwrap();
    assert_eq!(out.diagnostics.skipped.get("exact_copy"), Some(&1));
    assert_eq!(out.groups[0].members.len(), 4, "the byte-identical copy is left to the exact detector");
}

/// A deterministic synthetic scene (gradient, blocks and a disc).
fn synthetic(w: usize, h: usize, seed: u64) -> Vec<u8> {
    let n = noise(seed, 16);
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f64 / w as f64;
            let fy = y as f64 / h as f64;
            let mut v = 40.0 + 120.0 * (fx * (1.0 + n[0] as f64 / 128.0)).sin().abs() + 60.0 * fy;
            for k in 0..4 {
                let cx = n[1 + k * 3] as f64 / 255.0;
                let cy = n[2 + k * 3] as f64 / 255.0;
                let r = 0.08 + n[3 + k * 3] as f64 / 1200.0;
                if (fx - cx).powi(2) + (fy - cy).powi(2) < r * r {
                    v = if k % 2 == 0 { 240.0 } else { 15.0 };
                }
            }
            px[y * w + x] = v.clamp(0.0, 255.0) as u8;
        }
    }
    px
}

fn box_down(px: &[u8], w: usize, h: usize, f: usize) -> Vec<u8> {
    let (nw, nh) = (w / f, h / f);
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let mut s = 0u32;
            for dy in 0..f {
                for dx in 0..f {
                    s += px[(y * f + dy) * w + x * f + dx] as u32;
                }
            }
            out[y * nw + x] = (s / (f * f) as u32) as u8;
        }
    }
    out
}

#[test]
fn png_variants_group_and_unrelated_do_not() {
    let (w, h) = (240, 180);
    let base = synthetic(w, h, 11);
    let half = box_down(&base, w, h, 2);
    let noisy: Vec<u8> =
        base.iter().zip(noise(5, w * h)).map(|(&p, n)| p.saturating_add(n % 7).saturating_sub(3)).collect();
    let rgb: Vec<u8> = base.iter().flat_map(|&v| [v, v, v]).collect();
    let other = synthetic(w, h, 99);
    let t = TempDir::new("png").unwrap();
    t.write("base.png", &png::encode(w as u32, h as u32, 1, &base, true)).unwrap();
    t.write("half.png", &png::encode((w / 2) as u32, (h / 2) as u32, 1, &half, false)).unwrap();
    t.write("noisy.png", &png::encode(w as u32, h as u32, 1, &noisy, true)).unwrap();
    t.write("rgb.png", &png::encode(w as u32, h as u32, 3, &rgb, true)).unwrap();
    t.write("other.png", &png::encode(w as u32, h as u32, 1, &other, false)).unwrap();
    let (d0, p0) = perceptual::fingerprints(w, h, &base);
    let (d1, p1) = perceptual::fingerprints(w, h, &other);
    assert!(hamming(d0, d1) > 11 && hamming(p0, p1) > 11);
    let out = Harness::new(index_of(t.path())).run(&SimilarImages).unwrap();
    assert_eq!(group_names(&out), vec![vec!["base.png", "half.png", "noisy.png", "rgb.png"]]);
    assert_ne!(keeper_of(&out, "half.png"), "half.png", "the full-size image is kept");
}

#[test]
fn flat_images_of_different_colours_never_match_in_any_mode() {
    // DUP-3: black, dark grey and mid grey are different pictures.
    let (w, h) = (64u32, 48u32);
    let t = TempDir::new("flat").unwrap();
    for (n, v) in [("black.png", 0u8), ("dark.png", 64), ("grey.png", 128)] {
        t.write(n, &png::encode(w, h, 1, &vec![v; (w * h) as usize], false)).unwrap();
    }
    for mode in ["dhash", "phash", "both"] {
        let out = Harness::new(index_of(t.path())).text("match_mode", mode).run(&SimilarImages).unwrap();
        assert!(out.groups.is_empty(), "mode {mode}: {:?}", group_names(&out));
        assert_eq!(out.diagnostics.skipped.get("flat_image"), Some(&3), "mode {mode}");
    }
}

#[test]
fn a_thumbnail_decode_is_bounded_before_any_pixel() {
    // DUP-5: an EXIF "thumbnail" declaring a huge frame is refused from its header
    // (EXIF thumbnails are 160x120), the main picture is decoded instead.
    let a = fixture("scene_a_q92.jpg");
    assert_eq!(decode::jpeg_limited(a, 320 * 240 - 1), Err(decode::DecodeError::TooLarge));
    assert!(decode::jpeg_limited(a, 320 * 240).is_ok());
    const { assert!(320 * 240 <= mj_dupes::similar_images::MAX_THUMBNAIL_PIXELS) };
    const { assert!(8000 * 8000 > mj_dupes::similar_images::MAX_THUMBNAIL_PIXELS) };
}
