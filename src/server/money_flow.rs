//! Money flow: a 12-calendar-month income/expense trend, plus the same
//! income/expense/net totals as [`crate::server::income_expense`] for one
//! chosen period.
//!
//! The trend covers the 12 calendar months ending at the month of the chosen
//! `to` date (never the system clock — the anchor is always the caller's own
//! `to`). Each month's income and expense magnitudes are valued the same way
//! [`crate::server::income_expense`] values a category: each day's subtotal
//! converted at that day's own booking-date rate, then summed. Transfers are
//! excluded, matching every other report in this app.
//!
//! Every query is scoped by `user_id`.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, RoundingMode, Zero};
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::assets::conversion::{self, ConversionError};
use crate::server::assets::currency::{self, CurrencyError};
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::assets::rates::{self, RateResolutionError, ResolvedRate};
use crate::server::income_expense::{self, IncomeExpenseError};

/// Same rounding policy as every other converted report in this app.
const MONEY_FLOW_ROUNDING: RoundingMode = RoundingMode::HalfEven;

#[derive(Debug, thiserror::Error)]
pub enum MoneyFlowError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("that display currency does not exist")]
    CurrencyNotFound,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for MoneyFlowError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("money flow: database error: {err}");
        MoneyFlowError::Internal
    }
}

impl From<CurrencyError> for MoneyFlowError {
    fn from(err: CurrencyError) -> Self {
        match err {
            CurrencyError::InvalidInput(message) => MoneyFlowError::InvalidInput(message),
            CurrencyError::NotFound => MoneyFlowError::CurrencyNotFound,
            _ => MoneyFlowError::Internal,
        }
    }
}

impl From<ConversionError> for MoneyFlowError {
    fn from(err: ConversionError) -> Self {
        logging::error!("money flow: conversion rejected a server-built input: {err}");
        MoneyFlowError::Internal
    }
}

impl From<IncomeExpenseError> for MoneyFlowError {
    fn from(err: IncomeExpenseError) -> Self {
        match err {
            IncomeExpenseError::Unauthorized => MoneyFlowError::Unauthorized,
            IncomeExpenseError::InvalidInput(message) => MoneyFlowError::InvalidInput(message),
            IncomeExpenseError::CurrencyNotFound => MoneyFlowError::CurrencyNotFound,
            IncomeExpenseError::Internal => MoneyFlowError::Internal,
        }
    }
}

/// The currency the report is expressed in.
pub struct DisplayCurrency {
    pub asset_id: Uuid,
    pub alphabetic_code: String,
    pub minor_units: i16,
}

/// One month's valued income and expense magnitudes.
pub struct MonthFlow {
    /// First day of the month.
    pub month: NaiveDate,
    /// Sum of the month's positive amounts, valued (`>= 0`).
    pub income: BigDecimal,
    /// Sum of the month's negative amounts, valued (`<= 0`).
    pub expense: BigDecimal,
}

/// The whole report: the trailing 12-month trend, plus the `[from, to]`
/// totals.
pub struct Report {
    pub display: DisplayCurrency,
    pub months: Vec<MonthFlow>,
    pub total_income: BigDecimal,
    pub total_expense: BigDecimal,
    pub net: BigDecimal,
    pub complete: bool,
}

/// `(year, month)` for `date`. `sqlx::types::chrono` re-exports only the bare
/// date/time types, not the `Datelike` trait those fields normally come
/// through, so this goes via `NaiveDate`'s own `format` instead of adding
/// `chrono` as a direct dependency for one trait.
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

/// The `n`-th month before `(year, month)`, as `(year, month)`.
fn months_before(year: i32, month: u32, n: u32) -> (i32, u32) {
    let zero_based = (year * 12 + month as i32 - 1) - n as i32;
    (
        zero_based.div_euclid(12),
        zero_based.rem_euclid(12) as u32 + 1,
    )
}

/// The 12 calendar months ending at `anchor`'s month, oldest first, each as
/// the first day of that month.
fn trailing_months(anchor: NaiveDate) -> Vec<NaiveDate> {
    let (year, month) = year_month(anchor);
    (0..12)
        .rev()
        .map(|offset| {
            let (y, m) = months_before(year, month, offset);
            NaiveDate::from_ymd_opt(y, m, 1).expect("a valid first-of-month date")
        })
        .collect()
}

/// The signed-in user's money flow: the 12-calendar-month trend ending at
/// `to`'s month, plus the `[from, to]` income/expense/net totals (identical to
/// [`income_expense::report`]'s totals, reused rather than recomputed).
pub async fn report(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    display_currency_code: &str,
    from: &str,
    to: &str,
) -> Result<Report, MoneyFlowError> {
    let code = currency::validate_alphabetic_code(display_currency_code)?;
    let (_, to_date) = income_expense::parse_period(from, to)?;
    let display = display_currency(pool, &code).await?;

    let totals =
        income_expense::report(pool, cache, user_id, display_currency_code, from, to, None).await?;

    let months = trailing_months(to_date);
    let window_start = months[0];
    let dated = dated_flows(pool, user_id, window_start, to_date).await?;
    let (month_flows, trend_complete) = value_and_bucket(cache, &display, &months, &dated).await?;

    Ok(Report {
        display,
        months: month_flows,
        total_income: totals.total_income,
        total_expense: totals.total_expense,
        net: totals.net,
        complete: totals.complete && trend_complete,
    })
}

/// The `assets` / `fiat_assets` detail for one active currency, by code.
///
/// Same query as `crate::server::net_worth`/`crate::server::income_expense`'s
/// helper of the same name; a shared helper would remove the duplication but
/// is a refactor beyond this change.
async fn display_currency(pool: &PgPool, code: &str) -> Result<DisplayCurrency, MoneyFlowError> {
    let record = sqlx::query!(
        r#"
        SELECT
            a.id,
            a.code AS alphabetic_code,
            f.minor_units AS "minor_units!"
        FROM assets AS a
        INNER JOIN fiat_assets AS f ON f.asset_id = a.id
        WHERE
            a.asset_class = 'fiat'
            AND a.is_active = TRUE
            AND a.deleted_at IS NULL
            AND a.code = $1
        "#,
        code,
    )
    .fetch_optional(pool)
    .await?;

    let record = record.ok_or(MoneyFlowError::CurrencyNotFound)?;

    Ok(DisplayCurrency {
        asset_id: record.id,
        alphabetic_code: record.alphabetic_code,
        minor_units: record.minor_units,
    })
}

/// One `(account currency, booking date)` day's income and expense subtotals,
/// unconverted.
struct DatedFlow {
    account_asset_id: Uuid,
    account_currency_code: String,
    booking_date: NaiveDate,
    /// Sum of that day's positive account-currency amounts; `None` if none.
    income_subtotal: Option<BigDecimal>,
    /// Sum of that day's negative account-currency amounts; `None` if none.
    expense_subtotal: Option<BigDecimal>,
    /// Any transaction that day is still pending its booking-date conversion.
    pending: bool,
}

/// The user's non-transfer transactions in `[window_start, window_end]`,
/// summed per `(account currency, booking date)`, split into that day's
/// income and expense subtotals. Transfer legs are excluded with the same
/// `NOT EXISTS` anti-join `income_expense::dated_subtotals` uses.
async fn dated_flows(
    pool: &PgPool,
    user_id: Uuid,
    window_start: NaiveDate,
    window_end: NaiveDate,
) -> Result<Vec<DatedFlow>, MoneyFlowError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            acc.default_asset_id AS "account_asset_id!",
            acc_ccy.code AS "account_currency_code!",
            t.booking_date,
            sum(t.account_amount) FILTER (WHERE t.account_amount > 0) AS "income_subtotal?",
            sum(t.account_amount) FILTER (WHERE t.account_amount < 0) AS "expense_subtotal?",
            bool_or(t.account_amount IS NULL) AS "pending!"
        FROM transactions AS t
        INNER JOIN accounts AS acc ON acc.user_id = t.user_id AND acc.id = t.account_id
        INNER JOIN assets AS acc_ccy ON acc_ccy.id = acc.default_asset_id
        WHERE
            t.user_id = $1
            AND t.deleted_at IS NULL
            AND acc.deleted_at IS NULL
            AND t.booking_date >= $2
            AND t.booking_date <= $3
            AND NOT EXISTS (
                SELECT 1
                FROM transfers AS tr
                WHERE
                    tr.user_id = t.user_id
                    AND (
                        tr.source_transaction_id = t.id
                        OR tr.destination_transaction_id = t.id
                    )
                    AND tr.deleted_at IS NULL
            )
        GROUP BY acc.default_asset_id, acc_ccy.code, t.booking_date
        ORDER BY t.booking_date
        "#,
        user_id,
        window_start,
        window_end,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| DatedFlow {
            account_asset_id: row.account_asset_id,
            account_currency_code: row.account_currency_code,
            booking_date: row.booking_date,
            income_subtotal: row.income_subtotal,
            expense_subtotal: row.expense_subtotal,
            pending: row.pending,
        })
        .collect())
}

/// Value every dated flow into the display currency (one rate lookup per
/// `(asset, day)`, cached and reused for both its income and expense parts),
/// then bucket the results into `months`. Returns the bucketed months (in the
/// same order as `months`) and whether every bucket was fully valued.
async fn value_and_bucket(
    cache: &FxRateCache,
    display: &DisplayCurrency,
    months: &[NaiveDate],
    dated: &[DatedFlow],
) -> Result<(Vec<MonthFlow>, bool), MoneyFlowError> {
    let mut buckets: HashMap<(i32, u32), (BigDecimal, BigDecimal)> = months
        .iter()
        .map(|m| (year_month(*m), (BigDecimal::zero(), BigDecimal::zero())))
        .collect();
    let mut complete = true;
    let mut day_rates: HashMap<(Uuid, NaiveDate), Option<ResolvedRate>> = HashMap::new();

    for row in dated {
        let key = year_month(row.booking_date);
        let Some(bucket) = buckets.get_mut(&key) else {
            // Outside the 12-month window (shouldn't happen given the query's
            // own date bounds, but never silently fold it into another month).
            continue;
        };

        if row.pending {
            complete = false;
            continue;
        }

        let resolved = if row.account_asset_id == display.asset_id {
            None
        } else {
            let rate_key = (row.account_asset_id, row.booking_date);
            let cached = match day_rates.get(&rate_key) {
                Some(cached) => cached.clone(),
                None => {
                    let fresh = match rates::resolve_rate_as_of(
                        cache,
                        &row.account_currency_code,
                        &display.alphabetic_code,
                        row.booking_date,
                    )
                    .await
                    {
                        Ok(rate) => Some(rate),
                        Err(RateResolutionError::Unavailable) => None,
                        Err(RateResolutionError::Internal) => return Err(MoneyFlowError::Internal),
                    };
                    day_rates.insert(rate_key, fresh.clone());
                    fresh
                }
            };
            if cached.is_none() {
                complete = false;
                continue;
            }
            cached
        };

        let value = |amount: &BigDecimal| -> Result<BigDecimal, MoneyFlowError> {
            Ok(match &resolved {
                None => amount.clone(),
                Some(rate) => {
                    conversion::convert(
                        amount,
                        rate.rate.clone(),
                        row.account_asset_id,
                        display.asset_id,
                        rate.valuation_timestamp,
                        display.minor_units,
                        MONEY_FLOW_ROUNDING,
                    )?
                    .converted_amount
                }
            })
        };

        if let Some(amount) = &row.income_subtotal {
            bucket.0 += value(amount)?;
        }
        if let Some(amount) = &row.expense_subtotal {
            bucket.1 += value(amount)?;
        }
    }

    let month_flows = months
        .iter()
        .map(|m| {
            let (income, expense) = buckets
                .remove(&year_month(*m))
                .unwrap_or_else(|| (BigDecimal::zero(), BigDecimal::zero()));
            MonthFlow {
                month: *m,
                income,
                expense,
            }
        })
        .collect();

    Ok((month_flows, complete))
}

#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{
        create_user, currency_id, date, dec, insert_transaction, insert_transfer,
    };

    /// A rate cache with no HTTP reach — every fixture below is single-currency,
    /// so no rate is ever resolved.
    fn fx_cache() -> Arc<FxRateCache> {
        FxRateCache::new().expect("build fx cache")
    }

    #[sqlx::test]
    async fn a_transaction_lands_in_its_own_month_bucket(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");

        insert_transaction(&pool, alice, account.id, eur, "100", date(2026, 3, 15)).await;
        insert_transaction(&pool, alice, account.id, eur, "-40", date(2026, 3, 20)).await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-03-01", "2026-03-31")
            .await
            .expect("report");

        let march = report
            .months
            .iter()
            .find(|m| m.month == date(2026, 3, 1))
            .expect("march bucket");
        assert_eq!(march.income, dec("100"));
        assert_eq!(march.expense, dec("-40"));

        // Every other month in the trailing window stays at zero.
        let other_total = report
            .months
            .iter()
            .filter(|m| m.month != date(2026, 3, 1))
            .fold(BigDecimal::zero(), |acc, m| acc + &m.income + &m.expense);
        assert_eq!(other_total, dec("0"));

        // The totals are `income_expense::report`'s own totals: both
        // transactions are uncategorised, so they net to one group (100 - 40
        // = 60) placed entirely on the income side by that group's sign —
        // unlike the trend above, which splits by transaction sign per day,
        // not by category net.
        assert_eq!(report.total_income, dec("60"));
        assert_eq!(report.total_expense, dec("0"));
        assert!(report.complete);
    }

    #[sqlx::test]
    async fn transfer_legs_are_excluded_from_the_trend(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("account");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("account");

        let source =
            insert_transaction(&pool, alice, checking.id, eur, "-300", date(2026, 5, 10)).await;
        let destination =
            insert_transaction(&pool, alice, savings.id, eur, "300", date(2026, 5, 10)).await;
        insert_transfer(&pool, alice, source, destination).await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-05-01", "2026-05-31")
            .await
            .expect("report");

        let may = report
            .months
            .iter()
            .find(|m| m.month == date(2026, 5, 1))
            .expect("may bucket");
        assert_eq!(may.income, dec("0"));
        assert_eq!(may.expense, dec("0"));
    }

    #[sqlx::test]
    async fn the_trend_covers_12_months_ending_at_tos_month(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");

        // A year before `to` — outside the 12-month window ending at 2026-06.
        insert_transaction(&pool, alice, account.id, eur, "999", date(2025, 5, 1)).await;
        // The window's oldest month (11 months before 2026-06 is 2025-07).
        insert_transaction(&pool, alice, account.id, eur, "50", date(2025, 7, 15)).await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-06-01", "2026-06-30")
            .await
            .expect("report");

        assert_eq!(report.months.len(), 12);
        assert_eq!(report.months[0].month, date(2025, 7, 1));
        assert_eq!(report.months[0].income, dec("50"));
        assert_eq!(report.months[11].month, date(2026, 6, 1));

        let total_income = report
            .months
            .iter()
            .fold(BigDecimal::zero(), |acc, m| acc + &m.income);
        // Only the in-window transaction (50) is counted; the 999 one, a year
        // before `to`, falls outside the trailing 12-month window.
        assert_eq!(total_income, dec("50"));
    }

    #[sqlx::test]
    async fn an_unknown_display_currency_is_rejected(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let denied = report(&pool, &fx_cache(), alice, "ZZZ", "2026-01-01", "2026-01-31").await;
        assert!(matches!(denied, Err(MoneyFlowError::CurrencyNotFound)));
    }
}
