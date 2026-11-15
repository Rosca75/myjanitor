# W2-B dependency entries

Same columns as `docs/DEPENDENCIES.md`. Pinned in `crates/dupes/Cargo.toml`
(the orchestrator hoists it to `[workspace.dependencies]` on merge, D-120).

| Crate | Version | Used by | Purpose | Rewrite estimate (why > 5 h) | Licence | Transitive |
|---|---|---|---|---|---|---|
| `zune-jpeg` | =0.5.15 | `mj-dupes` (`decode::jpeg`) | Decode JPEG files and EXIF thumbnails straight to luma for dHash/pHash (CLN-DET-02) | A correct baseline **and** progressive decoder (Huffman with restart markers, successive approximation/spectral selection, chroma subsampling variants, 12-bit tolerance, CMYK/YCCK, fuzz-hardened) is several days of work plus a test corpus; well over 5 h. Pure Rust, focused, no `image` crate. Uses `unsafe` internally for SIMD; our crates stay `forbid(unsafe_code)`, and every decode runs inside `catch_unwind` | MIT OR Apache-2.0 OR Zlib | 1 (`zune-core` =0.5.3, same licence) |

Not added: PNG decoding is in-house (`crates/dupes/src/decode/png.rs`) on top of
the already-allowed `miniz_oxide` inflate; `miniz_oxide` was added to
`mj-dupes`'s `Cargo.toml` as a workspace dependency (no new crate).
`Cargo.lock` gains `zune-jpeg` 0.5.15 and `zune-core` 0.5.3 (committed separately).
