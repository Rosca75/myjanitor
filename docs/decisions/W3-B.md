# W3-B decisions (PDF, transparent compression; archiving follows)

Format of `docs/DECISIONS.md`; ids D-3Bxx.

## D-3B01 PDF reader/writer is in-house (no `lopdf` / `pdf`)
- **Context:** §7.1 rule: an external crate only if a correct in-house implementation with thorough tests exceeds about 5 h. The pass needs an object-level reader/writer, not a renderer.
- **Choice:** in-house `mj_formats::pdf` (object model, lexer, xref tables and streams, `/Prev` chains with loop detection, hybrid `/XRefStm`, object streams, direct/indirect `/Length`, FlateDecode with PNG/TIFF predictors via `miniz_oxide`, writer with object streams + xref stream). About 2,000 lines with tests. No new dependency, so no `docs/dependencies/W3-B.md`.
- **Why:** well under the threshold for this subset; `lopdf` would bring a general parser whose recovery heuristics we would have to disable for safety, plus transitive crates.

## D-3B02 Refuse rather than repair
- **Choice:** anything not read exactly is skipped with a reason: data before `%PDF`, missing `startxref`/`%%EOF`, an xref offset not pointing at the object it names, a `/Length` not ending at `endstream`, page-tree cycles, nesting over 64, files over 512 MiB, decoded streams over 256 MiB. Encrypted (`/Encrypt` in any trailer), signed or certified (`/ByteRange` anywhere in the raw bytes, `/Type /Sig` or `/DocTimeStamp`, signed `/FT /Sig` fields, `/SigFlags` bit 1, catalog `/Perms`, `/DocMDP`) are skipped too. Dangling references read as `null` (spec) and are written as `null`.
- **Why:** user data safety first; a repaired file is a guess. On the corpus this skips 6 malformed files that viewers can open.

## D-3B03 What the lossless pass changes
- Only objects reachable from the trailer (`/Root`, `/Info`, other non-layout trailer keys) are kept; `/ID` is preserved. Objects are renumbered breadth-first with generation 0.
- FlateDecode streams are re-deflated at level 10 only when smaller and the new data inflates to identical bytes; `/DecodeParms` (predictors) are never changed. Uncompressed streams of 32 bytes or more gain FlateDecode under the same test, except XMP (`/Type /Metadata`, kept readable and PDF/A-valid), external (`/F`) and parameterised streams. Every other filter (DCT, JPX, JBIG2, CCITT, LZW, ASCII85, chains) and any stream that does not inflate cleanly is copied byte for byte.
- Byte-identical streams (dictionary without `/Length`, plus data) are merged, repeated up to 4 rounds.
- Object streams (100 objects each) and an xref stream are used, raising the header to 1.5 if lower, except for PDF/A or PDF/X files (XMP `pdfaid:`/`pdfxid:`, `GTS_PDFXVersion`, or unreadable XMP), which keep a classic table and their version.
- An output that is not smaller is reported as skipped ("Already compact") and not written.

## D-3B04 Verification is two independent checks on the bytes read back from disk
- Whole-graph equivalence between the original (with the same removals applied) and the output, walked in lock step from the trailer with a consistent reference mapping; streams compare by canonical content (decoded where we decode, raw plus filter entries otherwise).
- Targeted invariants against the untouched original: page count and leaf order, each page's content streams, fonts (`/FontFile*`) and images (`/Subtype /Image`) as decoded-content hash sets, catalog keys (only keys an enabled removal targets may go), `/ID`.
- Failure removes `dst` and reports "Not optimised: verification failed". The pass and verification both run inside `catch_unwind`.

## D-3B05 Dead-weight removals and their defaults
- Thumbnails (`/Thumb`) on by default: viewers regenerate them; not lossy.
- Off by default and marked `lossy` (moves the step to `lossy_opt_in`): XMP (`/Metadata` pointing at metadata streams, everywhere), `/Info`, attachments (`/Names /EmbeddedFiles`, catalog `/AF`), JavaScript (`/Names /JavaScript`, a JavaScript `/OpenAction`, catalog and page `/AA`), `/PieceInfo` everywhere.
- Not removed (noted in the report): file attachment annotations, JavaScript on form fields and links. PDF/A and PDF/X files only lose thumbnails.
- The weight of each removal is the stream bytes it made unreachable. The size breakdown (OPT-ENG-09) is a note `breakdown images= fonts= content= metadata= other=` on `OptimiseOutput` (encoded bytes of the input).

## D-3B06 Estimate contract
- `estimate` is cheap and always `Estimated`: files up to 4 MiB run the pass in memory (nothing written, no verification); larger files get a head/tail check (PDF, `/Encrypt`, `/ByteRange`) and a ratio model (0.90 classic layout, 0.97 when xref/object streams already exist). `Exact` is the framework's dry run (running `optimise` to a temporary `dst`).

## D-3B07 PdfLossy (OPT-PDF-01/02/04) is visible but disabled
- **Why:** image downsampling needs a JPEG/JPX encoder and font subsetting a TrueType/CFF subsetter; neither is bundled and faking them is not allowed. Settings (preset, DPI, font subsetting) are shown with the reason. Duplicate embedded font files are already merged by the lossless pass when byte-identical.

## D-3B08 Transparent compression
- Windows WOF via hand-written `DeviceIoControl` FFI; buffer layouts and FSCTL codes are pure code (`wof_layout.rs`) unit-tested on Linux. The file is opened without `FILE_FLAG_OPEN_REPARSE_POINT` (a WOF file is a reparse point and must go through the filter), after an lstat refuses symlinks. `ERROR_COMPRESSION_NOT_BENEFICIAL` is `Unsupported` ("not compressible"); revert of an uncompressed file is `Ok`. Compile-checked only.
- Btrfs: `statfs` magic detection; set the `btrfs.compression` xattr to `zstd`, then `BTRFS_IOC_DEFRAG_RANGE` with the compress flag to rewrite existing extents (the property alone affects only new writes). On defrag failure the previous property is restored. Revert removes the property and rewrites without compression. Opened with `O_NOFOLLOW`. Risk: the rewrite unshares extents shared with snapshots or reflinks (more space, no data loss). Not exercised on a real Btrfs volume in this session.
- Candidates are files (the action is per file), so the card unit is "files": not modified for 90 days (setting), at least 16 KiB, not already compressed (platform `state`), on a supporting volume (else counted `unsupported_volume` with the reason in the notes).
- OPT-FSC-02: an extension table of compressed formats, plus a probe that deflates up to 4 evenly spaced 64 KiB blocks at level 1; above 0.9, skip. The same ratio, times a per-algorithm factor, rounded to 4 KiB clusters, is the estimate.
- OPT-FSC-04: path markers (`steamapps`, `Epic Games`, `XboxGames`, `GOG Galaxy`, `GOG Games`, `WindowsApps`) give a `warning` column, sub-group `game_folder`, not selected by default, plus a diagnostics note (`Estimate` has no warning field).
- OPT-FSC-03 watcher: not built; the setting is shown disabled with a reason.
- `optimise` returns `InvalidInput`: the work is done by `Action::FsCompress` through `ActionPlatform`.

## D-3B09 Shared deflate helper lives in `mj_formats::pdf::filter`
- `mj-optimise` has no `miniz_oxide` dependency and W3-B owns no `Cargo.toml`, so the probe uses `mj_formats::pdf::filter::deflate_raw`. A neutral `mj_formats::deflate` module would be cleaner (orchestrator request).

## D-3B10 Archiving follows the create-then-verify contract; nothing is left out silently
- **Context:** `mj-actions` runs `Archiver::create`, then `Archiver::verify`, and removes the sources only when `verify` returns `Ok` (it deletes the archive otherwise). The contract has no way to report skipped files.
- **Choice:** `create` fails the whole archive for anything it cannot store: a file over 512 MB in a native ZIP, a socket/pipe/device, a non-UTF-8 name or one containing `\`, a file that changes between hashing and writing. Symlinks and Windows junctions are never followed or stored; they appear in `INDEX.csv` as `symlink not archived -> <target>` (`skipped_links()` lists them for reports). Cancel is checked between files (and between 1 MiB chunks in tar); a cancelled or failed `create` removes the partial archive, the partial tar and the sidecar.
- **Why:** sources are removed after a verified archive, so an omission would be data loss.

## D-3B11 Integrity test (OPT-ARC-02)
- `verify` walks the sources again and hashes every file as it is now. ZIP: re-opened with the in-house reader (CRC-32 checked on read); each file has the same size and SHA-256; each folder entry exists; the index is the first entry and equals the index rebuilt from the sources; there are no extra or duplicate entries; links are absent. tar.zst: `zstd -d` to a scratch tar, read with the in-house tar reader, with the same checks. 7z: `7z t`, the index extracted with `7z e -so` and compared, then `7z l -slt` CRC-32 and size per file. There is no option to skip it. A failed verification removes our sidecar (only when its bytes are our index).
- `verify` receives no tool list in the contract, so it re-runs tool detection without settings overrides (contract gap).

## D-3B12 Formats and levels
- ZIP is in-house (W3-A writer). Each entry is compressed in memory, hence the 512 MB per-file cap. The level is 0 stored, 1-8 deflate (stored if not smaller), 9 = miniz level 10. Times are stored as MS-DOS fields (UTC) plus an extended-timestamp extra field. Unix modes are stored. ZIP64 is applied by the writer when needed.
- tar.zst uses an in-house ustar writer with pax `path`/`size` records, streamed with no size cap, and external `zstd` at level `1 + 2 x level` (1-19). GNU `tar -x` extracts it identically (tested).
- 7z: two `7z a` runs (the index from a scratch folder, then the files from a UTF-8 list file with `-snl`, `-ms=on`, `-mx=level`). 7z is never given a folder to recurse, so it cannot follow a link. Empty folders are therefore not stored in 7z (the index lists them).
- 7z and zstd were not installed on the build machine. The argument builders and the `-slt` parser are unit-tested; the integration test runs only when the tools are found.

## D-3B13 Index (OPT-ARC-04)
- `INDEX.csv` (`path,size,mtime,sha256,note`, RFC 4180 quoting) is the first entry of every archive (`INDEX (MyJanitor).csv` if a source is named `INDEX.csv`). A sidecar `<archive>.index.csv` is written beside the archive, never over an existing file. The journal records only the archive (`created = [dst]`), so undo leaves the sidecar behind (contract change requested).

## D-3B14 Encryption and split volumes are disabled (OPT-ARC-05)
- `ArchiveJob` has no password or volume-size field. 7z accepts a password only through `-p<password>` on its command line, which other processes can read. Both toggles are shown disabled with the reason. Native ZIP offers neither.

## D-3B15 Cold folders and estimates
- Cold = nothing modified for `min_age_months` (12). Access times are ignored (relatime/noatime, disabled on Windows). Only the topmost cold folder of a subtree is proposed, at least `min_size_mib` (10 MiB). Folders holding online-only placeholders are skipped (archiving would download them). This is computed from the index, because `mj-optimise` cannot depend on `mj-detect`.
- Estimate: files of compressed types count at full size; up to 12 other files are probed (four 64 KiB blocks, deflate level 1). A sample at or above 0.95 counts as stored; otherwise it is scaled by 0.9 for the best level. Add 120 bytes per entry. Always `Estimated`.

## D-3B16 Recompression (OPT-ARC-03)
- `.zip` files whose content is a plain ZIP (not Office, ODF, EPUB or JAR) go through `zip_repack::repack_file`, followed by our own check: the same entry names in order and the same SHA-256 per entry. Encrypted and signed (`META-INF/*.SF|*.RSA`) archives are skipped, and so are results that are not smaller (nothing written). ZIP to 7z is disabled: an `Optimise` result replaces the original under its own name, so a 7z file would be named `.zip`.
