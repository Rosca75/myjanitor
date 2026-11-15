//! Plain-text output helpers: aligned tables and JSON. Owner: W3-E.

use mj_core::size::format_bytes;

/// Print any serde value as pretty JSON.
pub fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Render rows as a left-aligned table with two spaces between columns;
/// columns listed in `right` are right-aligned. Widths count characters.
pub fn table(rows: &[Vec<String>], right: &[usize]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let width = |c: usize| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(width).collect();
    let mut out = String::new();
    for r in rows {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let pad = widths[i].saturating_sub(cell.chars().count());
                if right.contains(&i) {
                    format!("{}{cell}", " ".repeat(pad))
                } else {
                    format!("{cell}{}", " ".repeat(pad))
                }
            })
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// "1.5 KB" style size.
pub fn bytes(n: u64) -> String {
    format_bytes(n)
}

/// Local-independent date: `2026-11-13 08:47 UTC`.
pub fn when(t: mj_core::Timestamp) -> String {
    let (y, m, d) = t.ymd();
    let (h, mi, _) = t.hms();
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_columns_and_right_aligns_numbers() {
        let rows = vec![
            vec!["name".into(), "size".into()],
            vec!["a".into(), "1.5 KB".into()],
            vec!["longer".into(), "20 B".into()],
        ];
        assert_eq!(table(&rows, &[1]), "name      size\na       1.5 KB\nlonger    20 B\n");
    }

    #[test]
    fn dates_are_utc_text() {
        assert_eq!(when(mj_core::Timestamp::from_ymd_hms(2026, 1, 2, 3, 4, 5)), "2026-01-02 03:04 UTC");
    }
}
