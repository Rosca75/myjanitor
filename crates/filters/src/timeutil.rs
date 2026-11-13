//! Overflow-safe wrappers around the date maths in `mj_core::time`. Owner: W2-C.
//!
//! `Timestamp` is nanoseconds in an `i64`, which only spans the years
//! 1677-2262. The core calendar helpers multiply days by `NANOS_PER_DAY`
//! unchecked, so a hostile "50000 years ago" would overflow. Every relative
//! span and absolute year that comes from a rule or a query goes through the
//! checks here first.

use mj_core::time::{RelativeSpan, SpanUnit};
use mj_core::Timestamp;

/// First and last calendar year accepted for absolute dates and computed bounds.
pub const MIN_YEAR: i32 = 1700;
/// See [`MIN_YEAR`].
pub const MAX_YEAR: i32 = 2250;

/// `now - span` with calendar months and years, or an explanation when the
/// result would fall outside the representable range.
pub fn span_before(span: RelativeSpan, now: Timestamp) -> Result<Timestamp, String> {
    let (ny, _, _) = now.ymd();
    if !(MIN_YEAR..=MAX_YEAR).contains(&ny) {
        return Err(format!("the current date (year {ny}) is outside the supported range"));
    }
    let months = match span.unit {
        SpanUnit::Months => span.amount as i64,
        SpanUnit::Years => span.amount as i64 * 12,
        SpanUnit::Days | SpanUnit::Weeks => 0,
    };
    let days = match span.unit {
        SpanUnit::Days => span.amount as i64,
        SpanUnit::Weeks => span.amount as i64 * 7,
        _ => 0,
    };
    // Target year of the subtraction, computed without touching the unchecked helpers.
    let approx_year = ny as i64 - months / 12 - days / 365;
    if approx_year < MIN_YEAR as i64 {
        return Err(format!(
            "{} {} ago is before the year {MIN_YEAR}; use a smaller span",
            span.amount,
            unit_name(span.unit)
        ));
    }
    Ok(span.before(now))
}

/// Check that an absolute year is in the supported range.
pub fn check_year(y: i32) -> Result<(), String> {
    if (MIN_YEAR..=MAX_YEAR).contains(&y) {
        Ok(())
    } else {
        Err(format!("year {y} is outside the supported range {MIN_YEAR}-{MAX_YEAR}"))
    }
}

/// Plural unit name for messages.
pub fn unit_name(u: SpanUnit) -> &'static str {
    match u {
        SpanUnit::Days => "days",
        SpanUnit::Weeks => "weeks",
        SpanUnit::Months => "months",
        SpanUnit::Years => "years",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_spans_are_bounded() {
        let now = Timestamp::from_ymd_hms(2026, 11, 13, 12, 0, 0);
        let two_years = span_before(RelativeSpan { amount: 2, unit: SpanUnit::Years }, now).unwrap();
        assert_eq!(two_years.ymd(), (2024, 11, 13));
        assert!(span_before(RelativeSpan { amount: 90_000, unit: SpanUnit::Years }, now).is_err());
        assert!(span_before(RelativeSpan { amount: u32::MAX, unit: SpanUnit::Months }, now).is_err());
        assert!(span_before(RelativeSpan { amount: u32::MAX, unit: SpanUnit::Days }, now).is_err());
        assert!(span_before(RelativeSpan { amount: 1, unit: SpanUnit::Days }, Timestamp(i64::MAX)).is_err());
        assert!(check_year(10_000).is_err());
        assert!(check_year(2024).is_ok());
    }
}
