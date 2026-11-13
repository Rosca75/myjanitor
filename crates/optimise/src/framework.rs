//! Optimisation framework helpers (W3-A): estimate models per format,
//! fit-to-target-size search (OPT-ENG-08), per-file size breakdown
//! (OPT-ENG-09), lossless/lossy switch handling (OPT-ENG-01). The safe
//! replace / keep-if-not-smaller / light-copy / backup policy itself is
//! implemented in `mj-actions` (it owns every write to user paths).
