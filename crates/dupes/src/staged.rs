//! Staged content hashing shared by the exact-duplicate lane (W2-B).
//!
//! Contract (ARCHITECTURE.md §6): bucket **all** files by size (cache hits
//! included, D-006); partial hash = xxh64 over head + middle + tail windows of
//! `PARTIAL_WINDOW` bytes (whole file when `size <= PARTIAL_FULL_THRESHOLD`);
//! full hash streamed in 1 MiB buffers; every stage read through and written
//! to the `FingerprintStore`; every failed read recorded in `Diagnostics`.
//! Owner: W2-B.
