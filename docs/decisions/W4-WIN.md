# W4-WIN: making the test suite pass on Windows

The first Windows CI runs (job `test (windows-latest)`) compiled and linted clean but
failed at run time. This file records each root cause, whether it was a product bug or a
test portability problem, and the rule chosen. No guard, protected-path, allowlist, link
or confirmation check was weakened.

## D-W1 Verbatim paths are normalised before the `\\?\` prefix (product bug)

`ffi::windows::wide_path` added `\\?\` to the raw path. Under that prefix the Win32
normaliser is skipped: `/` is not a separator and `.`/`..` are names, so any path built
with a forward slash (`data_dir.join("state/data")`, a path typed in the UI, a test's
`t.join("in/b.txt")`) failed with `ERROR_INVALID_NAME` (123), or could in principle
name another file. `ffi::winpath::verbatim` (pure, tested on every platform) now does
what the normaliser would have done to a drive-absolute or UNC path (`/` to `\`, repeated
separators collapsed, `.` dropped, `..` resolved but never above the drive or share) and
then adds the prefix. Trailing dots and spaces are kept, because they name the file that
really has that name; the guard refuses such names below the root (F-19). The Recycle
Bin route uses the same normalised form before stripping the prefix for the shell.

## D-W2 The F-19 short-name rule applies only below the allowed root

`%TEMP%` on the CI runner is `C:\Users\RUNNER~1\AppData\Local\Temp`: a short name above
every test root. The guard already checks only the components of the path below the
allowlisted root (`path.strip_prefix(root)`), and that is the rule kept: an ancestor at
or above the root is the location the user picked, while a short name or a trailing
dot/space below it is refused. The containment and protection checks do not rely on the
spelling above the root alone: `check_canonical` compares `canonicalize` results, which
expand short names, so a lexical match through a short spelling is checked again in long
form. A Windows unit test pins both halves (`RUNNER~1` in the root is fine, `DOCUME~1`
below it is refused).

## D-W3 MFT link counts exclude DOS short-name aliases (product bug)

The NTFS FILE record header's "hard link count" counts directory entries, including the
separate DOS 8.3 alias that a non-8.3 name such as `setup-1.0.msi` gets. The MFT fast
path reported such a file as having two links, so the removal estimate assumed the
space stays held by "the other link" and promised nothing. `parse_file_record` now
counts the `$FILE_NAME` attributes whose namespace is not DOS (2); with an attribute
list (names possibly in other records) the header count stands, which can only
under-promise.

## D-W4 7-Zip listings with CRLF (product bug)

`parse_slt` split entries on `"\n\n"`; 7-Zip for Windows writes CRLF, so every entry ran
into one block and archive verification reported files missing. CRLF is normalised first.

## D-W5 The instance lock is closed before it is removed (product bug)

On Windows the lock file is open without sharing, so `remove_file` in `Drop` failed while
the handle was still open and the file stayed behind. The handle is now dropped first.

## D-W6 Test portability

- Pure path tests used Unix spellings (`/home/m`, `/etc`) that are drive-relative on
  Windows: `mj_core::testutil::abs` maps them to `C:\...` on Windows.
- `engine::paths::resolve(Os::Linux, ..)` judged XDG values with the host's
  `is_absolute`; it now uses the Unix rule (leading `/`) so it is a pure function of `os`.
- Windows has no freedesktop Trash and tests never use the real Recycle Bin: the actions
  harness uses a fixture trash of the same shape (`files/`, `info/`) on Windows.
- Folder links: `testutil::dir_links` makes a symlink on Unix and, on Windows, a
  directory symlink when privileged (skipped cleanly on `ERROR_PRIVILEGE_NOT_HELD`) plus
  always a junction (`mklink /J`), so the guard's link refusals run on Windows too.
- The system `tar` of Git for Windows reads `C:\...` as `host:path`: the test runs it
  with relative names from inside the fixture.
- Fixture JSON escapes backslashes; relative paths compared as strings use `/`.
- The scan's fast index (MFT) is legitimately used on an elevated Windows runner, and a
  whole-volume read can outlast the 2 s cache trust window: the tests assert the Linux
  expectation only on Linux, and re-write the "fresh" file right before the second scan.
