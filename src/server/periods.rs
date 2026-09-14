//! Pure date arithmetic resolving a [`PeriodPreset`](crate::periods::types::PeriodPreset)
//! against an explicitly given "today" — this module never reads the clock
//! itself; [`crate::periods::api::resolve_period`] does that once, on demand,
//! when a visitor picks a preset.
//!
//! `sqlx::types::chrono` only re-exports a handful of bare chrono types
//! (`DateTime`, `NaiveDate`, `Utc`, ...) — no `Datelike`, `Duration`, `Days`,
//! or `Months`, since `chrono` itself is only a transitive dependency (pulled
//! in by `sqlx`'s `chrono` feature), not one this crate declares directly.
//! Every computation below goes through `NaiveDate::format` (to read a
//! year/month/weekday number) and `NaiveDate::pred_opt` (to step back a day
//! at a time) — both plain inherent methods — rather than adding `chrono`
//! directly for those few trait methods and types.

use sqlx::types::chrono::NaiveDate;

use crate::periods::types::PeriodPreset;

#[derive(Debug, thiserror::Error)]
pub enum PeriodError {
    #[error("you must be signed in to do this")]
    Unauthorized,
}

/// `(year, month)` for `date`.
fn year_month(date: NaiveDate) -> (i32, u32) {
    let year = date
        .format("%Y")
        .to_string()
        .parse()
        .expect("chrono formats %Y as an integer");
    let month = date
        .format("%m")
        .to_string()
        .parse()
        .expect("chrono formats %m as an integer");
    (year, month)
}

/// ISO weekday number: `1` = Monday, ..., `7` = Sunday.
fn iso_weekday(date: NaiveDate) -> u32 {
    date.format("%u")
        .to_string()
        .parse()
        .expect("chrono formats %u as an integer")
}

/// `date` stepped back `n` days. `n` stays small in this module (at most 89,
/// for "last 90 days"), so stepping one day at a time is simpler than
/// reimplementing calendar-aware day arithmetic by hand.
fn days_before(date: NaiveDate, n: u32) -> NaiveDate {
    let mut d = date;
    for _ in 0..n {
        d = d
            .pred_opt()
            .expect("stepping back a small number of days from a valid date stays in range");
    }
    d
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = year_month(date);
    NaiveDate::from_ymd_opt(year, month, 1).expect("a valid first-of-month date")
}

/// The `n`-th month before `(year, month)`, as `(year, month)`.
fn months_before(year: i32, month: u32, n: u32) -> (i32, u32) {
    let zero_based = (year * 12 + month as i32 - 1) - n as i32;
    (
        zero_based.div_euclid(12),
        zero_based.rem_euclid(12) as u32 + 1,
    )
}

/// Resolve `preset`'s inclusive `[from, to]` bounds relative to `today`.
pub fn resolve(preset: PeriodPreset, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match preset {
        PeriodPreset::ThisWeek => {
            let weekday = iso_weekday(today); // 1..=7, Monday first
            (days_before(today, weekday - 1), today)
        }
        PeriodPreset::Last7Days => (days_before(today, 6), today),
        PeriodPreset::ThisMonth => (first_of_month(today), today),
        PeriodPreset::Last30Days => (days_before(today, 29), today),
        PeriodPreset::LastMonth => {
            let (year, month) = year_month(today);
            let (start_year, start_month) = months_before(year, month, 1);
            let start = NaiveDate::from_ymd_opt(start_year, start_month, 1)
                .expect("a valid first-of-month date");
            let end = first_of_month(today)
                .pred_opt()
                .expect("the day before this month's first day is valid");
            (start, end)
        }
        PeriodPreset::Last90Days => (days_before(today, 89), today),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid test date")
    }

    #[test]
    fn last_7_days_is_a_7_day_inclusive_window_ending_today() {
        let today = date(2026, 9, 14);
        let (from, to) = resolve(PeriodPreset::Last7Days, today);
        assert_eq!(to, today);
        assert_eq!(from, date(2026, 9, 8));
    }

    #[test]
    fn last_30_days_is_a_30_day_inclusive_window_ending_today() {
        let today = date(2026, 9, 14);
        let (from, to) = resolve(PeriodPreset::Last30Days, today);
        assert_eq!(to, today);
        assert_eq!(from, date(2026, 8, 16));
    }

    #[test]
    fn last_90_days_is_a_90_day_inclusive_window_ending_today() {
        let today = date(2026, 9, 14);
        let (from, to) = resolve(PeriodPreset::Last90Days, today);
        assert_eq!(to, today);
        assert_eq!(from, date(2026, 6, 17));
    }

    #[test]
    fn this_week_is_a_no_op_when_today_is_a_monday() {
        let today = date(2024, 1, 1); // a known Monday
        let (from, to) = resolve(PeriodPreset::ThisWeek, today);
        assert_eq!(from, today);
        assert_eq!(to, today);
    }

    #[test]
    fn this_week_starts_on_the_preceding_monday() {
        let today = date(2024, 1, 5); // the Friday of the same week
        let (from, to) = resolve(PeriodPreset::ThisWeek, today);
        assert_eq!(from, date(2024, 1, 1));
        assert_eq!(to, today);
    }

    #[test]
    fn this_month_starts_on_the_1st_and_ends_today() {
        let today = date(2026, 9, 14);
        let (from, to) = resolve(PeriodPreset::ThisMonth, today);
        assert_eq!(to, today);
        assert_eq!(from, date(2026, 9, 1));
    }

    #[test]
    fn last_month_is_the_full_previous_calendar_month() {
        let today = date(2026, 9, 14);
        let (from, to) = resolve(PeriodPreset::LastMonth, today);
        assert_eq!(from, date(2026, 8, 1));
        assert_eq!(to, date(2026, 8, 31));
    }

    #[test]
    fn last_month_rolls_back_across_a_year_boundary() {
        let today = date(2026, 1, 15);
        let (from, to) = resolve(PeriodPreset::LastMonth, today);
        assert_eq!(from, date(2025, 12, 1));
        assert_eq!(to, date(2025, 12, 31));
    }
}
