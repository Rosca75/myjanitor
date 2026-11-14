//! Excel optimisation (OPT-XLS-01..10, SAF-08).
//! Owner: W3-A.
//!
//! On by default: repack, thumbnail, media dedup (lossless). Opt-in: reset the
//! inflated used range (XLS-02, with the macro warning XLS-10), drop pivot cache
//! records (XLS-03, Excel rebuilds them on refresh), remove unused *named* cell
//! styles (XLS-04, partial: styles.xml-only remap), remove exact duplicate
//! conditional-formatting rules (XLS-05), remove broken defined names nothing
//! uses (XLS-06). Reported only: bloat breakdown (XLS-01), external links and
//! broken names (XLS-06), hidden sheets (XLS-07). Disabled with reason: picture
//! compression (XLS-08), .xlsb conversion (XLS-09), unused cell-format pruning.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};
use mj_formats::ooxml::tree::Tree;
use mj_formats::sniff::FileKind;
use mj_formats::xml::{self, Edits, TokenKind, Tokenizer};

use super::package::{self as pk, disabled, toggle, OpReport, Pkg};
use crate::framework;

/// Excel workbooks: Trim bloated used ranges, styles and pivot caches
#[derive(Debug, Default, Clone, Copy)]
pub struct Excel;

/// Extensions (lower case) this optimiser targets.
pub const EXCEL_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xltx", "xltm"];

pub const P_USED_RANGE: &str = "reset_used_range";
pub const P_PIVOT: &str = "drop_pivot_records";
pub const P_STYLES: &str = "remove_unused_styles";
pub const P_CF: &str = "dedupe_conditional_formats";
pub const P_NAMES: &str = "remove_broken_names";

fn settings() -> Vec<mj_core::SettingSpec> {
    let mut v = pk::common_settings();
    v.extend([
        toggle(
            P_USED_RANGE,
            "Reset inflated used ranges",
            "Removes empty formatted rows and cells after the last value. Macros that rely on UsedRange may behave differently (OPT-XLS-02/10)",
            false,
            true,
        ),
        toggle(P_PIVOT, "Stop saving pivot cache data", "Pivot tables refresh from their source when the file is opened (OPT-XLS-03)", false, false),
        toggle(P_STYLES, "Remove unused cell styles", "Named styles no cell format uses (often imported from templates) (OPT-XLS-04)", false, false),
        toggle(P_CF, "Remove duplicate conditional formatting", "Exact duplicate rules on the same range (OPT-XLS-05)", false, false),
        toggle(P_NAMES, "Remove broken named ranges", "Names that evaluate to #REF! and are used nowhere (OPT-XLS-06)", false, true),
        disabled("compress_pictures", "Compress embedded pictures", pk::NEEDS_CODEC),
        disabled("convert_xlsb", "Convert to .xlsb", "MyJanitor does not write the binary workbook format; converting needs Excel"),
        disabled("prune_cell_formats", "Remove unused cell formats", "Cell formats may be referenced by parts this tool does not rewrite; unused named styles are removed instead"),
        disabled("narrow_whole_column_cf", "Narrow whole-column conditional formatting", "Needs formula-aware range rewriting, not available yet"),
    ]);
    v
}

// ---------------------------------------------------------------- cell references

/// `AB12` / `$AB$12` / `AB` / `12` -> (column 1-based or 0, row 1-based or 0).
pub fn parse_ref(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().rsplit('!').next()?.replace('$', "");
    let letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &s[letters.len()..];
    if letters.len() > 3 || (letters.is_empty() && digits.is_empty()) || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut col = 0u32;
    for c in letters.chars() {
        col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    let row = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    Some((col, row))
}

fn col_name(mut c: u32) -> String {
    let mut s = Vec::new();
    while c > 0 {
        let r = (c - 1) % 26;
        s.push(b'A' + r as u8);
        c = (c - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// Largest (column, row) a space-separated list of references / ranges touches.
fn max_of_refs(list: &str) -> (u32, u32) {
    let mut m = (0, 0);
    for r in list.split_whitespace() {
        for end in r.split(':') {
            if let Some((c, w)) = parse_ref(end) {
                m.0 = m.0.max(c);
                m.1 = m.1.max(w);
            }
        }
    }
    m
}

// ---------------------------------------------------------------- used range (XLS-02)

#[derive(Debug)]
struct Cell {
    span: Range<usize>,
    col: u32,
    row: u32,
    value: bool,
}

#[derive(Debug)]
struct Row {
    span: Range<usize>,
    row: u32,
    cells: Vec<Cell>,
    /// Only `c` children.
    plain: bool,
}

#[derive(Debug, Default)]
struct SheetScan {
    rows: Vec<Row>,
    dimension: Option<Range<usize>>,
    /// Bounds from merges, hyperlinks, validations, conditional formats, filters.
    anchored: (u32, u32),
    /// Elements directly under the root: (local name, span).
    top: Vec<(String, Range<usize>)>,
    implicit_refs: bool,
}

fn scan_sheet(text: &str) -> Result<SheetScan> {
    let mut t = Tokenizer::new(text)?;
    let mut s = SheetScan::default();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut row: Option<Row> = None;
    let mut cell: Option<Cell> = None;
    let mut in_views = false;
    while let Some(tok) = t.next_token()? {
        match tok.kind {
            TokenKind::StartTag | TokenKind::EmptyTag => {
                let local = tok.local_name().to_string();
                let empty = tok.kind == TokenKind::EmptyTag;
                let parent = stack.last().map(|x| x.0.as_str()).unwrap_or("");
                if tok.depth == 2 {
                    in_views = local == "sheetViews";
                    if empty {
                        s.top.push((local.clone(), tok.span.clone()));
                    }
                }
                if local == "dimension" && tok.depth == 2 {
                    s.dimension = tok.attr("ref").map(|a| a.value_span.clone());
                }
                if !in_views && tok.depth > 2 && parent != "c" && local != "c" && local != "row" {
                    for k in ["ref", "sqref"] {
                        if let Some(a) = tok.attr(k) {
                            let (c, r) = max_of_refs(&a.value());
                            s.anchored = (s.anchored.0.max(c), s.anchored.1.max(r));
                        }
                    }
                }
                if parent == "c" {
                    if let Some(c) = cell.as_mut() {
                        c.value = true;
                    }
                } else if local == "row" && parent == "sheetData" {
                    let r = tok.attr("r").and_then(|a| a.value().parse().ok());
                    if r.is_none() {
                        s.implicit_refs = true;
                    }
                    let rw = Row { span: tok.span.clone(), row: r.unwrap_or(0), cells: Vec::new(), plain: true };
                    if empty {
                        s.rows.push(rw);
                    } else {
                        row = Some(rw);
                    }
                } else if local == "c" && parent == "row" {
                    let pos = tok.attr("r").and_then(|a| parse_ref(&a.value()));
                    if pos.is_none() {
                        s.implicit_refs = true;
                    }
                    let (col, r) = pos.unwrap_or((0, 0));
                    let c = Cell { span: tok.span.clone(), col, row: r, value: false };
                    if empty {
                        if let Some(rw) = row.as_mut() {
                            rw.cells.push(c);
                        }
                    } else {
                        cell = Some(c);
                    }
                } else if parent == "row" {
                    if let Some(rw) = row.as_mut() {
                        rw.plain = false;
                    }
                }
                if !empty {
                    stack.push((local, tok.span.start));
                }
            }
            TokenKind::EndTag => {
                let (local, start) = stack.pop().unwrap_or_default();
                if tok.depth == 2 {
                    s.top.push((local.clone(), start..tok.span.end));
                    in_views = false;
                }
                if local == "c" {
                    if let (Some(mut c), Some(rw)) = (cell.take(), row.as_mut()) {
                        c.span.end = tok.span.end;
                        rw.cells.push(c);
                    }
                } else if local == "row" {
                    if let Some(mut rw) = row.take() {
                        rw.span.end = tok.span.end;
                        s.rows.push(rw);
                    }
                }
            }
            TokenKind::Text | TokenKind::CData if !tok.raw().trim().is_empty() => {
                if let Some(c) = cell.as_mut() {
                    c.value = true;
                }
            }
            _ => {}
        }
    }
    Ok(s)
}

/// Edits trimming the used range of one sheet, and how many rows / cells go.
/// `extra` = bounds anchored by other parts (drawings, tables, comments).
fn trim_sheet(text: &str, extra: (u32, u32)) -> Result<Option<(Edits, usize, usize)>> {
    let s = scan_sheet(text)?;
    if s.implicit_refs {
        return Ok(None);
    }
    let mut last = (extra.0.max(s.anchored.0), extra.1.max(s.anchored.1));
    for r in &s.rows {
        if !r.plain {
            last.1 = last.1.max(r.row);
        }
        for c in r.cells.iter().filter(|c| c.value) {
            last = (last.0.max(c.col), last.1.max(c.row));
        }
    }
    let mut e = Edits::new();
    let (mut rows, mut cells) = (0, 0);
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    let mut grow = |c: u32, r: u32| {
        bbox = Some(match bbox {
            None => (c, r, c, r),
            Some((a, b, x, y)) => (a.min(c), b.min(r), x.max(c), y.max(r)),
        })
    };
    for r in &s.rows {
        if r.row > last.1 && r.plain && r.cells.iter().all(|c| !c.value) {
            e.delete(r.span.clone());
            rows += 1;
            continue;
        }
        for c in &r.cells {
            if !c.value && c.col > last.0 {
                e.delete(c.span.clone());
                cells += 1;
            } else {
                grow(c.col, c.row);
            }
        }
    }
    if let Some(span) = s.dimension.clone() {
        let want = match bbox {
            None => "A1".to_string(),
            Some((a, b, x, y)) if (a, b) == (x, y) => format!("{}{b}", col_name(a)),
            Some((a, b, x, y)) => format!("{}{b}:{}{y}", col_name(a), col_name(x)),
        };
        if text[span.clone()] != want && (rows > 0 || cells > 0 || extends_beyond(&text[span.clone()], bbox)) {
            e.replace(span, want);
        }
    }
    if e.is_empty() {
        return Ok(None);
    }
    Ok(Some((e, rows, cells)))
}

fn extends_beyond(dim: &str, bbox: Option<(u32, u32, u32, u32)>) -> bool {
    let (c, r) = max_of_refs(dim);
    match bbox {
        None => c > 1 || r > 1,
        Some((_, _, x, y)) => c > x || r > y,
    }
}

/// Bounds anchored by a sheet's drawings, tables, comments and threaded comments.
fn anchored_by_parts(pkg: &mut Pkg, sheet: &str) -> Result<(u32, u32)> {
    let mut m = (0u32, 0u32);
    for (r, part) in pkg.rels(sheet)?.into_iter().filter(|r| !r.external).filter_map(|r| {
        let t = mj_formats::ooxml::rels::resolve(sheet, &r.target)?;
        Some((r, t))
    }) {
        let Some(part) = pkg.canonical(&part) else { continue };
        if !pk::is_xml_part(&part) {
            continue;
        }
        let b = pkg.read(&part)?;
        let x = xml::as_utf8(&b)?;
        let t = Tree::parse(x.text)?;
        match r.kind() {
            "drawing" => {
                for el in t.elements.iter().filter(|e| e.local == "row" || e.local == "col") {
                    if let Ok(v) = el.text.trim().parse::<u32>() {
                        if el.local == "row" {
                            m.1 = m.1.max(v + 1);
                        } else {
                            m.0 = m.0.max(v + 1);
                        }
                    }
                }
            }
            _ => {
                for el in &t.elements {
                    for k in ["ref", "sqref"] {
                        if let Some(v) = el.plain(k) {
                            let (c, w) = max_of_refs(v);
                            m = (m.0.max(c), m.1.max(w));
                        }
                    }
                }
            }
        }
    }
    Ok(m)
}

fn sheets(pkg: &mut Pkg, wb: &str) -> Result<Vec<String>> {
    Ok(pk::targets(pkg, wb, "worksheet")?.into_iter().map(|(_, t)| t).collect())
}

fn reset_used_range(pkg: &mut Pkg, wb: &str, rep: &mut OpReport) -> Result<()> {
    let (mut rows, mut cells) = (0, 0);
    for sh in sheets(pkg, wb)? {
        let extra = anchored_by_parts(pkg, &sh)?;
        let b = pkg.read(&sh)?;
        let x = xml::as_utf8(&b)?;
        if let Some((e, r, c)) = trim_sheet(x.text, extra)? {
            let out = xml::apply_edits(&x, e)?;
            pkg.replace(&sh, out)?;
            rows += r;
            cells += c;
        }
    }
    if rows + cells > 0 {
        rep.lossy = true;
        rep.notes.push(format!("used range reset: {rows} empty row(s) and {cells} empty cell(s) removed"));
        if pkg.parts().iter().any(|p| p.to_ascii_lowercase().ends_with("vbaproject.bin")) {
            rep.notes
                .push("warning: this workbook has macros; code relying on UsedRange may behave differently".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- pivot caches (XLS-03)

fn drop_pivot_records(pkg: &mut Pkg, wb: &str, rep: &mut OpReport) -> Result<()> {
    let mut n = 0;
    for (_, def) in pk::targets(pkg, wb, "pivotCacheDefinition")? {
        let records = pk::targets(pkg, &def, "pivotCacheRecords")?;
        if records.is_empty() {
            continue;
        }
        let b = pkg.read(&def)?;
        let x = xml::as_utf8(&b)?;
        let t = Tree::parse(x.text)?;
        // Only caches whose source is a range inside this workbook can be rebuilt.
        let internal = t.by_local("cacheSource").next().is_some_and(|(_, e)| e.plain("type") == Some("worksheet"))
            && t.by_local("worksheetSource").all(|(_, e)| e.rel_attrs().next().is_none());
        if !internal {
            rep.notes.push(format!("{def}: pivot cache from an external source kept"));
            continue;
        }
        let rids: Vec<String> = records.iter().map(|(r, _)| r.id.clone()).collect();
        let mut e = Edits::new();
        pk::set_attr(x.text, &t, 0, "saveData", "0", &mut e);
        pk::set_attr(x.text, &t, 0, "refreshOnLoad", "1", &mut e);
        pk::remove_attr(
            x.text,
            &t,
            0,
            |a| mj_formats::ooxml::tree::is_rel_ns(a.ns.as_deref()) && rids.contains(&a.value),
            &mut e,
        );
        let out = xml::apply_edits(&x, e)?;
        xml::check_well_formed(&out)?;
        pkg.replace(&def, out)?;
        for id in rids {
            pkg.remove_relationship(&def, &id)?;
        }
        n += 1;
    }
    if n > 0 {
        rep.notes.push(format!("{n} pivot cache(s) no longer saved; Excel refreshes them on open"));
    }
    Ok(())
}

// ---------------------------------------------------------------- named styles (XLS-04 partial)

/// Remove custom named styles that no cell format uses, remapping `xfId` indices
/// inside styles.xml (nothing outside styles.xml refers to cell style formats).
pub fn prune_named_styles(text: &str) -> Result<Option<(Edits, usize)>> {
    let t = Tree::parse(text)?;
    if t.root().is_none() {
        return Ok(None);
    }
    let child = |name: &str| t.elements[0].children.iter().copied().find(|&c| t.elements[c].local == name);
    let (Some(csx), Some(cx), Some(cs)) = (child("cellStyleXfs"), child("cellXfs"), child("cellStyles")) else {
        return Ok(None);
    };
    let xf_of =
        |e: &mj_formats::ooxml::tree::Element| e.plain("xfId").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let style_xfs: Vec<usize> = t.children_named(csx, "xf").collect();
    let used: BTreeSet<usize> = t.children_named(cx, "xf").map(|i| xf_of(&t.elements[i])).collect();
    let styles: Vec<usize> = t.children_named(cs, "cellStyle").collect();
    let mut drop_styles = Vec::new();
    for &s in &styles {
        let el = &t.elements[s];
        if el.plain("builtinId").is_none() && !used.contains(&xf_of(el)) {
            drop_styles.push(s);
        }
    }
    if drop_styles.is_empty() {
        return Ok(None);
    }
    let mut keep_xf: BTreeSet<usize> = used.clone();
    keep_xf.insert(0);
    for &s in styles.iter().filter(|s| !drop_styles.contains(s)) {
        keep_xf.insert(xf_of(&t.elements[s]));
    }
    if keep_xf.iter().any(|&i| i >= style_xfs.len()) {
        return Err(Error::parse("styles", "xfId out of range"));
    }
    let remap: HashMap<usize, usize> = keep_xf.iter().enumerate().map(|(new, &old)| (old, new)).collect();
    let mut e = Edits::new();
    for (i, &x) in style_xfs.iter().enumerate() {
        if !keep_xf.contains(&i) {
            e.delete(t.elements[x].span.clone());
        }
    }
    for &s in &drop_styles {
        e.delete(t.elements[s].span.clone());
    }
    let fix = |i: usize, e: &mut Edits| {
        let el = &t.elements[i];
        if let Some(a) = el.attrs.iter().find(|a| a.name == "xfId") {
            let old = a.value.parse::<usize>().unwrap_or(0);
            if let Some(&new) = remap.get(&old) {
                if new != old {
                    e.replace(a.value_span.clone(), new.to_string());
                }
            }
        }
    };
    for i in t.children_named(cx, "xf") {
        fix(i, &mut e);
    }
    for &s in styles.iter().filter(|s| !drop_styles.contains(s)) {
        fix(s, &mut e);
    }
    for (el, n) in [(csx, keep_xf.len()), (cs, styles.len() - drop_styles.len())] {
        if let Some(a) = t.elements[el].attrs.iter().find(|a| a.name == "count") {
            e.replace(a.value_span.clone(), n.to_string());
        }
    }
    Ok(Some((e, drop_styles.len())))
}

fn remove_unused_styles(pkg: &mut Pkg, wb: &str, rep: &mut OpReport) -> Result<()> {
    for (_, st) in pk::targets(pkg, wb, "styles")? {
        let b = pkg.read(&st)?;
        let x = xml::as_utf8(&b)?;
        if let Some((e, n)) = prune_named_styles(x.text)? {
            let out = xml::apply_edits(&x, e)?;
            xml::check_well_formed(&out)?;
            pkg.replace(&st, out)?;
            rep.notes.push(format!("{n} unused named cell style(s) removed"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- conditional formats (XLS-05)

fn canonical_rule(raw: &str) -> String {
    // The priority differs between duplicates and does not change what a rule does.
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find(" priority=\"") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 11..];
        rest = after.find('"').map(|j| &after[j + 1..]).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// Edits removing exact duplicate conditional-formatting rules on the same range.
fn dedupe_cf(text: &str) -> Result<Option<(Edits, usize)>> {
    let s = scan_sheet(text)?;
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut e = Edits::new();
    let mut n = 0;
    for (_, span) in s.top.iter().filter(|(n, _)| n == "conditionalFormatting") {
        let block = &text[span.clone()];
        let t = Tree::parse(block)?;
        let Some(root) = t.root() else { continue };
        let key = {
            let mut refs: Vec<&str> = root.plain("sqref").unwrap_or("").split_whitespace().collect();
            refs.sort_unstable();
            format!("{}|{}", refs.join(" "), root.plain("pivot").unwrap_or(""))
        };
        let rules: Vec<usize> = t.children_named(0, "cfRule").collect();
        let mut dups = Vec::new();
        for &r in &rules {
            let canon = canonical_rule(&block[t.elements[r].span.clone()]);
            if !seen.insert((key.clone(), canon)) {
                dups.push(r);
            }
        }
        if dups.is_empty() {
            continue;
        }
        n += dups.len();
        if dups.len() == rules.len() && t.elements[0].children.len() == rules.len() {
            e.delete(span.clone());
        } else {
            for r in dups {
                let sp = &t.elements[r].span;
                e.delete(span.start + sp.start..span.start + sp.end);
            }
        }
    }
    Ok((n > 0).then_some((e, n)))
}

fn dedupe_conditional_formats(pkg: &mut Pkg, wb: &str, rep: &mut OpReport) -> Result<()> {
    let mut total = 0;
    for sh in sheets(pkg, wb)? {
        let b = pkg.read(&sh)?;
        let x = xml::as_utf8(&b)?;
        if let Some((e, n)) = dedupe_cf(x.text)? {
            pkg.replace(&sh, xml::apply_edits(&x, e)?)?;
            total += n;
        }
    }
    if total > 0 {
        rep.notes.push(format!("{total} duplicate conditional-formatting rule(s) removed"));
    }
    Ok(())
}

// ---------------------------------------------------------------- names, links, hidden sheets

/// What the workbook part says about itself (XLS-06/07 report).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkbookReport {
    pub hidden_sheets: Vec<String>,
    pub broken_names: Vec<String>,
    pub external_links: usize,
    pub macros: bool,
}

fn workbook_report(pkg: &mut Pkg, wb: &str) -> Result<WorkbookReport> {
    let b = pkg.read(wb)?;
    let x = xml::as_utf8(&b)?;
    let t = Tree::parse(x.text)?;
    let mut r = WorkbookReport::default();
    for (_, s) in t.by_local("sheet") {
        if matches!(s.plain("state"), Some("hidden") | Some("veryHidden")) {
            r.hidden_sheets.push(s.plain("name").unwrap_or("?").to_string());
        }
    }
    for (_, d) in t.by_local("definedName") {
        if d.text.contains("#REF!") {
            r.broken_names.push(d.plain("name").unwrap_or("?").to_string());
        }
    }
    r.external_links = pk::targets(pkg, wb, "externalLink")?.len()
        + pkg.rels(wb)?.iter().filter(|x| x.kind() == "externalLink" && x.external).count();
    r.macros = pkg.parts().iter().any(|p| p.to_ascii_lowercase().ends_with("vbaproject.bin"));
    Ok(r)
}

/// True when `name` occurs in `hay` as a whole identifier (not inside a longer name).
fn contains_name(hay: &str, name: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\');
    hay.match_indices(name).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + name.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

fn remove_broken_names(pkg: &mut Pkg, wb: &str, rep: &mut OpReport) -> Result<()> {
    let b = pkg.read(wb)?;
    let x = xml::as_utf8(&b)?;
    let t = Tree::parse(x.text)?;
    let candidates: Vec<(usize, String)> = t
        .by_local("definedName")
        .filter(|(_, d)| d.text.contains("#REF!"))
        .filter_map(|(i, d)| d.plain("name").map(|n| (i, n.to_string())))
        .filter(|(_, n)| !n.starts_with("_xlnm."))
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    // A name used anywhere else (formulas, charts, validations, other names) is kept.
    let mut haystack = String::new();
    for p in pkg.parts() {
        if pk::is_xml_part(&p) && !p.eq_ignore_ascii_case(wb) {
            haystack.push_str(&String::from_utf8_lossy(&pkg.read(&p)?).to_lowercase());
        }
    }
    let mut own = String::new();
    for (_, d) in t.by_local("definedName") {
        own.push_str(&d.text.to_lowercase());
        own.push('\n');
    }
    let mut e = Edits::new();
    let mut removed = Vec::new();
    let mut names = Vec::new();
    for (i, n) in candidates {
        let l = n.to_lowercase();
        if contains_name(&haystack, &l) || contains_name(&own, &l) {
            continue;
        }
        t.remove_element(i, &mut e, &mut removed);
        names.push(n);
    }
    if names.is_empty() {
        return Ok(());
    }
    let out = xml::apply_edits(&x, e)?;
    xml::check_well_formed(&out)?;
    pkg.replace(wb, out)?;
    rep.lossy = true;
    rep.notes.push(format!("broken named range(s) removed: {}", names.join(", ")));
    Ok(())
}

fn ops(pkg: &mut Pkg, p: &ParamValues, rep: &mut OpReport) -> Result<()> {
    let wb = pk::main_part(pkg)?;
    let r = workbook_report(pkg, &wb)?;
    if !r.hidden_sheets.is_empty() {
        rep.notes.push(format!("hidden sheets (review only, kept): {}", r.hidden_sheets.join(", ")));
    }
    if r.external_links > 0 {
        rep.notes.push(format!("{} external link(s) (review only, kept)", r.external_links));
    }
    if p.bool(P_USED_RANGE, false) {
        reset_used_range(pkg, &wb, rep)?;
    }
    if p.bool(P_PIVOT, false) {
        drop_pivot_records(pkg, &wb, rep)?;
    }
    if p.bool(P_STYLES, false) {
        remove_unused_styles(pkg, &wb, rep)?;
    }
    if p.bool(P_CF, false) {
        dedupe_conditional_formats(pkg, &wb, rep)?;
    }
    if p.bool(P_NAMES, false) {
        remove_broken_names(pkg, &wb, rep)?;
    }
    Ok(())
}

/// Review-table columns: bloat breakdown (XLS-01) and the XLS-06/07/10 report.
fn columns(e: &FileEntry) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(dir) = pk::directory(&e.path) {
        m.insert("breakdown".into(), pk::breakdown(&dir).summary());
    }
    if let Ok(mut pkg) = mj_formats::ooxml::Package::open_file(&e.path, Default::default()) {
        if let Ok(wb) = pk::main_part(&mut pkg) {
            if let Ok(r) = workbook_report(&mut pkg, &wb) {
                if !r.hidden_sheets.is_empty() {
                    m.insert("hidden_sheets".into(), r.hidden_sheets.join(", "));
                }
                if !r.broken_names.is_empty() {
                    m.insert("broken_names".into(), r.broken_names.join(", "));
                }
                if r.external_links > 0 {
                    m.insert("external_links".into(), r.external_links.to_string());
                }
                if r.macros {
                    m.insert("macros".into(), "yes: a used-range reset may affect VBA relying on UsedRange".into());
                }
            }
        }
    }
    m
}

impl Optimiser for Excel {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Excel,
            title: "Excel workbooks".into(),
            description: "Trim bloated used ranges, styles and pivot caches".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: (1..=10)
                .map(|i| format!("OPT-XLS-{i:02}"))
                .chain(["OPT-OFC-01".into(), "OPT-OFC-02".into()])
                .collect(),
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && EXCEL_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let mut extra = 0;
        if params.bool(P_PIVOT, false) {
            if let Ok(dir) = pk::directory(&entry.path) {
                extra += dir
                    .iter()
                    .filter(|e| e.name.to_ascii_lowercase().contains("pivotcacherecords"))
                    .map(|e| e.compressed_size)
                    .sum::<u64>();
            }
        }
        pk::estimate(entry, params, extra)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        pk::run(job, FileKind::Xlsx, &ops)
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        framework::analyse_by_content(
            self,
            ctx,
            &[FileKind::Xlsx],
            &["docx", "pptx", "docm", "pptm"],
            |_, _| "Lossless repack; used range, pivot caches and styles can be trimmed".into(),
            columns,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::office::package::fixtures::*;
    use crate::office::package::testkit::*;
    use mj_core::testutil::TempDir;
    use mj_core::ParamValue;

    const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn sml(p: &str) -> String {
        format!("application/vnd.openxmlformats-officedocument.spreadsheetml.{p}+xml")
    }

    pub(crate) fn sheet1() -> String {
        let mut rows = String::new();
        for r in 1..=3 {
            rows.push_str(&format!(r#"<row r="{r}" spans="1:26"><c r="A{r}"><v>{r}</v></c><c r="B{r}" t="s"><v>0</v></c><c r="C{r}"><f>A{r}*2</f><v>{}</v></c>"#, r * 2));
            if r == 1 {
                for c in 4..=26u32 {
                    rows.push_str(&format!(r#"<c r="{}1" s="1"/>"#, col_name(c)));
                }
            }
            rows.push_str("</row>");
        }
        for r in 4..=2000 {
            rows.push_str(&format!(
                r#"<row r="{r}" s="1" customFormat="1"><c r="A{r}" s="1"/><c r="B{r}" s="1"></c></row>"#
            ));
        }
        let cf = r#"<conditionalFormatting sqref="A1:A3"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>1</formula></cfRule></conditionalFormatting>"#;
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{S}" xmlns:r="{R}"><dimension ref="A1:Z2000"/><sheetViews><sheetView workbookViewId="0"><selection activeCell="Z5000" sqref="Z5000"/></sheetView></sheetViews><sheetData>{rows}</sheetData><mergeCells count="1"><mergeCell ref="E10:F11"/></mergeCells>{cf}{}<conditionalFormatting sqref="B1:B3"><cfRule type="cellIs" dxfId="0" priority="3" operator="lessThan"><formula>1</formula></cfRule></conditionalFormatting></worksheet>"#,
            cf.replace("priority=\"1\"", "priority=\"2\"")
        )
    }

    fn styles() -> String {
        format!(
            r#"<?xml version="1.0"?><styleSheet xmlns="{S}"><fonts count="1"><font/></fonts><cellStyleXfs count="3"><xf numFmtId="0"/><xf numFmtId="1"/><xf numFmtId="2"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" xfId="0"/><xf numFmtId="2" xfId="2"/></cellXfs><cellStyles count="3"><cellStyle name="Normal" xfId="0" builtinId="0"/><cellStyle name="Imported 1" xfId="1"/><cellStyle name="Mine" xfId="2"/></cellStyles><dxfs count="1"><dxf/></dxfs></styleSheet>"#
        )
    }

    pub(crate) fn workbook(macros: bool) -> Vec<u8> {
        let mut ov = vec![
            ("xl/workbook.xml".to_string(), sml(if macros { "sheet.macroEnabled.main" } else { "sheet.main" })),
            ("xl/worksheets/sheet1.xml".into(), sml("worksheet")),
            ("xl/worksheets/sheet2.xml".into(), sml("worksheet")),
            ("xl/styles.xml".into(), sml("styles")),
            ("xl/pivotCache/pivotCacheDefinition1.xml".into(), sml("pivotCacheDefinition")),
            ("xl/pivotCache/pivotCacheRecords1.xml".into(), sml("pivotCacheRecords")),
        ];
        for (p, t) in props_overrides(false) {
            ov.push((p.into(), t.into()));
        }
        let ovr: Vec<(&str, &str)> = ov.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let wb = format!(
            r#"<?xml version="1.0"?><workbook xmlns="{S}" xmlns:r="{R}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Secret" sheetId="2" state="hidden" r:id="rId2"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">#REF!</definedName><definedName name="Broken">#REF!$A$1</definedName><definedName name="UsedBroken">#REF!$B$2</definedName><definedName name="Good">Data!$A$1</definedName></definedNames><pivotCaches><pivotCache cacheId="1" r:id="rId4"/></pivotCaches></workbook>"#
        );
        let mut records = String::from(
            r#"<?xml version="1.0"?><pivotCacheRecords xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="500">"#,
        );
        for i in 0..500 {
            records.push_str(&format!("<r><n v=\"{i}\"/><s v=\"item {}\"/></r>", i * 7919 % 1000));
        }
        records.push_str("</pivotCacheRecords>");
        let mut e: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", content_types(&ovr).into_bytes()),
            ("_rels/.rels", root_rels("xl/workbook.xml", false)),
            ("xl/workbook.xml", wb.into_bytes()),
            ("xl/_rels/workbook.xml.rels", rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "worksheets/sheet2.xml"), ("rId3", "styles", "styles.xml"), ("rId4", "pivotCacheDefinition", "pivotCache/pivotCacheDefinition1.xml")]).into_bytes()),
            ("xl/worksheets/sheet1.xml", sheet1().into_bytes()),
            ("xl/worksheets/sheet2.xml", format!(r#"<?xml version="1.0"?><worksheet xmlns="{S}"><sheetData><row r="1"><c r="A1"><f>UsedBroken*2</f><v>0</v></c></row></sheetData></worksheet>"#).into_bytes()),
            ("xl/styles.xml", styles().into_bytes()),
            ("xl/pivotCache/pivotCacheDefinition1.xml", format!(r#"<?xml version="1.0"?><pivotCacheDefinition xmlns="{S}" xmlns:r="{R}" r:id="rId1" refreshedBy="me" recordCount="500"><cacheSource type="worksheet"><worksheetSource ref="A1:C3" sheet="Data"/></cacheSource><cacheFields count="0"/></pivotCacheDefinition>"#).into_bytes()),
            ("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", rels(&[("rId1", "pivotCacheRecords", "pivotCacheRecords1.xml")]).into_bytes()),
            ("xl/pivotCache/pivotCacheRecords1.xml", records.into_bytes()),
        ];
        if macros {
            e.push(("xl/vbaProject.bin", noise(3000, 11)));
        }
        e.extend(doc_props(false));
        build(&e)
    }

    #[test]
    fn references() {
        assert_eq!(parse_ref("$AB$12"), Some((28, 12)));
        assert_eq!(parse_ref("Sheet1!C3"), Some((3, 3)));
        assert_eq!(parse_ref("XFD"), Some((16384, 0)));
        assert_eq!(parse_ref("1048576"), Some((0, 1048576)));
        assert_eq!(parse_ref("ABCD1"), None);
        assert_eq!(col_name(28), "AB");
        assert_eq!(max_of_refs("A1:B5 D2"), (4, 5));
    }

    #[test]
    fn defaults_change_no_sheet() {
        let t = TempDir::new("xls").unwrap();
        let (o, dst) = run_on(&Excel, &t, "book.xlsx", &workbook(false), &ParamValues::new());
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(!o.lossy);
        unzip_ok(&dst);
        let (a, b) = (read_all(&t.join("book.xlsx")), read_all(&dst));
        for p in
            ["xl/worksheets/sheet1.xml", "xl/styles.xml", "xl/workbook.xml", "xl/pivotCache/pivotCacheRecords1.xml"]
        {
            assert_eq!(part(&a, p), part(&b, p), "{p}");
        }
        assert!(o.notes.iter().any(|n| n.contains("hidden sheets") && n.contains("Secret")));
    }

    #[test]
    fn used_range_reset_respects_anchors() {
        let t = TempDir::new("xls2").unwrap();
        let p = ParamValues::new().with(P_USED_RANGE, ParamValue::Bool(true));
        let (o, dst) = run_on(&Excel, &t, "book.xlsm", &workbook(true), &p);
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(o.lossy);
        assert!(o.notes.iter().any(|n| n.contains("macros")), "{:?}", o.notes);
        let s = part(&read_all(&dst), "xl/worksheets/sheet1.xml").unwrap();
        assert!(s.contains(r#"<dimension ref="A1:F11"/>"#), "{}", &s[..200]);
        assert!(s.contains(r#"<row r="11""#) && !s.contains(r#"<row r="12""#), "merge E10:F11 anchors row 11");
        assert!(s.contains(r#"<c r="F1" s="1"/>"#) && !s.contains(r#"<c r="G1""#));
        assert!(s.contains("<f>A3*2</f>"));
        assert!(s.contains(r#"activeCell="Z5000""#), "views untouched");
    }

    #[test]
    fn pivot_styles_cf_and_names() {
        let t = TempDir::new("xls3").unwrap();
        let mut p = ParamValues::new();
        for k in [P_PIVOT, P_STYLES, P_CF, P_NAMES] {
            p.set(k, ParamValue::Bool(true));
        }
        let (o, dst) = run_on(&Excel, &t, "book.xlsx", &workbook(false), &p);
        assert!(o.skipped.is_none(), "{o:?}");
        let all = read_all(&dst);
        assert!(part(&all, "xl/pivotCache/pivotCacheRecords1.xml").is_none());
        let def = part(&all, "xl/pivotCache/pivotCacheDefinition1.xml").unwrap();
        assert!(
            def.contains(r#"saveData="0""#) && def.contains(r#"refreshOnLoad="1""#) && !def.contains("r:id"),
            "{def}"
        );
        assert!(!part(&all, "[Content_Types].xml").unwrap().contains("pivotCacheRecords1"));
        let st = part(&all, "xl/styles.xml").unwrap();
        assert!(!st.contains("Imported 1"));
        assert!(st.contains(r#"<cellStyleXfs count="2"><xf numFmtId="0"/><xf numFmtId="2"/></cellStyleXfs>"#), "{st}");
        assert!(st.contains(r#"<xf numFmtId="2" xfId="1"/>"#) && st.contains(r#"name="Mine" xfId="1""#));
        let s1 = part(&all, "xl/worksheets/sheet1.xml").unwrap();
        assert_eq!(s1.matches("<cfRule").count(), 2);
        let wb = part(&all, "xl/workbook.xml").unwrap();
        assert!(
            !wb.contains("\"Broken\"")
                && wb.contains("UsedBroken")
                && wb.contains("_xlnm.Print_Area")
                && wb.contains("\"Good\"")
        );
        assert!(o.lossy);
    }

    #[test]
    fn named_style_pruning_is_a_noop_without_unused_styles() {
        let x = format!(
            r#"<styleSheet xmlns="{S}"><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="1"><xf xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
        );
        assert!(prune_named_styles(&x).unwrap().is_none());
        assert!(prune_named_styles("<styleSheet/>").unwrap().is_none());
    }

    #[test]
    fn implicit_cell_positions_are_left_alone() {
        let x = format!(
            r#"<worksheet xmlns="{S}"><dimension ref="A1:C9"/><sheetData><row><c><v>1</v></c><c s="1"/></row><row/></sheetData></worksheet>"#
        );
        assert!(trim_sheet(&x, (0, 0)).unwrap().is_none());
    }

    #[test]
    fn estimate_and_columns() {
        let t = TempDir::new("xls4").unwrap();
        let p = t.write("book.xlsm", &workbook(true)).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, std::fs::metadata(&p).unwrap().len());
        let c = columns(&e);
        assert!(c["broken_names"].contains("Broken"));
        assert_eq!(c["hidden_sheets"], "Secret");
        assert!(c["macros"].starts_with("yes"));
        assert!(c["breakdown"].contains("caches"));
        let base = Excel.estimate(&e, &ParamValues::new()).unwrap();
        let more = Excel.estimate(&e, &ParamValues::new().with(P_PIVOT, ParamValue::Bool(true))).unwrap();
        assert!(more.saving() > base.saving());
    }
}
