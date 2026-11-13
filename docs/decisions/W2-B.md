# W2-B decisions (duplicates and similarity)

Format of `docs/DECISIONS.md`; ids D-2Bxx.

## D-2B01 Exact-group confirmation is SHA-256, computed in the full-hash pass
- **Context:** research §3.2 asks for a byte compare or a second hash before a delete is offered; the stub allows "SHA-256 or byte confirmation".
- **Choice:** stage 4 streams each partial-collision file once into both xxh64 and SHA-256; groups are equal `(size, full xxh64, SHA-256)`. Small files (<= 192 KiB) get both from the bytes read in the partial stage. Both values are cached (`Fingerprints::full`, `sha256`).
- **Why:** confirmation without a second read, and it survives rescans; a byte compare would re-read every duplicate on every scan.

## D-2B02 Every read stage uses the adaptive runner
- **Choice:** partial, full and image-header reads all go through `staged::adaptive::run` (parallel, abort when more than half of at least 12 attempts failed, serial retry with 25/50 ms back-off). NotFound, PermissionDenied, "changed during scan" and panics are not retried.
- **Why:** zero cost on healthy disks; simpler than two lanes. A size or type change between the scan and the read is counted as `skipped["changed_during_scan"]`; every other failure is `unreadable`.

## D-2B03 Hard links and online-only files
- **Choice:** one path per `FileId` (first in index order) takes part; aliases are counted as `skipped["hardlink"]`. Online-only placeholders are never opened (`skipped["online_only"]`). Members with `links > 1` carry `Badges::HARDLINKED`.
- **Why:** hard links share content and space; reading a placeholder downloads it.

## D-2B04 JPEGs without an embedded thumbnail are fully decoded by default (deviation)
- **Context:** ARCHITECTURE.md §6 / research invariant 8 say "no full-decode fallback". That rule was measured on HEIC decoded through WASM (3-13 s per file). The W2-B brief asks for the thumbnail fast path first and a capped full decode.
- **Choice:** order = cache -> EXIF thumbnail from the first 128 KiB (widened once to 1 MiB when IFD1 points just past it) -> full decode with `zune-jpeg` (tens of ms per 12 MP image). Bounded: files over 64 MiB, images over 64 MP or 32768 px a side are `too_large`; at most two images over 16 MP decode at once. A toggle `decode_full` (advanced, default on) restores the strict reference behaviour: such files are flagged `NO_THUMBNAIL` and counted `skipped["no_thumbnail"]`. PNG has no thumbnails and is always decoded within the same limits.
- **Why:** a general cleaner sees many JPEGs without EXIF (web, messaging apps); skipping all of them would hide most similar images. Needs an orchestrator note in ARCHITECTURE.md §6.

## D-2B05 Similar-image pre-selection
- **Choice:** keeper = highest resolution by default (ties: larger file, shallower path). Non-keepers are pre-selected only when the group diameter is <= 3 bits (re-saves and resizes); wider groups are listed with nothing selected. Columns: `resolution`, `quality` (research §6 score, never cached), `similarity`.
- **Why:** perceptual matches can be false; only near-identical groups are safe defaults.

## D-2B06 Fingerprint details
- dHash/pHash follow research §4 bit for bit (box-average grid on luma, unnormalised DCT basis, upper median, bit 63 = 0, pHash 0 -> 1). JPEG luma = decoder's Y plane; PNG luma = `((299 r + 587 g + 114 b) / 1000) >> 8` on 16-bit values; alpha ignored.
- A flat image has dHash 0 and is unusable in `dhash`/`both` modes (`skipped["flat_image"]`); its pHash is noise-derived but deterministic (reference behaviour).
- EXIF orientation is not applied (reference behaviour; thumbnails share the main image's orientation).
- `ExifSummary` is `Some` when EXIF was parsed, or when the header walk reached the image data without an EXIF segment (absence really observed); `None` when the window ended first.
- Decode failures are cached (`DECODE_FAILED`) and not retried while the stamp is unchanged; `too_large`/`unsupported` are not cached (cheap header checks).
- Interlaced (Adam7) PNG is reported `unsupported` (exact-matched only).
- `UNREADABLE` is never written to the cache: `merge_from` ORs flags, so it would stick under the same stamp.

## D-2B07 Keep-rule defaults
- **Choice:** exact duplicates `oldest` (the first copy is usually the original, later copies are downloads/re-saves); similar images `highest_resolution`; version chains `newest` (the latest modified); fonts: the copy in a system font folder, else shortest path; derived files: the source document; attachment copies: the synced file. Exactly one keeper per group (`Badges::KEEPER`, never selected). `preferred_folder` with an empty folder falls back to the detector default.

## D-2B08 Version-chain rules
- **Choice:** same folder + extension family + base name after removing markers (copy numbers, copy words, v/ver/version/rev numbers, final/draft/latest/updated/revised/edited/clean/old, full dates in five layouts); initials only as the last token and only together with another marker; fuzzy = one token, one edit, >= 5 letters, no digits, both sides carrying a marker. Bare years, quarters, months, plain numbers and 6-digit dates are never markers. Review only, nothing pre-selected. Full rule table in `crates/dupes/src/version_chains.rs`.
- **Why:** consulting folders are full of semantically different files that differ by one token (Q1/Q2, 2023/2024, EU/US, Part 1/Part 2); a false chain costs a user's document, a missed chain costs only bytes.

## D-2B09 Detector parameters for roots the contract does not carry
- **Context:** `DetectContext` has no sync roots, Downloads folders or font folders.
- **Choice:** text parameters `downloads_folders`, `synced_folders` (attachment copies) and `font_folders` (fonts), one path per line (`;` also separates). Empty = name heuristics: `Downloads`; `OneDrive*`, `Dropbox`, `Google Drive`, `iCloudDrive`; any `Fonts` / `.fonts` folder. A Downloads folder inside a synced root counts as Downloads.
- **Why:** works today with `params::defaults`; the engine can fill the parameters from `ActionPlatform::sync_roots` once W2-E lands it (contract change request below).

## D-2B10 Similar images never double-report exact copies
- **Choice:** before fingerprinting, image files are run through the staged exact lane; only the first path of each exact group is fingerprinted (`skipped["exact_copy"]` for the rest).

## Contract change requests (for the orchestrator)
1. `DetectContext` (or `ParamValues` defaults filled by the engine) should carry the detected sync roots and well-known folders (Downloads, font folders), so CLN-DET-20 and OPT-FNT-01 do not rely on name heuristics (SharePoint libraries synced under a tenant-named folder are invisible to the heuristic).
2. `mj_actions::select::apply_keep_rule` passes `pixels: None`; to make `highest_resolution` work after a user re-applies a rule, candidates could carry the pixel count (W2-B fills `columns["resolution"]` = `WxH` today).
3. ARCHITECTURE.md §6 "no full-decode fallback" should mention D-2B04.
