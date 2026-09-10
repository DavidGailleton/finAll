//! Timing helper for the Frankfurter currency-sync background task in `main.rs`:
//! how long to sleep from `now` until the next run boundary. Pure date
//! arithmetic on UTC, no clock reads of its own — the caller passes `now`.

use std::time::Duration;

use sqlx::types::chrono::{DateTime, NaiveDate, Utc};

/// A fallback sleep used only if the target instant cannot be constructed
/// (`chrono` runs out of representable dates); never reached in practice.
const THIRTY_DAYS: Duration = Duration::from_secs(30 * 24 * 60 * 60);

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
