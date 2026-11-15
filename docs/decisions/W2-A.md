# W2-A decisions (scan engine, scan cache, platform scanning)

Format of `docs/DECISIONS.md`: *context → options → choice → why*.

## D-2A01 Parallel walk shape
- **Context:** the walk must be parallel on the in-house pool (no rayon); `mj_core::pool::parallel_map` takes a fixed item list, a tree walk is dynamic.
- **Options:** (a) a work queue with condvars on `ThreadPool`, (b) level-synchronous BFS on `parallel_map`.
- **Choice:** (b), two parallel phases per level: list every directory of the level, then stat every entry of the level as one flattened list. Phases under 64 items run inline.
- **Why:** reuses the audited pool (cancel/pause checkpoint per item, `catch_unwind`, index-aligned slots, no per-item lock); the expensive stat phase is balanced even when one folder holds 100 000 files. Measured: 200 000 files / 421 folders walked in 0.52 s (4 workers, release, warm page cache).

## D-2A02 `statx` through `syscall(2)`
- **Context:** glibc only has a `statx` wrapper from 2.28; linking a missing symbol stops the binary from starting.
- **Choice:** `extern "C" fn syscall` with `SYS_statx` (332 on x86_64, 291 on aarch64), a 256-byte aligned buffer decoded by a pure `parse_statx` (unit-tested with hand-built buffers); `ENOSYS`/`EPERM` (old kernel, seccomp) switch the process to `std::fs::symlink_metadata` for good. `st_dev` is rebuilt with glibc's `makedev` so ids match the std fallback. `AT_NO_AUTOMOUNT` is set so a scan never mounts anything.
- **Why:** works on every glibc; no struct layout in FFI to get wrong.

## D-2A03 Windows metadata without opening placeholders
- **Choice:** `GetFileAttributesExW` (no handle) → reparse tag via `FindFirstFileExW` when the reparse bit is set → only name surrogates (symlink, junction, mount point) are `Link`; a reparse point whose tag cannot be read is also `Link` (never descend into the unknown); cloud tags (`0x9000x01A`) stay files/folders and get `CLOUD_LOCAL` unless online-only; WOF tag → `WOF_COMPRESSED`. For local entries a `FILE_READ_ATTRIBUTES` handle gives file id (volume serial + file index) and link count; online-only entries are never opened. Allocated size: `GetCompressedFileSizeW` for compressed/sparse/WOF files, otherwise the size rounded up to the volume cluster (cached per volume), 0 for online-only. Sharing violations fall back to the directory entry data.
- **Why:** opening an online-only placeholder can recall it; file ids are needed so hard links are never reported as duplicates.
- **Status:** compile-checked for `x86_64-pc-windows-msvc` and reviewed against the SDK layouts (sizes asserted at compile time); not run.

## D-2A04 MFT fast path = USN enumeration + FILE records
- **Context:** `FSCTL_ENUM_USN_DATA` yields names, parents and attributes but no sizes or times.
- **Choice:** enumerate USN records, keep those below the root (BFS from the root's file reference, never entering name surrogates; a reparse folder is entered only when its tag is read and is not a surrogate), then read each entry's FILE record with `FSCTL_GET_NTFS_FILE_RECORD` (sizes, `$STANDARD_INFORMATION` times and attributes, reparse tag, link count). Records the in-place parser cannot answer (attribute lists, extension records) fall back to a per-path stat. The walker applies exclusions, hidden, depth and one-file-system to the result with the same `admit` function as the walk. Any error (no admin rights, not NTFS, failure midway) → the result is discarded and the root is walked.
- **Why:** no per-file open; parsing is pure and tested on Linux with hand-built V2/V3 USN buffers and FILE records (fixups, resident/non-resident/compressed `$DATA`, torn records, malformed chains).
- **Known gap:** a file with several hard links appears once (first name) on the MFT path; reading the raw `$MFT` in bulk would be faster still.

## D-2A05 The cache records every file a scan sees
- **Choice:** `scan_with_cache` writes a stamp-only record (size, mtime, file id) for each local file; fingerprint stages stay `Option`s behind `Option<Box<Fingerprints>>` (~110 bytes per entry on disk at 200k files, 22 MB). Presence bit 9 (file id) was added to the format layout before anything shipped, so `FORMAT_VERSION` stays 1.
- **Why:** hit/miss counts ("Scanned · cached"), pruning and the file-id carry-over need to know every file; D-106's size estimate assumes it.

## D-2A06 Trust window anchored at the scan start
- **Choice:** a stamp whose mtime is within 2 s before the scan start, or in the future, never hits, and `put` never stores fingerprints for it (the stamp is still recorded). The anchor is set by `observe`; before any scan it is "now".
- **Why:** a file still being written, or rewritten within a coarse mtime granule (FAT 2 s), must not keep stale hashes.

## D-2A07 Pruning rules
- **Choice:** `prune(index)` uses the coverage `observe` recorded for that same index: under each walked root, records the scan did not see are dropped (deleted, moved, excluded, hidden-skipped) except below directories that could not be read; everything under a vanished root is dropped; unreadable and excluded roots are kept; scans with include patterns or a depth limit prune nothing but vanished roots. Without a matching record (an index from plain `scan`) only vanished roots are pruned.
- **Why:** D-006 pruning without letting a transient permission or network error throw the cache away.

## D-2A08 File ids carry fingerprints
- **Choice:** on a miss whose `(file id, size, mtime)` equals a fingerprinted record, the fingerprints are copied to the new key.
- **Why:** same id and stamp = same content (rename or new hard link); avoids re-hashing moved files.

## D-2A09 What is not indexed
- **Choice:** other filesystems below a root (one-file-system) and directories reached a second time by file id (bind-mount loops) are counted in `excluded` and not recorded at all; an entry that vanishes between listing and stat is skipped (it no longer exists; not "unreadable"); online-only files count in `files` and `online_only_*` but not in `bytes`/`allocated`, and get no cache record.
- **Why:** an empty-looking mount point must never become an "empty folder" removal candidate; SCAN-09.

## D-2A10 Exclusion pattern semantics
- **Choice:** absolute literal path → that entry and everything below; pattern without a separator → name match at any depth (gitignore style); relative pattern with a separator → anchored as `**/pattern`; absolute glob → whole path. Excluded folders are not entered. A root inside an exclusion is skipped.
- **Why:** settings exclusions like `node_modules` or `build/out` previously could never match (they were compared as absolute paths).

## D-2A11 Roots that are not folders
- **Choice:** a file root is indexed as that file; a link root is recorded as `Link` and not followed.
- **Why:** links are never followed, not even at the top.

## D-2A12 Folder tree parents by path lookup
- **Context:** in `path_key` order `/r/a-b` sorts between `/r/a` and `/r/a/x`, so a stack over the sorted entries mis-parents.
- **Choice:** `FolderTree::build` maps folder paths to positions and attaches each entry to its nearest indexed ancestor; one reverse pass accumulates totals (ancestors always sort before descendants). 200k entries: 49 ms.

## D-2A13 Linux volumes
- **Choice:** `/proc/self/mountinfo` (octal escapes decoded), pseudo filesystems and system mount points (`/proc`, `/sys`, `/dev`, `/run` except `/run/media`, `/snap`, `/boot/efi`) hidden, one entry per device (shortest mount point), `statvfs` via FFI, removable from sysfs or `/media`, labels from `/dev/disk/by-label`.

## D-2A14 Owner on Windows
- **Choice:** not collected yet (`owner: None`); Linux reports `uid:N` from `statx` for free.
- **Why:** a SID lookup per file needs `GetSecurityInfo` + `ConvertSidToStringSidW`; deferred, `collect_owner` is currently ignored on Windows.
