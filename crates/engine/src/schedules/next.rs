//! Next-run maths for schedules (AUT-02), in-house and DST-agnostic. Owner: W3-E.
//!
//! A cadence is a wall-clock time. The maths runs on a *fixed* UTC offset in
//! minutes (the machine's current one, see [`local_offset_minutes`]); a DST
//! change is not modelled, so a displayed next run can be an hour off until the
//! next recomputation (the OS scheduler itself fires at wall-clock time).

use mj_core::time::{civil_from_days, days_from_civil};
use mj_core::{Cadence, Error, Result, Timestamp};

/// Check hour, minute, weekday and day ranges.
pub fn validate_cadence(c: &Cadence) -> Result<()> {
    let (h, m) = match *c {
        Cadence::Daily { hour, minute } => (hour, minute),
        Cadence::Weekly { weekday, hour, minute } => {
            if !(1..=7).contains(&weekday) {
                return Err(Error::InvalidInput("cadence.weekday must be 1 (Monday) to 7 (Sunday)".into()));
            }
            (hour, minute)
        }
        Cadence::Monthly { day, hour, minute } => {
            if !(1..=28).contains(&day) {
                return Err(Error::InvalidInput("cadence.day must be between 1 and 28".into()));
            }
            (hour, minute)
        }
    };
    if h > 23 || m > 59 {
        return Err(Error::InvalidInput("cadence time must be 00:00 to 23:59".into()));
    }
    Ok(())
}

fn hm(c: &Cadence) -> (u8, u8) {
    match *c {
        Cadence::Daily { hour, minute }
        | Cadence::Weekly { hour, minute, .. }
        | Cadence::Monthly { hour, minute, .. } => (hour, minute),
    }
}

/// The first run strictly after `after`, for a machine at `offset_min` minutes east of UTC.
pub fn next_run(c: &Cadence, after: Timestamp, offset_min: i32) -> Timestamp {
    let off = offset_min as i64 * 60;
    let local = after.secs() + off;
    let day = local.div_euclid(86_400);
    let (h, m) = hm(c);
    let at = |d: i64| d * 86_400 + h as i64 * 3600 + m as i64 * 60;
    let candidate = match *c {
        Cadence::Daily { .. } => {
            let t = at(day);
            if t > local {
                t
            } else {
                at(day + 1)
            }
        }
        Cadence::Weekly { weekday, .. } => {
            // 1970-01-01 was a Thursday (ISO 4).
            let wd = (day + 3).rem_euclid(7) + 1;
            let delta = (weekday as i64 - wd).rem_euclid(7);
            let t = at(day + delta);
            if t > local {
                t
            } else {
                at(day + delta + 7)
            }
        }
        Cadence::Monthly { day: dom, .. } => {
            let (y, mo, _) = civil_from_days(day);
            let this = at(days_from_civil(y, mo, dom as u32));
            if this > local {
                this
            } else {
                let (ny, nm) = if mo == 12 { (y + 1, 1) } else { (y, mo + 1) };
                at(days_from_civil(ny, nm, dom as u32))
            }
        }
    };
    Timestamp::from_secs(candidate - off)
}

/// `+0530` / `-0800` (output of `date +%z`) to minutes east of UTC.
pub fn parse_date_z(s: &str) -> Option<i32> {
    let s = s.trim();
    let sign = match s.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits = &s[1..];
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i32, i32) = (digits[..2].parse().ok()?, digits[2..].parse().ok()?);
    Some(sign * (h * 60 + m))
}

/// `reg query ...TimeZoneInformation /v ActiveTimeBias` output to minutes east of UTC
/// (the bias is minutes *west*, a 32-bit two's-complement hex DWORD).
pub fn parse_active_bias(out: &str) -> Option<i32> {
    let hex = out.split_whitespace().find_map(|w| w.strip_prefix("0x"))?;
    let bias = u32::from_str_radix(hex, 16).ok()? as i32;
    Some(-bias)
}

/// Env override for the UTC offset (minutes); used by tests and odd setups.
pub const OFFSET_VAR: &str = "MYJANITOR_UTC_OFFSET_MINUTES";

/// The machine's current UTC offset in minutes (cached): override, then `date +%z`
/// on Linux or the registry on Windows, else 0 (UTC).
pub fn local_offset_minutes() -> i32 {
    static CACHE: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        if let Some(v) = std::env::var(OFFSET_VAR).ok().and_then(|v| v.trim().parse().ok()) {
            return v;
        }
        use crate::settings::os::{Cmd, Runner, StdRunner};
        let out = if cfg!(windows) {
            let c = Cmd::new(
                "reg.exe",
                ["query", "HKLM\\SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation", "/v", "ActiveTimeBias"]
                    .map(String::from)
                    .to_vec(),
            );
            StdRunner.run(&c).ok().filter(|o| o.ok).and_then(|o| parse_active_bias(&o.stdout))
        } else {
            StdRunner
                .run(&Cmd::new("date", vec!["+%z".into()]))
                .ok()
                .filter(|o| o.ok)
                .and_then(|o| parse_date_z(&o.stdout))
        };
        out.unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(y: i32, m: u32, d: u32, h: u32, mi: u32) -> Timestamp {
        Timestamp::from_ymd_hms(y, m, d, h, mi, 0)
    }

    #[test]
    fn daily_today_or_tomorrow() {
        let c = Cadence::Daily { hour: 8, minute: 47 };
        assert_eq!(next_run(&c, ts(2026, 11, 13, 7, 0), 0), ts(2026, 11, 13, 8, 47));
        assert_eq!(next_run(&c, ts(2026, 11, 13, 8, 47), 0), ts(2026, 11, 14, 8, 47), "strictly after");
        assert_eq!(next_run(&c, ts(2026, 12, 31, 23, 0), 0), ts(2027, 1, 1, 8, 47));
    }

    #[test]
    fn weekly_finds_the_weekday() {
        // 2026-11-13 is a Friday.
        let mon = Cadence::Weekly { weekday: 1, hour: 8, minute: 0 };
        assert_eq!(next_run(&mon, ts(2026, 11, 13, 9, 0), 0), ts(2026, 11, 16, 8, 0));
        let fri = Cadence::Weekly { weekday: 5, hour: 8, minute: 0 };
        assert_eq!(next_run(&fri, ts(2026, 11, 13, 7, 0), 0), ts(2026, 11, 13, 8, 0));
        assert_eq!(next_run(&fri, ts(2026, 11, 13, 9, 0), 0), ts(2026, 11, 20, 8, 0));
        let sun = Cadence::Weekly { weekday: 7, hour: 0, minute: 0 };
        assert_eq!(next_run(&sun, ts(2026, 11, 13, 9, 0), 0), ts(2026, 11, 15, 0, 0));
    }

    #[test]
    fn monthly_handles_month_and_year_ends_and_leap_february() {
        let c = Cadence::Monthly { day: 28, hour: 6, minute: 30 };
        assert_eq!(next_run(&c, ts(2026, 11, 13, 0, 0), 0), ts(2026, 11, 28, 6, 30));
        assert_eq!(next_run(&c, ts(2026, 11, 28, 6, 30), 0), ts(2026, 12, 28, 6, 30));
        assert_eq!(next_run(&c, ts(2026, 12, 29, 0, 0), 0), ts(2027, 1, 28, 6, 30));
        assert_eq!(next_run(&c, ts(2028, 2, 1, 0, 0), 0), ts(2028, 2, 28, 6, 30));
        let first = Cadence::Monthly { day: 1, hour: 0, minute: 0 };
        assert_eq!(next_run(&first, ts(2028, 2, 29, 12, 0), 0), ts(2028, 3, 1, 0, 0));
        assert_eq!(next_run(&first, ts(2026, 12, 1, 0, 0), 0), ts(2027, 1, 1, 0, 0));
    }

    #[test]
    fn offsets_shift_the_wall_clock() {
        let c = Cadence::Daily { hour: 8, minute: 0 };
        // UTC+2: 08:00 local is 06:00 UTC.
        assert_eq!(next_run(&c, ts(2026, 11, 13, 5, 0), 120), ts(2026, 11, 13, 6, 0));
        assert_eq!(next_run(&c, ts(2026, 11, 13, 7, 0), 120), ts(2026, 11, 14, 6, 0));
        // UTC-8 across midnight UTC: 08:00 local is 16:00 UTC.
        assert_eq!(next_run(&c, ts(2026, 11, 13, 17, 0), -480), ts(2026, 11, 14, 16, 0));
        // A local weekday differs from the UTC one near midnight (Fri 23:00 UTC is Sat in UTC+2).
        let sat = Cadence::Weekly { weekday: 6, hour: 1, minute: 0 };
        // Sat 01:00 local (UTC+2) is Fri 23:00 UTC: not strictly after it, so the next Saturday.
        assert_eq!(next_run(&sat, ts(2026, 11, 13, 23, 0), 120), ts(2026, 11, 20, 23, 0));
        assert_eq!(next_run(&sat, ts(2026, 11, 13, 22, 0), 120), ts(2026, 11, 13, 23, 0));
    }

    #[test]
    fn validation_bounds() {
        assert!(validate_cadence(&Cadence::Daily { hour: 24, minute: 0 }).is_err());
        assert!(validate_cadence(&Cadence::Daily { hour: 23, minute: 60 }).is_err());
        assert!(validate_cadence(&Cadence::Weekly { weekday: 0, hour: 1, minute: 1 }).is_err());
        assert!(validate_cadence(&Cadence::Weekly { weekday: 8, hour: 1, minute: 1 }).is_err());
        assert!(validate_cadence(&Cadence::Monthly { day: 29, hour: 1, minute: 1 }).is_err());
        assert!(validate_cadence(&Cadence::Monthly { day: 0, hour: 1, minute: 1 }).is_err());
        assert!(validate_cadence(&Cadence::Monthly { day: 28, hour: 23, minute: 59 }).is_ok());
    }

    #[test]
    fn offset_parsers() {
        assert_eq!(parse_date_z("+0530\n"), Some(330));
        assert_eq!(parse_date_z("-0800"), Some(-480));
        assert_eq!(parse_date_z("UTC"), None);
        // UTC+1 (CET): bias -60 = 0xffffffc4.
        assert_eq!(parse_active_bias("    ActiveTimeBias    REG_DWORD    0xffffffc4\n"), Some(60));
        assert_eq!(parse_active_bias("    ActiveTimeBias    REG_DWORD    0x1e0"), Some(-480));
        assert_eq!(parse_active_bias("nothing"), None);
    }
}
