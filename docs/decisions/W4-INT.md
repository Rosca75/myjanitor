# W4-INT decisions (Wave 4 integration pass)

Format of `docs/DECISIONS.md`; ids D-4Ixx. Rule applied throughout: user data safety
first (reversible over irreversible, skip over guess, report over act).

## D-4I01 Optimise path: what was already wired, what was broken at the seams
- **Context:** the engine facade already injected `mj_optimise::optimisers()` and
  `archiver()` into `mj_actions::Services`; no test drove a real optimiser through
  scan → card → plan → dry run → apply → undo.
- **Found and fixed:**
  1. A step's parameters never reached the optimiser when they were given to
     `plan_add_step` / `plan_update_step` (only `step_params_set` copied them into
     `Action::Optimise::params`), so a chosen preset (e.g. audio `speech`) silently ran
     with the card defaults, and a lossy preset did not make the step `lossy_opt_in`.
     The step's params are now the single source of truth: values given inside the
     action, then the request's `params`, are merged into the step and copied into the
     action; `lossy` is recomputed from the params (`steps::params_lossy`).
  2. A conversion (WebP, Opus, MP4) replaced `x.wav` with Opus bytes under the old
     name (the type travelled only in a note). The action engine now names the result
     `stem.<new ext>` (unique), in replace, delete and light-copy modes; undo already
     handled an output path different from the original.
  3. The dry run compared the optimiser output (file length) with the *allocated* size
     of the original, so a file a few bytes larger after optimisation could be promised
     as a saving and then refused at apply. Both sides now use file lengths.
- **Tests:** `crates/engine/tests/optimise_e2e.rs` (PNG, JPEG, EPUB, PPTX with duplicate
  media + thumbnail, PDF; replace + undo byte-exact, light copy + undo, keep-if-not-smaller,
  labelled deck locked, lossy gate with real ffmpeg).

## D-4I02 Additive `OptimiseOutput` fields
- `output_extension: Option<String>`, `breakdown: Vec<SizePart>` (OPT-ENG-09),
  `privacy: bool`; the struct now derives `Default` and every literal uses
  `..OptimiseOutput::default()` so further additions break nobody.
- The legacy `output_extension=<ext>` note is still read
  (`OptimiseOutput::converted_extension`), so an optimiser written against the old
  contract (e.g. the concurrent `archive.rs`) keeps working. Only short ASCII
  alphanumeric extensions are accepted (never a path fragment).
- `privacy`: a GPS strip that zeroes bytes in place keeps its size; keep-if-not-smaller
  keeps such an output only when it is exactly the same size, never larger
  (`mj_actions::engine::keeps_output`). The JPEG optimiser sets it for `strip_gps`.
  Known gap: the JPEG *analysis* still lists only files whose estimate shows a saving,
  so a GPS-only privacy run is reachable from a step whose params are `strip_gps` on a
  file that is otherwise listed, not as a card of its own.
- Office optimisers fill `breakdown` and the review columns `part:<name>` = bytes
  (the UI's size-parts bar, D2-06); PDF fills `breakdown` from its report line.

## D-4I03 Estimate warnings travel per candidate, not on `Estimate`
- **Context:** `Estimate` is `Copy` and built literally in ~25 places, one of them in a
  file another agent is editing right now; a `Vec<String>` field would break `Copy` and
  every literal.
- **Choice:** compatibility warnings (OPT-FSC-04 game libraries / DirectStorage) stay in
  the reserved candidate column `warning`; the engine aggregates them: plan checklist item
  `compatibility_warnings` (level warning) and one line per distinct warning in
  `DryRunStep.warnings`.

## D-4I04 Other additive contract follow-ups
- `gifsicle` is a known tool id (detected and listed); there is still no GIF optimiser
  card (an `OptimiserId` variant is a contract enumeration the UI keys on; no verified
  GIF pass exists), so only the argument builder ships.
- `Settings.general.kept_languages` (normalised: trimmed, lower case, `_`→`-`, deduped;
  validated as `ll[-region]`): fills the language-pack detector's `kept_languages`
  parameter when the step leaves it empty. Empty = system language as before.
- `AppInfo.capabilities`: static per build and OS (`recycle_bin`/`freedesktop_trash`,
  `wof_compression`/`btrfs_compression`, `cloud_dehydrate`, `reflinks`, `hardlinks`,
  plus `context_menu` and `os_schedules` outside portable mode). Volume features stay
  per drive in `drives_list`. `ui/js/lib/platform.js` labels "Windows only" from it
  (platform name only as a fallback).
- Restore pre-check: `HistoryRunDetail.occupied` lists the page's entries whose original
  place is taken now (removals, moves, conversions; never in-place replacements, whose
  place holds MyJanitor's own output). The History view disables Restore for them and
  says why; the restore itself still re-checks and refuses (`conflict`).
- Not done: report paging (`RunRequest` is `deny_unknown_fields`, so paging needs a new
  request type and UI paging; reports are bounded by run size today); Btrfs zlib/lzo
  reporting (a new `FsCompressAlgo` variant the UI and journal key on; not trivial).
