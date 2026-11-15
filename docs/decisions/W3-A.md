# W3-A decisions (optimisation framework, ZIP, XML, OOXML, Office)

Format as `docs/DECISIONS.md`. Ids D-3Axx. Rule throughout: user data safety first
(skip over guess, verify before keeping, report over act).

## D-3A01 Framework scope inside `mj-optimise`
- `mj-actions` owns every write to a user path (replace, originals policy, light copy
  placement, timestamps/permissions, journal). `framework.rs` therefore provides the
  pieces around the pure transform: batch dry run / apply into a caller-supplied
  working directory (`run_batch`, catch_unwind per file, cancel/pause between files,
  progress per file), `MinGain` (default: at least 512 B **and** 0.5 %; the action
  engine additionally enforces "strictly smaller"), `light_copy_path` (`name_light.ext`,
  ` (n)` on collision), `optimise_item` (the `ActionItem` hand-off carrying output mode
  and originals policy, keep-if-not-smaller always true), `preserve_metadata` (for files
  this crate writes), `fit_to_target` (ladder = presets in declared order, best quality
  first; first rung under the target wins; otherwise smallest reported, `reached=false`),
  `LossyPolicy` (global + per type; lossy settings reset, lossy presets hidden; disabled
  settings always reset), content routing (`route`, `content_kind`: sniff + ZIP directory
  refinement), `SizeBreakdown` and `analyse_by_content`.
- The Office and ZIP optimisers apply `MinGain` themselves and return `skipped` below it.

## D-3A02 ZIP writer
- Entries are compressed in memory before the header is written: sizes/CRC always in
  the local header, bit 3 never set. ZIP64 only where a field does not fit (pure header
  builders are unit-tested at the thresholds; `force_zip64` produces small ZIP64 archives
  that `unzip -t` and Python `zipfile` accept).
- "Version made by" is not exposed by the reader: Unix (`3`) is inferred when the high
  16 bits of the external attributes are non-zero, DOS otherwise.
- Local and central extra fields are kept separately (read from the local header by
  `zip::write::source`); ZIP64 records are stripped and regenerated. Malformed extra
  data refuses the entry. Raw name bytes are kept from the local header; a local/central
  name mismatch refuses the archive.
- Per entry the smallest of stored, deflate level 10 and the original compressed bytes
  is written (never worse than the source).

## D-3A03 Repack refusals
- Refused (skipped with the reason): encrypted archives/entries; APKs (extension,
  `AndroidManifest.xml`+`classes.dex`, or an APK Signing Block) because v2+ signatures and
  `zipalign` cover the whole file; signed ODF/OOXML; methods other than stored/deflate;
  **any data outside the entries** (prefix, gaps beyond a data descriptor, overlaps,
  bytes before the central directory) since a repack would silently drop it.
- Signed JARs are repacked: v1 signatures cover entry contents and manifest bytes, which
  a recompression does not change.
- EPUB/ODF: a first `mimetype` entry is kept first and forced stored. Entry order is never
  changed (so `[Content_Types].xml` stays first in OOXML).
- After writing: same names in the same order, every entry decodes with the expected
  CRC-32 and size (replacements: their new CRC).

## D-3A04 Caps
- Per package: 100 000 entries, 1 GiB per inflated entry, 4 GiB total inflated (declared
  totals are checked before reading). XML: depth 256, 512 MiB per document, 4096
  attributes per element; DOCTYPE rejected, only the five entities and numeric references.

## D-3A05 OOXML verification tolerates pre-existing problems only
- The output is scanned with the same rules as the source (well-formed XML parts,
  internal relationship targets exist, content types cover every part, no orphan rels
  part); only issues **not present in the source** fail. Untouched parts must be
  CRC-identical, `[Content_Types].xml` must stay first when it was. Any failure: the output
  is deleted and the file reported "not optimised".

## D-3A06 Office safety rules
- Skipped: encrypted / RMS (CFB container), ZIP with encrypted entries, signed packages
  (`_xmlsignatures/` or a signature-origin relationship), legacy 97-2003 files (never
  written, never produced), content of another kind than the optimiser's (routing).
- `docProps/custom.xml`, `docMetadata/LabelInfo.xml` and `_xmlsignatures/*` are protected
  in the package model: replacing or removing them is an internal error.
- Custom XML removal is refused on labelled files and never removes items carrying label,
  SharePoint metadata, content-type schema, bibliography or cover-page data, items
  referenced by anything but the main part, or (Word) items bound to content controls
  (`storeItemID`). Items without a datastore id are assumed bound.
- If an optional operation fails on an unusual file, the run falls back to the safe subset
  (repack, thumbnail, dedup) and notes why, rather than skipping the file.
- Garbage collection removes only parts that were reachable before the edits and are not
  afterwards; parts already unreachable in the source are left alone.
- Operations that remove user content (notes, comments, hidden slides, custom XML,
  fonts, chart data, template reference, broken names, used-range trim) set
  `OptimiseOutput::lossy = true`, which makes the action engine require confirmation.

## D-3A07 PowerPoint
- Unused layouts (default on): layouts no slide uses; each master keeps at least one;
  a layout referenced by any part other than its master is kept. Masters (opt-in): masters
  none of whose layouts is used; one master always kept.
- Hidden slides (opt-in): removed from `sldIdLst`, custom shows and `p14` sections; refused
  when another slide links to it, and when every slide is hidden.
- Picture editing history (PPT-03) is **disabled**: dropping the kept originals of edited
  pictures needs no codec, but its rendering result could not be validated against
  PowerPoint here.

## D-3A08 Excel
- Used range (opt-in, lossy): rows after the last value that hold only empty cells are
  removed, and empty cells right of the last value column; bounds also include merges,
  hyperlinks, validations, conditional formats, filters (not sheet views), and anchors of
  drawings, tables and comments. Sheets with implicit (`r`-less) rows/cells are left alone.
  `<dimension>` is set to the bounding box of the remaining cells. Macro warning in the
  setting help, the notes and the `macros` column.
- Pivot records (opt-in): only caches whose source is a worksheet range in the workbook;
  `saveData="0"`, `refreshOnLoad="1"`, records part and relationship removed.
- XLS-04 is implemented for **named styles** (custom `cellStyle` + `cellStyleXfs` that no
  cell format uses, `xfId` remapped inside styles.xml only). Pruning `cellXfs` is disabled:
  cell format indices are referenced from every sheet, macro sheet and possibly VBA.
- CF dedup (opt-in, default off): a rule identical (ignoring `priority`) to an earlier rule
  on the same `sqref` is redundant whatever the ordering; empty blocks are removed.
- Broken names (opt-in): `#REF!` names, not `_xlnm.*`, not used as a whole identifier in
  any other XML part or name.

## Contract change requests (not made)
- `OptimiseOutput` / `Candidate` have no structured breakdown; it is a `breakdown` text
  column (`images 4.2 MB · fonts 1.1 MB`). A `breakdown: Vec<(category, bytes)>` field would
  let the UI draw the stacked bar.
- `ZipEntry` lacks `version_made_by` and the archive comment; the writer works around both.
- `OptimiseJob` carries no "skip labelled" setting; labelled files are optimised with labels
  preserved (the action engine already skips them per `Settings::safety`).
