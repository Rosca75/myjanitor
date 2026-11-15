//! Review thumbnails (W6-A) on real JPEG fixtures: full decode, the EXIF embedded
//! thumbnail fast path, EXIF orientation, and the letterbox guard.

use mj_dupes::decode::thumb::{self, THUMB_MAX_PIXELS};
use mj_dupes::decode::DecodeError;
use mj_formats::exif::build::{app1, insert_app1, tiff, Value};

const SCENE_A: &[u8] = include_bytes!("fixtures/scene_a_q92.jpg"); // 320 x 240
const SCENE_GREY: &[u8] = include_bytes!("fixtures/scene_c_grey.jpg"); // 320 x 240
const PROGRESSIVE: &[u8] = include_bytes!("fixtures/scene_a_progressive.jpg");
const THUMB_A: &[u8] = include_bytes!("fixtures/thumb_a.jpg"); // 160 x 120

fn with_exif(jpeg: &[u8], orientation: u16, thumb: Option<&[u8]>) -> Vec<u8> {
    let ifd1 = thumb.map(|t| vec![(0u16, Value::Thumbnail(t.to_vec()))]);
    let t = tiff(&[(0x0112, Value::Short(orientation))], ifd1.as_deref());
    insert_app1(jpeg, &app1(&t))
}

#[test]
fn full_decode_downscales_colour_grey_and_progressive() {
    for src in [SCENE_A, SCENE_GREY, PROGRESSIVE] {
        let t = thumb::jpeg(src, 128, THUMB_MAX_PIXELS).unwrap();
        assert_eq!((t.width, t.height, t.channels), (128, 96, 3));
        assert_eq!(t.pixels.len(), 128 * 96 * 3);
        let png = thumb::encode_png(&t);
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.len() < 64 * 1024, "{} bytes", png.len());
    }
    // Larger request (the preview pane): never upscaled.
    let t = thumb::jpeg(SCENE_A, 512, THUMB_MAX_PIXELS).unwrap();
    assert_eq!((t.width, t.height), (320, 240));
}

#[test]
fn embedded_thumbnail_is_used_for_row_thumbnails_only() {
    let src = with_exif(SCENE_A, 1, Some(THUMB_A));
    let fast = thumb::jpeg_embedded(&src, 128).expect("fast path");
    assert_eq!((fast.width, fast.height), (128, 96));
    // A header window holding the EXIF segment is enough for the fast path.
    let window = &src[..src.len() - 1000];
    assert!(thumb::jpeg_embedded(window, 128).is_some());
    // The preview pane wants more pixels than an embedded thumbnail has.
    assert!(thumb::jpeg_embedded(&src, 512).is_none());
    assert_eq!(thumb::jpeg(&src, 512, THUMB_MAX_PIXELS).unwrap().width, 320);
}

#[test]
fn orientation_is_applied_on_both_paths() {
    let src = with_exif(SCENE_A, 6, Some(THUMB_A));
    let fast = thumb::jpeg_embedded(&src, 128).unwrap();
    assert_eq!((fast.width, fast.height), (96, 128), "rotated a quarter turn");
    let full = thumb::jpeg(&with_exif(SCENE_A, 8, None), 128, THUMB_MAX_PIXELS).unwrap();
    assert_eq!((full.width, full.height), (96, 128));
}

#[test]
fn letterboxed_embedded_thumbnail_is_refused() {
    // The same 4:3 thumbnail declared for a 320 x 100 image: its aspect does not match,
    // so it would show bars (or the wrong crop); the fast path refuses it.
    let mut src = with_exif(SCENE_A, 1, Some(THUMB_A));
    let sof = (2..src.len() - 9).find(|&i| src[i] == 0xFF && matches!(src[i + 1], 0xC0 | 0xC2)).unwrap();
    src[sof + 5..sof + 7].copy_from_slice(&100u16.to_be_bytes());
    assert!(thumb::jpeg_embedded(&src, 128).is_none());
}

#[test]
fn pixel_cap_refuses_before_decoding() {
    assert_eq!(thumb::jpeg(SCENE_A, 128, 1000), Err(DecodeError::TooLarge));
}
