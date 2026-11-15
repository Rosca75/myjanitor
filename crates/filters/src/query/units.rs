//! Size, span and date value parsers of the quick-query syntax. Owner: W2-C.
//!
//! Sizes are binary (`1KB` = 1024 bytes, as the rest of the app shows them).
//! All arithmetic is checked, and every year or span is range-checked before
//! the core calendar helpers see it, so no input can overflow or panic.

use mj_core::time::{parse_date, DateRange, RelativeSpan, SpanUnit};

use crate::timeutil;

/// A date value: an absolute period or a relative age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSpec {
    Abs(DateRange),
    Rel(RelativeSpan),
}

/// Parse `1GB`, `500k`, `1.5 MB`, `2048` (bytes) into bytes.
pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    if num.is_empty() {
        return Err(format!("'{s}' is not a size; write a number with an optional unit, e.g. 500KB or 1.5GB"));
    }
    let mult: u128 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        other => return Err(format!("unknown size unit '{other}'; use B, KB, MB, GB or TB")),
    };
    let (int_part, frac_part) = match num.split_once('.') {
        Some((i, f)) => (i, f),
        None => (num, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() || frac_part.contains('.') || int_part.len() > 18 {
        return Err(format!("'{s}' is not a valid size number"));
    }
    let int: u128 =
        if int_part.is_empty() { 0 } else { int_part.parse().map_err(|_| format!("'{s}' is too large"))? };
    let frac_digits = &frac_part[..frac_part.len().min(9)];
    let frac: u128 = if frac_digits.is_empty() { 0 } else { frac_digits.parse().unwrap_or(0) };
    let scale = 10u128.pow(frac_digits.len() as u32);
    let bytes = int * mult + frac * mult / scale;
    u64::try_from(bytes).map_err(|_| format!("'{s}' is larger than the maximum file size"))
}

/// Parse a non-negative integer (`depth:>3`).
pub fn parse_u32(s: &str) -> Result<u32, String> {
    s.trim().parse::<u32>().map_err(|_| format!("'{s}' is not a whole number"))
}

/// Parse `90d`, `3w`, `6m` (months), `2y`, `30days`, `1 year`.
pub fn parse_span(s: &str) -> Option<RelativeSpan> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let amount: u32 = s[..split].parse().ok()?;
    let unit = match s[split..].trim() {
        "d" | "day" | "days" => SpanUnit::Days,
        "w" | "wk" | "week" | "weeks" => SpanUnit::Weeks,
        "m" | "mo" | "mon" | "month" | "months" => SpanUnit::Months,
        "y" | "yr" | "year" | "years" => SpanUnit::Years,
        _ => return None,
    };
    Some(RelativeSpan { amount, unit })
}

/// Rough length of a span in days (only for ordering and sanity checks).
pub fn span_days(s: RelativeSpan) -> u64 {
    let a = s.amount as u64;
    match s.unit {
        SpanUnit::Days => a,
        SpanUnit::Weeks => a * 7,
        SpanUnit::Months => a * 31,
        SpanUnit::Years => a * 366,
    }
}

/// Parse a date value: `2024`, `2024-03`, `2024-03-15`, `2024-03-15T10:30`, or a span.
pub fn parse_datespec(s: &str) -> Result<DateSpec, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("a date is missing; use YYYY, YYYY-MM, YYYY-MM-DD or a span like 30d, 6m, 2y".into());
    }
    if let Some(span) = parse_span(s) {
        if span_days(span) > 366 * 250 {
            return Err(format!("'{s}' reaches back further than the year {}", timeutil::MIN_YEAR));
        }
        return Ok(DateSpec::Rel(span));
    }
    let head: String = s.chars().take(4).collect();
    if head.len() == 4 && head.bytes().all(|b| b.is_ascii_digit()) {
        let year: i32 = head.parse().unwrap_or(0);
        timeutil::check_year(year)?;
    }
    parse_date(s).map(DateSpec::Abs).ok_or_else(|| {
        if s.bytes().all(|b| b.is_ascii_digit()) {
            format!("'{s}' needs a unit (30d, 6m, 2y) or must be a 4-digit year")
        } else {
            format!("'{s}' is not a date; use YYYY, YYYY-MM, YYYY-MM-DD or a span like 30d, 6m, 2y")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_binary_and_checked() {
        assert_eq!(parse_size("0"), Ok(0));
        assert_eq!(parse_size("1GB"), Ok(1 << 30));
        assert_eq!(parse_size("500k"), Ok(500 << 10));
        assert_eq!(parse_size("1.5MB"), Ok(3 << 19));
        assert_eq!(parse_size("2 TB"), Ok(2 << 40));
        assert_eq!(parse_size("10b"), Ok(10));
        assert_eq!(parse_size("1KiB"), Ok(1024));
        assert_eq!(parse_size(".5k"), Ok(512));
        assert!(parse_size("").is_err());
        assert!(parse_size("GB").is_err());
        assert!(parse_size("5XB").is_err());
        assert!(parse_size("1.2.3").is_err());
        assert!(parse_size("99999999999999TB").is_err());
        assert!(parse_size("9999999999999999999999").is_err());
    }

    #[test]
    fn spans_and_dates() {
        assert_eq!(parse_span("90d"), Some(RelativeSpan { amount: 90, unit: SpanUnit::Days }));
        assert_eq!(parse_span("6m"), Some(RelativeSpan { amount: 6, unit: SpanUnit::Months }));
        assert_eq!(parse_span("2Y"), Some(RelativeSpan { amount: 2, unit: SpanUnit::Years }));
        assert_eq!(parse_span("2024"), None);
        assert!(matches!(parse_datespec("2024"), Ok(DateSpec::Abs(_))));
        assert!(matches!(parse_datespec("2024-03"), Ok(DateSpec::Abs(_))));
        assert!(matches!(parse_datespec("2024-03-15"), Ok(DateSpec::Abs(_))));
        assert!(matches!(parse_datespec("30d"), Ok(DateSpec::Rel(_))));
        assert!(parse_datespec("2024-13").is_err());
        assert!(parse_datespec("12").unwrap_err().contains("unit"));
        assert!(parse_datespec("9999-12").is_err(), "no overflow in the core calendar helpers");
        assert!(parse_datespec("9999").is_err());
        assert!(parse_datespec("99999y").is_err());
        assert!(parse_datespec("nonsense").is_err());
    }
}
