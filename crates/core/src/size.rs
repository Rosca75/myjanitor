//! Byte-size formatting and parsing.
//!
//! Units are binary (1 KB = 1024 B) with the Windows Explorer labels
//! `B, KB, MB, GB, TB, PB` (decision D-107): MyJanitor's numbers must agree
//! with what Explorer and the drive bars show on the primary platform. The
//! parser also accepts `KiB/MiB/...` and treats them identically.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Format bytes with one decimal above KB: `0 B`, `512 B`, `1.5 KB`, `12.4 GB`.
/// Values of 100 or more in their unit drop the decimal (`354 GB`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // Round first so 1023.96 KB prints as "1.0 MB", not "1024.0 KB".
    let rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1024.0 && unit < UNITS.len() - 1 {
        value = rounded / 1024.0;
        unit += 1;
    } else {
        value = rounded;
    }
    if value >= 100.0 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Parse `"1GB"`, `"1.5 gb"`, `"500MiB"`, `"42"` (bytes), `"10k"`. Case-insensitive.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let num: f64 = num.parse().ok()?;
    if !num.is_finite() || num < 0.0 {
        return None;
    }
    let mult: u64 = match unit.trim() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        "p" | "pb" | "pib" => 1 << 50,
        _ => return None,
    };
    let v = num * mult as f64;
    if v > u64::MAX as f64 {
        return None;
    }
    Some(v.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024 - 1), "1.0 MB");
        assert_eq!(format_bytes(12_400 * 1024 * 1024), "12.1 GB");
        assert_eq!(format_bytes(354 * (1 << 30)), "354 GB");
    }

    #[test]
    fn parses() {
        assert_eq!(parse_size("42"), Some(42));
        assert_eq!(parse_size("1GB"), Some(1 << 30));
        assert_eq!(parse_size("1.5 kb"), Some(1536));
        assert_eq!(parse_size("500MiB"), Some(500 << 20));
        assert_eq!(parse_size("1x"), None);
        assert_eq!(parse_size(""), None);
    }
}
