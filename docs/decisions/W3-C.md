# W3-C decisions (images, media, external tools)

Format of `docs/DECISIONS.md`; ids D-3Cxx.

- **D-3C01 Tools**: only `.exe`/`.com` count on Windows (never `.bat`/`.cmd`: argument re-parsing); relative PATH entries are skipped; `7z` also matches `7zz`/`7za`; versions are probed in parallel with a 3 s timeout and cached by path, size and mtime; `tools_refresh` clears the cache. `gifsicle` is not in `KNOWN_TOOLS` and no optimiser id exists for GIF, so only an argument builder (`images::adapters::gifsicle_args`) ships.
- **D-3C02 Runner**: own process group on Unix, `kill -KILL -pgid` / `taskkill /T` best effort on timeout or cancel; stderr keeps the tail, stdout the head; `CREATE_NO_WINDOW` on Windows.
- **D-3C03 PNG**: re-deflate at miniz level 10, optional adaptive row filters, original stream kept when already smaller; verified by inflated-stream equality (or unfiltered equality); oxipng output accepted only if `decode_rgba16` equals the source pixels. `strip_all` keeps tRNS and render-critical chunks; removing colour chunks needs `strip_colour` and marks the output lossy.
- **D-3C04 JPEG**: only headers before the first SOS are rewritten, scan bytes copied verbatim and verified; default metadata policy `strip_thumbnail`; comments are dropped by every policy except keep_all; orientation survives `strip_all` as a 26-byte EXIF; MPF files are skipped; XMP packets mentioning GPS are dropped by `strip_gps`; trailing non-zero data (motion photos) is kept. jpegtran output is accepted only when dimensions and kept APP/COM segments match.
- **D-3C05 GPS strip does not shrink** when the GPS IFD is not at the tail (bytes are zeroed in place). The optimiser still writes an equal-size changed file for `strip_gps`; the framework's keep-if-not-smaller would drop it (see change requests).
- **D-3C06 Conversions** write to `temp_dir` with the right extension and report `output_extension=<ext>` in `notes`. Resize works with WebP only (`cwebp -resize`); AVIF and JXL refuse it (no verified resize flag).
- **D-3C07 Audio**: `silenceremove` with `stop_silence=0` (ffmpeg passes `stop_duration` through itself); FLAC from float PCM is marked lossy; OPT-AUD-08 disabled ("Requires a local transcription tool").
- **D-3C08 Video**: `quality` maps to a CRF per codec; hardware encoders are used only when asked and trial-encoded first, falling back to software; screen-content flags are used only after a trial encode succeeds; container is MP4 unless MKV is needed. Slide-PDF output (OPT-SCR-05) is not offered (no PDF writer).
- **D-3C09 Estimates** need no tool: bitrate arithmetic over in-house metadata, always `Estimated`.

## Contract change requests
1. `OptimiseOutput` needs an `output_extension` (conversions change the file type); today it travels in `notes`.
2. `KNOWN_TOOLS` lacks `gifsicle`; add it plus an optimiser id for GIF.
3. Privacy rewrites (GPS strip) need an exemption from keep-if-not-smaller.
