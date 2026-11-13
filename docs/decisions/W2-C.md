# W2-C decisions: filters, rule engine, presets, quick query, sniffer

Format as in `docs/DECISIONS.md`. Ids D-2Cnn.

## Quick-query grammar (CLN-ENG-05)

Implemented in `crates/filters/src/query/` (lexer, parser, values, units, suggest);
the doc comment of `query/mod.rs` is the reference.

```text
query := or                       or  := and ("OR" and)*
and   := unary (["AND"] unary)*   unary := "-" unary | "NOT" unary | "(" or ")" | term
term  := key ":" [op] item ("," item)*  |  item          (bare item = file name)
op    := < <= > >= =              item := word | "text" | 'text' | /regex/
key   := ext name path size modified created accessed older newer type attr is kind
         owner depth pathlen content regex pathregex top
```

Terms: `ext:jpg,png` · `name:*.tmp` (wildcard if `*`/`?`, else contains, `=x` exact, `/re/` regex) ·
`path:` (same; a wildcard without a separator is "anywhere in the path": `*Photos*` becomes `**Photos**`) ·
`size:>1GB`, `<=500k`, `1MB..10MB`, `=0` (zero-byte) · `modified:<2024` (before 2024-01-01), `<=2024`
(before 2025-01-01), `>2024` (from 2025), `2024`/`2024-03`/`2024-03-15` (within), `a..b` ·
relative values are **ages**: `modified:>2y` older than 2 years, `<30d` newer than 30 days, bare `30d`
within the last 30 days (`d w m y`, `m` = months) · `older:90d` / `newer:7d` shorthands for modified ·
`type:video` · `attr:hidden`, `attr:!hidden` · `is:empty|hidden|invalid|mismatch|file|dir|<attribute>` ·
`kind:` `owner:` `depth:>3` `pathlen:>240` `content:text` `top:N`.
Free words: name contains. Upper-case `OR`/`AND`/`NOT` only. Errors: byte `offset`/`len`, several
bad terms are reported together, syntax errors stop the parse. Size units binary (KB = 1024).

## Decisions

- **D-2C01 Three-valued evaluation.** A condition that could not be evaluated (unreadable header,
  document with no extractor, content search truncated by the byte cap, online-only file) is *unknown*;
  `NOT unknown` is unknown; only a definite true selects. Otherwise "files not containing X" would list
  PDFs that were never searched (same rule as matching invariant 9.5 in the reference project). Reported
  in `FilterDiagnostics` (`to_core()` maps to `Diagnostics`).
- **D-2C02 Cheap before expensive.** Children of `all`/`any` are stably sorted by cost: metadata, then
  extension, then name/path text (exact < contains < glob < regex), then header read (mismatch), then
  content read. Test: a content condition listed first reads only the files that passed the cheap tests.
- **D-2C03 Scope of type conditions.** Extension, type group, zero-byte, mismatch and content apply to
  regular files only (a folder named `x.mp4` is not a video). Content and header reads skip online-only
  files (reading would download them) and count them.
- **D-2C04 `LargestN`** is lifted out of the top-level `all` (or the lone top-level condition) and applied
  to the matches, then `topmost_only` is applied. Several limiters: the smallest wins. Anywhere else it is
  `InvalidInput`. In queries `top:N` is only valid under top-level ANDs (nested ANDs are flattened first).
- **D-2C05 Additive API** next to the frozen one: `compile_with`, `select_with(SelectOptions{topmost_only,
  cancel, exclusions, content})` returning `Selection{positions, diagnostics}`, `parse_with_groups`,
  `topmost`, `Exclusions`, `presets::{all,to_json,from_json,validate,upsert,remove}`,
  `groups::{builtin,effective,find}`, `media`. `FilterContext` is unchanged.
- **D-2C06 Exclusions** (`Exclusions::from_entries(&Settings::safety::exclusions)`): an absolute entry
  without wildcards is a path (whole subtree); anything else is a glob (full path if it has a separator,
  else the file name, so `*.sys` excludes by type). Excluded folders exclude their subtree. Applied
  before the rule and before `LargestN`.
- **D-2C07 Topmost** uses subtree ranges from `FileIndex::subtree`, kept as a list of active ranges
  because `a.d` and `a.txt` sort between `a` and `a/x` (`.` < `/`), so one running range is wrong.
- **D-2C08 Date safety.** `Timestamp` is i64 ns (1677-2262) and core's `add_months` multiplies unchecked.
  `timeutil::span_before` / `check_year` bound spans and absolute years to 1700-2250 before any core
  helper runs (`modified:9999-12` would otherwise panic in debug).
- **D-2C09 Regex limits:** pattern at most 1024 chars, compiled size 1 MiB, nest limit 64; patterns are
  empty-checked (an empty contains/regex would match everything). Paths are matched with `/` on every
  platform. Rule trees deeper than 64 or with over 20 000 nodes are refused.
- **D-2C10 Content search:** at most 8 MiB per file (`ContentConfig::max_bytes`), UTF-8/BOM, UTF-16
  LE/BE (BOM or NUL pattern) and legacy 8-bit text; binary skipped. PDF/Word/Excel/PowerPoint/ODF go
  through `TextExtractor`; with none registered they are `Unavailable`. Evaluation is serial (see gaps).
- **D-2C11 Mismatch:** unrecognised content under a checked extension counts as a mismatch (an HTML page
  named `.jpg`), except `.mp3`; empty files and unchecked extensions are never mismatches; a plain ZIP
  satisfies `.docx/.xlsx/.pptx/.odt/.epub/.jar` (cannot tell), a *different* specific kind does not.
- **D-2C12 Sniffer:** `FileKind` gained Docx, Xlsx, Pptx, Odf, Epub, Jar, Tar, Aiff, Avi, Sqlite, Iso,
  Ttf, Otf, Woff, Woff2 (existing variants kept). BMP, MP3 frames, gzip, bzip2, TrueType were tightened
  beyond the 2-byte magics. ISO needs 0x9006 bytes (`sniff_len_for_ext`); `check_extension` returns
  `Unchecked` if the header is shorter.
- **D-2C13 Media metadata (CLN-CNT-03)** is a separate serialisable `MediaCondition` list matched against
  a caller-supplied `MediaMeta` as a second pass (`filter_by_media`), because the frozen `Condition`
  enum has no media variant. Missing fields never match; files with no record are counted.
- **D-2C14 Empty query** is an error (not "match everything") so an empty search box cannot select the
  whole index by accident. `serde_json` (already pinned) is a normal dependency for preset JSON.
- **D-2C15 Built-in presets** keep the three Wave 1 ids and add dev_junk, large_videos, old_installers,
  temp_and_backup_files, zero_byte_files, extension_mismatch, invalid_windows_names, long_paths,
  largest_100_files, hidden_system_files. `upsert` ignores the `builtin` flag of input and refuses built-in ids.
