//! Timestamps and calendar maths (in-house, no `chrono`).
//!
//! [`Timestamp`] is signed nanoseconds since the Unix epoch, UTC. On the wire it
//! is a JSON integer; JavaScript reads it as a double (precision about 256 ns at
//! today's dates, irrelevant for display: `new Date(ts / 1e6)`).
//!
//! Civil-date conversion uses Howard Hinnant's `days_from_civil` /
//! `civil_from_days` algorithms (proleptic Gregorian calendar).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Nanoseconds per second.
pub const NANOS_PER_SEC: i64 = 1_000_000_000;
/// Nanoseconds per day.
pub const NANOS_PER_DAY: i64 = 86_400 * NANOS_PER_SEC;

/// A point in time: nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// The current time.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// From a `SystemTime` (saturating on overflow).
    pub fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp(i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)),
            Err(e) => Timestamp(-i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX)),
        }
    }

    /// Back to a `SystemTime`.
    pub fn to_system_time(self) -> SystemTime {
        if self.0 >= 0 {
            UNIX_EPOCH + Duration::from_nanos(self.0 as u64)
        } else {
            UNIX_EPOCH - Duration::from_nanos(self.0.unsigned_abs())
        }
    }

    /// Whole seconds since the epoch (floor).
    pub fn secs(self) -> i64 {
        self.0.div_euclid(NANOS_PER_SEC)
    }

    /// Milliseconds since the epoch (floor), the unit JavaScript `Date` uses.
    pub fn millis(self) -> i64 {
        self.0.div_euclid(1_000_000)
    }

    /// From seconds since the epoch.
    pub fn from_secs(secs: i64) -> Self {
        Timestamp(secs.saturating_mul(NANOS_PER_SEC))
    }

    /// From a civil date and time (UTC).
    pub fn from_ymd_hms(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> Self {
        let days = days_from_civil(y, m, d);
        Timestamp::from_secs(days * 86_400 + (hh as i64) * 3600 + (mm as i64) * 60 + ss as i64)
    }

    /// From a Windows FILETIME (100 ns ticks since 1601-01-01).
    pub fn from_filetime(ticks: u64) -> Self {
        const EPOCH_DIFF_TICKS: i64 = 116_444_736_000_000_000;
        Timestamp((ticks as i64 - EPOCH_DIFF_TICKS).saturating_mul(100))
    }

    /// Civil date (UTC): `(year, month 1-12, day 1-31)`.
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0.div_euclid(NANOS_PER_DAY))
    }

    /// `(hour, minute, second)` of the UTC day.
    pub fn hms(self) -> (u32, u32, u32) {
        let s = self.0.rem_euclid(NANOS_PER_DAY) / NANOS_PER_SEC;
        ((s / 3600) as u32, ((s / 60) % 60) as u32, (s % 60) as u32)
    }

    /// RFC 3339 / ISO 8601 UTC with second precision, e.g. `2026-11-13T15:04:05Z`.
    pub fn to_rfc3339(self) -> String {
        let (y, m, d) = self.ymd();
        let (hh, mm, ss) = self.hms();
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
    }

    /// Add a signed number of days.
    pub fn add_days(self, days: i64) -> Self {
        Timestamp(self.0.saturating_add(days.saturating_mul(NANOS_PER_DAY)))
    }

    /// Add calendar months (clamping the day, e.g. Mar 31 - 1 month = Feb 28/29).
    pub fn add_months(self, months: i64) -> Self {
        let (y, m, d) = self.ymd();
        let total = y as i64 * 12 + (m as i64 - 1) + months;
        let ny = total.div_euclid(12) as i32;
        let nm = (total.rem_euclid(12) + 1) as u32;
        let nd = d.min(days_in_month(ny, nm));
        let tod = self.0.rem_euclid(NANOS_PER_DAY);
        Timestamp(days_from_civil(ny, nm, nd) * NANOS_PER_DAY + tod)
    }

    /// Whole days from `self` to `later` (negative when `later` is earlier).
    pub fn days_until(self, later: Timestamp) -> i64 {
        (later.0 - self.0).div_euclid(NANOS_PER_DAY)
    }
}

/// True for Gregorian leap years.
pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days in a month (1-12).
pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 for a civil date.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date for days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + if m <= 2 { 1 } else { 0 }) as i32, m, d)
}

/// Unit of a relative span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanUnit {
    Days,
    Weeks,
    Months,
    Years,
}

/// A relative span such as "90 days" or "2 years" (CLN-FLT-04, query `modified:>2y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelativeSpan {
    pub amount: u32,
    pub unit: SpanUnit,
}

impl RelativeSpan {
    /// The instant `self` before `now`, using calendar months/years.
    pub fn before(self, now: Timestamp) -> Timestamp {
        let n = self.amount as i64;
        match self.unit {
            SpanUnit::Days => now.add_days(-n),
            SpanUnit::Weeks => now.add_days(-7 * n),
            SpanUnit::Months => now.add_months(-n),
            SpanUnit::Years => now.add_months(-12 * n),
        }
    }

    /// Parse `90d`, `3w`, `6mo`, `2y` (also `days`, `weeks`, `months`, `years`,
    /// with or without a space). Case-insensitive.
    pub fn parse(s: &str) -> Option<RelativeSpan> {
        let s = s.trim().to_ascii_lowercase();
        let split = s.find(|c: char| !c.is_ascii_digit())?;
        let amount: u32 = s[..split].parse().ok()?;
        let unit = match s[split..].trim() {
            "d" | "day" | "days" => SpanUnit::Days,
            "w" | "wk" | "week" | "weeks" => SpanUnit::Weeks,
            "mo" | "mon" | "month" | "months" => SpanUnit::Months,
            "y" | "yr" | "year" | "years" => SpanUnit::Years,
            _ => return None,
        };
        Some(RelativeSpan { amount, unit })
    }
}

/// A parsed absolute date with its precision, so `modified:<2024` means
/// "before 2024-01-01" and `modified:2024` means "within 2024".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    /// Inclusive start.
    pub start: Timestamp,
    /// Exclusive end.
    pub end: Timestamp,
}

/// Parse `YYYY`, `YYYY-MM`, `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS]` (UTC) into
/// the range it denotes.
pub fn parse_date(s: &str) -> Option<DateRange> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    let num = |p: &str, len: usize| -> Option<u32> {
        if p.len() == len && p.bytes().all(|b| b.is_ascii_digit()) {
            p.parse().ok()
        } else {
            None
        }
    };
    match (parts.as_slice(), time) {
        ([y], None) => {
            let y = num(y, 4)? as i32;
            Some(DateRange {
                start: Timestamp::from_ymd_hms(y, 1, 1, 0, 0, 0),
                end: Timestamp::from_ymd_hms(y + 1, 1, 1, 0, 0, 0),
            })
        }
        ([y, m], None) => {
            let (y, m) = (num(y, 4)? as i32, num(m, 2)?);
            if !(1..=12).contains(&m) {
                return None;
            }
            let start = Timestamp::from_ymd_hms(y, m, 1, 0, 0, 0);
            Some(DateRange { start, end: start.add_months(1) })
        }
        ([y, m, d], t) => {
            let (y, m, d) = (num(y, 4)? as i32, num(m, 2)?, num(d, 2)?);
            if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
                return None;
            }
            match t {
                None => {
                    let start = Timestamp::from_ymd_hms(y, m, d, 0, 0, 0);
                    Some(DateRange { start, end: start.add_days(1) })
                }
                Some(t) => {
                    let t = t.trim_end_matches('Z');
                    let tp: Vec<&str> = t.split(':').collect();
                    let (hh, mm, ss) = match tp.as_slice() {
                        [h, m] => (num(h, 2)?, num(m, 2)?, 0),
                        [h, m, s] => (num(h, 2)?, num(m, 2)?, num(s, 2)?),
                        _ => return None,
                    };
                    if hh > 23 || mm > 59 || ss > 59 {
                        return None;
                    }
                    let start = Timestamp::from_ymd_hms(y, m, d, hh, mm, ss);
                    Some(DateRange { start, end: Timestamp(start.0 + NANOS_PER_SEC) })
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-800_000i64, -1, 0, 1, 10_957, 19_000, 20_405, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn rfc3339_and_filetime() {
        let t = Timestamp::from_ymd_hms(2026, 11, 13, 15, 4, 5);
        assert_eq!(t.to_rfc3339(), "2026-11-13T15:04:05Z");
        assert_eq!(Timestamp::from_filetime(116_444_736_000_000_000), Timestamp(0));
        assert_eq!(Timestamp::from_system_time(t.to_system_time()), t);
    }

    #[test]
    fn month_arithmetic_clamps() {
        let t = Timestamp::from_ymd_hms(2024, 3, 31, 12, 0, 0);
        assert_eq!(t.add_months(-1).ymd(), (2024, 2, 29));
        assert_eq!(t.add_months(-13).ymd(), (2023, 2, 28));
        assert_eq!(t.add_months(10).ymd(), (2025, 1, 31));
    }

    #[test]
    fn relative_spans() {
        let now = Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0);
        assert_eq!(RelativeSpan::parse("90d"), Some(RelativeSpan { amount: 90, unit: SpanUnit::Days }));
        assert_eq!(RelativeSpan::parse("2 years").unwrap().before(now).ymd(), (2024, 11, 13));
        assert_eq!(RelativeSpan::parse("6mo").unwrap().before(now).ymd(), (2026, 5, 13));
        assert_eq!(RelativeSpan::parse("3W").unwrap().before(now).ymd(), (2026, 10, 23));
        assert!(RelativeSpan::parse("6").is_none());
        assert!(RelativeSpan::parse("x2y").is_none());
    }

    #[test]
    fn date_ranges() {
        let r = parse_date("2024").unwrap();
        assert_eq!(r.start.ymd(), (2024, 1, 1));
        assert_eq!(r.end.ymd(), (2025, 1, 1));
        let r = parse_date("2024-02").unwrap();
        assert_eq!(r.end.ymd(), (2024, 3, 1));
        assert!(parse_date("2023-02-29").is_none());
        assert!(parse_date("2024-02-29").is_some());
        let r = parse_date("2024-02-29T10:30").unwrap();
        assert_eq!(r.start.hms(), (10, 30, 0));
        assert!(parse_date("24-01-01").is_none());
    }
}
