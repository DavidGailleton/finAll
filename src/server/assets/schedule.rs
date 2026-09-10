//! Timing helpers for the opt-in Frankfurter background tasks in `main.rs`:
//! how long to sleep from `now` until the next run boundary. Pure date
//! arithmetic on UTC, no clock reads of their own — the caller passes `now`.

use std::time::Duration;

use sqlx::types::chrono::{DateTime, NaiveDate, Utc};

/// A fallback sleep used only if the target instant cannot be constructed
/// (`chrono` runs out of representable dates); never reached in practice.
const ONE_DAY: Duration = Duration::from_secs(24 * 60 * 60);
const THIRTY_DAYS: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Time from `now` until the next `00:00:00Z` (tomorrow's midnight UTC). Exact
/// midnight is not special-cased — callers run the job first and then sleep, so
/// a run finishing just after `00:00` sleeps ~24h.
pub fn duration_until_next_utc_midnight(now: DateTime<Utc>) -> Duration {
    let next = now
        .date_naive()
        .succ_opt()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));

    match next {
        Some(next) => (next - now).to_std().unwrap_or(Duration::ZERO),
        None => ONE_DAY,
    }
}

/// Time from `now` until `00:00:00Z` on the first day of the next month.
pub fn duration_until_next_month_start(now: DateTime<Utc>) -> Duration {
    // chrono's `Datelike` trait is not re-exported through `sqlx::types::chrono`,
    // so the year and month are read from a formatted date rather than accessor
    // methods.
    let next = now
        .format("%Y-%m")
        .to_string()
        .split_once('-')
        .and_then(|(year, month)| {
            let year: i32 = year.parse().ok()?;
            let month: u32 = month.parse().ok()?;
            let (year, month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            NaiveDate::from_ymd_opt(year, month, 1)
        })
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));

    match next {
        Some(next) => (next - now).to_std().unwrap_or(Duration::ZERO),
        None => THIRTY_DAYS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp literal")
            .with_timezone(&Utc)
    }

    #[test]
    fn midnight_is_the_start_of_the_next_day() {
        assert_eq!(
            duration_until_next_utc_midnight(at("2026-09-10T00:00:00Z")),
            Duration::from_secs(24 * 60 * 60),
        );
        assert_eq!(
            duration_until_next_utc_midnight(at("2026-09-10T23:59:59Z")),
            Duration::from_secs(1),
        );
        assert_eq!(
            duration_until_next_utc_midnight(at("2026-09-10T06:00:00Z")),
            Duration::from_secs(18 * 60 * 60),
        );
    }

    #[test]
    fn month_start_rolls_over_the_year() {
        // Mid-month within the year.
        assert_eq!(
            duration_until_next_month_start(at("2026-09-10T00:00:00Z")),
            Duration::from_secs(21 * 24 * 60 * 60),
        );
        // December rolls into January of the next year.
        assert_eq!(
            duration_until_next_month_start(at("2026-12-15T00:00:00Z")),
            Duration::from_secs(17 * 24 * 60 * 60),
        );
        // Exactly on the 1st still waits a whole month.
        assert_eq!(
            duration_until_next_month_start(at("2026-11-01T00:00:00Z")),
            Duration::from_secs(30 * 24 * 60 * 60),
        );
    }
}
