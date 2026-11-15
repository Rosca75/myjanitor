//! DUP-5: decoding a PNG keeps O(width) memory, whatever the image's raw size.
//! Alone in its test binary: the counting allocator sees only this test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use mj_dupes::decode::png;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::SeqCst) + l.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static A: Counting = Counting;

fn chunk(out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(data);
    out.extend_from_slice(&[0; 4]); // CRCs are not verified
}

#[test]
fn a_large_rgba16_png_decodes_in_bounded_memory() {
    // 2000 x 2000 RGBA16: 32 MB of raw scanlines from a file of a few KB.
    let (w, h) = (2000u32, 2000u32);
    let row = w as usize * 8;
    let mut raw = vec![0u8; (row + 1) * h as usize];
    for (y, line) in raw.chunks_exact_mut(row + 1).enumerate() {
        // A gradient so the hashes are not trivially flat: one sample per row changes.
        line[1 + (y % row)] = 0xFF;
    }
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 1);
    drop(raw);
    let mut file = png::SIGNATURE.to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[16, 6, 0, 0, 0]);
    chunk(&mut file, b"IHDR", &ihdr);
    for part in z.chunks(8192) {
        chunk(&mut file, b"IDAT", part);
    }
    chunk(&mut file, b"IEND", &[]);
    assert!(file.len() < 1 << 20, "{}", file.len());

    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let d = png::decode(&file).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - base;
    assert_eq!((d.width, d.height), (w, h));
    assert!(peak < 4 << 20, "decode peaked at {peak} bytes over the file");
}
