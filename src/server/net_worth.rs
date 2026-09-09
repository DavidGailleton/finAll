//! The signed-in user's net worth: the sum of every account balance, valued in
//! one chosen display currency, with a per-currency breakdown.
//!
//! `account_balances` (the SQL view) gives the exact, unconverted balance of
//! each currency an account holds. This module re-aggregates those rows across
//! all of a user's accounts into one signed position per currency, values each
//! position in the display currency using a rate resolved by
//! [`crate::server::assets::rates`], and sums them.
//!
//! Unlike [`crate::server::balances`], a currency with no resolvable rate does
//! not fail the whole request: it becomes an unvalued line and is left out of
//! the total, with the report marked incomplete. Nothing is silently combined
//! or dropped — the excluded currencies stay visible in the breakdown.
//!
//! Every query is scoped by `user_id` so one user can never read another's
//! balances.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, RoundingMode, Zero};
use leptos::logging;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::assets::conversion::{self, ConversionError};
use crate::server::assets::currency::{self, CurrencyError};
use crate::server::assets::rates::{self, RateResolutionError, ResolvedRate};

/// Rounding applied when a currency position is converted into the display
/// currency, to that currency's minor-unit precision (inside
/// [`conversion::convert`]). Deliberately the same policy as
/// `crate::server::balances`' balance rounding — banker's rounding.
const NET_WORTH_ROUNDING: RoundingMode = RoundingMode::HalfEven;

#[derive(Debug, thiserror::Error)]
pub enum NetWorthError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("that display currency does not exist")]
    CurrencyNotFound,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for NetWorthError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("net worth: database error: {err}");
        NetWorthError::Internal
    }
}

impl From<CurrencyError> for NetWorthError {
    fn from(err: CurrencyError) -> Self {
        match err {
            CurrencyError::InvalidInput(message) => NetWorthError::InvalidInput(message),
            CurrencyError::NotFound => NetWorthError::CurrencyNotFound,
            _ => NetWorthError::Internal,
        }
    }
}

impl From<ConversionError> for NetWorthError {
    fn from(err: ConversionError) -> Self {
        // `convert` only rejects caller-supplied inputs; here every one of them
        // is server-controlled, so a rejection is a bug, not user error.
        logging::error!("net worth: conversion rejected a server-built input: {err}");
        NetWorthError::Internal
    }
}

// `RateResolutionError` is intentionally not given a `From` impl: the
// orchestrator matches on it so that `Unavailable` degrades to an unvalued line
// while `Internal` fails the request.

/// The currency the report is expressed in.
pub struct DisplayCurrency {
    pub asset_id: Uuid,
    pub alphabetic_code: String,
    pub minor_units: i16,
}

/// One signed position: the sum of the user's balances in a single currency
/// across every account (`account_balances` re-aggregated by asset).
pub struct Position {
    pub asset_id: Uuid,
    pub currency_code: String,
    pub balance: BigDecimal,
}

/// One valued breakdown line, before it is rendered to strings for the DTO.
pub struct ValuedLine {
    pub currency_code: String,
    pub amount: BigDecimal,
    pub converted_amount: Option<BigDecimal>,
    pub rate: Option<BigDecimal>,
    pub valuation_timestamp: Option<DateTime<Utc>>,
    pub is_display_currency: bool,
}

/// The outcome of valuing a set of positions into the display currency.
pub struct PositionValuation {
    pub lines: Vec<ValuedLine>,
    pub total: BigDecimal,
    pub complete: bool,
    pub rates_as_of: Option<DateTime<Utc>>,
}

/// The whole report: the display currency plus its valued breakdown.
pub struct ValuedReport {
    pub display: DisplayCurrency,
    pub lines: Vec<ValuedLine>,
    pub total: BigDecimal,
    pub complete: bool,
    pub rates_as_of: Option<DateTime<Utc>>,
}

/// Value every `position` into `display`, reusing the pre-resolved rates in
/// `resolved`.
///
/// The position already in the display currency contributes its exact balance,
/// unrounded. Any other position is converted with the rate in `resolved` for
/// its `asset_id`, rounded to `display.minor_units` with [`NET_WORTH_ROUNDING`].
/// A position with no entry in `resolved` becomes a line with
/// `converted_amount = None`, flips `complete` to `false`, and is excluded from
/// the total (never silently folded in). Lines keep the order of `positions`.
pub fn value_positions(
    positions: &[Position],
    display: &DisplayCurrency,
    resolved: &HashMap<Uuid, ResolvedRate>,
) -> Result<PositionValuation, NetWorthError> {
    let mut lines = Vec::with_capacity(positions.len());
    let mut total = BigDecimal::zero();
    let mut complete = true;
    let mut rates_as_of: Option<DateTime<Utc>> = None;

    for position in positions {
        if position.asset_id == display.asset_id {
            total += &position.balance;
            lines.push(ValuedLine {
                currency_code: position.currency_code.clone(),
                amount: position.balance.clone(),
                converted_amount: Some(position.balance.clone()),
                rate: None,
                valuation_timestamp: None,
                is_display_currency: true,
            });
            continue;
        }

        let Some(resolved_rate) = resolved.get(&position.asset_id) else {
            complete = false;
            lines.push(ValuedLine {
                currency_code: position.currency_code.clone(),
                amount: position.balance.clone(),
                converted_amount: None,
                rate: None,
                valuation_timestamp: None,
                is_display_currency: false,
            });
            continue;
        };

        let converted = conversion::convert(
            &position.balance,
            resolved_rate.rate.clone(),
            position.asset_id,
            display.asset_id,
            resolved_rate.valuation_timestamp,
            display.minor_units,
            NET_WORTH_ROUNDING,
        )?;

        total += &converted.converted_amount;
        rates_as_of = Some(match rates_as_of {
            Some(current) => current.min(resolved_rate.valuation_timestamp),
            None => resolved_rate.valuation_timestamp,
        });

        lines.push(ValuedLine {
            currency_code: position.currency_code.clone(),
            amount: position.balance.clone(),
            converted_amount: Some(converted.converted_amount),
            rate: Some(resolved_rate.rate.clone()),
            valuation_timestamp: Some(resolved_rate.valuation_timestamp),
            is_display_currency: false,
        });
    }

    Ok(PositionValuation {
        lines,
        total,
        complete,
        rates_as_of,
    })
}

/// The signed-in user's net worth in `display_currency_code`.
///
/// Fails only when the display currency is unknown or a database/rate lookup
/// errors internally; a merely missing rate yields an incomplete report.
pub async fn report(
    pool: &PgPool,
    user_id: Uuid,
    display_currency_code: &str,
) -> Result<ValuedReport, NetWorthError> {
    let code = currency::validate_alphabetic_code(display_currency_code)?;
    let display = display_currency(pool, &code).await?;
    let positions = positions_for_user(pool, user_id).await?;

    let mut resolved: HashMap<Uuid, ResolvedRate> = HashMap::new();
    for position in &positions {
        if position.asset_id == display.asset_id || resolved.contains_key(&position.asset_id) {
            continue;
        }
        match rates::resolve_rate(pool, position.asset_id, display.asset_id).await {
            Ok(rate) => {
                resolved.insert(position.asset_id, rate);
            }
            // Left out on purpose: `value_positions` renders an unvalued line
            // for this currency and marks the report incomplete.
            Err(RateResolutionError::Unavailable) => {}
            Err(RateResolutionError::Internal) => return Err(NetWorthError::Internal),
        }
    }

    let valuation = value_positions(&positions, &display, &resolved)?;

    Ok(ValuedReport {
        display,
        lines: valuation.lines,
        total: valuation.total,
        complete: valuation.complete,
        rates_as_of: valuation.rates_as_of,
    })
}

/// The `assets` / `fiat_assets` detail for one active currency, by code.
async fn display_currency(pool: &PgPool, code: &str) -> Result<DisplayCurrency, NetWorthError> {
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

    let record = record.ok_or(NetWorthError::CurrencyNotFound)?;

    Ok(DisplayCurrency {
        asset_id: record.id,
        alphabetic_code: record.alphabetic_code,
        minor_units: record.minor_units,
    })
}

/// The user's `account_balances` rows re-aggregated across all their accounts
/// into one signed position per currency, ordered by currency code.
///
/// `INNER JOIN fiat_assets` cannot drop a position today because
/// `assets.asset_class` is constrained to `'fiat'`; a future non-fiat asset
/// class would need a `LEFT JOIN` here plus an unvalued line for it.
async fn positions_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Position>, NetWorthError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            ab.asset_id AS "asset_id!",
            a.code AS "currency_code!",
            sum(ab.balance) AS "balance!"
        FROM account_balances AS ab
        INNER JOIN assets AS a ON a.id = ab.asset_id
        INNER JOIN fiat_assets AS f ON f.asset_id = ab.asset_id
        WHERE ab.user_id = $1
        GROUP BY ab.asset_id, a.code
        ORDER BY a.code
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Position {
            asset_id: row.asset_id,
            currency_code: row.currency_code,
            balance: row.balance,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn asset(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid fixed timestamp")
    }

    fn eur() -> DisplayCurrency {
        DisplayCurrency {
            asset_id: asset(1),
            alphabetic_code: "EUR".to_owned(),
            minor_units: 2,
        }
    }

    fn rate(value: &str, secs: i64) -> ResolvedRate {
        ResolvedRate {
            rate: dec(value),
            valuation_timestamp: at(secs),
        }
    }

    fn position(asset_n: u128, code: &str, balance: &str) -> Position {
        Position {
            asset_id: asset(asset_n),
            currency_code: code.to_owned(),
            balance: dec(balance),
        }
    }

    #[test]
    fn no_positions_is_a_zero_complete_report() {
        let valuation = value_positions(&[], &eur(), &HashMap::new()).expect("values");
        assert!(valuation.lines.is_empty());
        assert_eq!(valuation.total, dec("0"));
        assert!(valuation.complete);
        assert_eq!(valuation.rates_as_of, None);
    }

    #[test]
    fn a_display_currency_position_is_exact_and_unrounded() {
        let positions = [position(1, "EUR", "10.005")];
        let valuation = value_positions(&positions, &eur(), &HashMap::new()).expect("values");

        assert_eq!(valuation.total, dec("10.005"));
        assert!(valuation.complete);
        let line = &valuation.lines[0];
        assert!(line.is_display_currency);
        assert_eq!(line.converted_amount, Some(dec("10.005")));
        assert_eq!(line.rate, None);
        assert_eq!(line.valuation_timestamp, None);
    }

    #[test]
    fn a_zero_display_currency_position_still_produces_a_line() {
        let positions = [position(1, "EUR", "0")];
        let valuation = value_positions(&positions, &eur(), &HashMap::new()).expect("values");

        assert_eq!(valuation.lines.len(), 1);
        assert_eq!(valuation.total, dec("0"));
    }

    #[test]
    fn a_foreign_position_is_converted_and_rounded_to_minor_units() {
        let positions = [position(2, "USD", "100")];
        let resolved = HashMap::from([(asset(2), rate("1.234", 1_700_000_000))]);
        let valuation = value_positions(&positions, &eur(), &resolved).expect("values");

        // 100 * 1.234 = 123.400, rounded to 2 minor units.
        assert_eq!(valuation.total, dec("123.40"));
        let line = &valuation.lines[0];
        assert_eq!(line.converted_amount, Some(dec("123.40")));
        assert_eq!(line.rate, Some(dec("1.234")));
        assert_eq!(line.valuation_timestamp, Some(at(1_700_000_000)));
        assert!(valuation.complete);
    }

    #[test]
    fn a_negative_foreign_position_keeps_its_sign() {
        let positions = [position(2, "USD", "-50")];
        let resolved = HashMap::from([(asset(2), rate("2", 1_700_000_000))]);
        let valuation = value_positions(&positions, &eur(), &resolved).expect("values");

        assert_eq!(valuation.total, dec("-100.00"));
    }

    #[test]
    fn display_and_foreign_positions_are_summed() {
        let positions = [
            position(1, "EUR", "10"),
            position(2, "USD", "100"),
            position(3, "GBP", "5"),
        ];
        let resolved = HashMap::from([
            (asset(2), rate("1.25", 1_700_000_000)),
            (asset(3), rate("0.5", 1_700_000_000)),
        ]);
        let valuation = value_positions(&positions, &eur(), &resolved).expect("values");

        // 10 + (100 * 1.25 -> 125.00) + (5 * 0.5 -> 2.50)
        assert_eq!(valuation.total, dec("137.50"));
        assert!(valuation.complete);
    }

    #[test]
    fn a_missing_rate_makes_an_unvalued_line_and_an_incomplete_report() {
        let positions = [position(1, "EUR", "10"), position(2, "USD", "100")];
        let valuation = value_positions(&positions, &eur(), &HashMap::new()).expect("values");

        // The EUR position is still counted; USD is not.
        assert_eq!(valuation.total, dec("10"));
        assert!(!valuation.complete);
        let usd = valuation
            .lines
            .iter()
            .find(|line| line.currency_code == "USD")
            .expect("a USD line");
        assert_eq!(usd.converted_amount, None);
        assert_eq!(usd.rate, None);
    }

    #[test]
    fn rates_as_of_is_the_earliest_converted_timestamp() {
        let positions = [
            position(2, "USD", "1"),
            position(3, "GBP", "1"),
            position(4, "CHF", "1"),
        ];
        let resolved = HashMap::from([
            (asset(2), rate("1", 1_700_000_200)),
            (asset(3), rate("1", 1_700_000_100)),
            (asset(4), rate("1", 1_700_000_300)),
        ]);
        let valuation = value_positions(&positions, &eur(), &resolved).expect("values");

        assert_eq!(valuation.rates_as_of, Some(at(1_700_000_100)));
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{create_user, currency_id, date, dec, insert_transaction};

    #[sqlx::test]
    async fn a_currency_is_summed_across_the_users_accounts(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let cash = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's cash account");
        let bank = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("alice's bank account");

        insert_transaction(&pool, alice, cash.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, bank.id, eur, "25.50", date(2026, 1, 16)).await;

        let report = report(&pool, alice, "EUR").await.expect("a report");

        assert_eq!(report.lines.len(), 1);
        assert_eq!(report.total, dec("125.50"));
        assert!(report.complete);
        assert_eq!(report.rates_as_of, None);
    }

    #[sqlx::test]
    async fn another_users_balances_are_never_included(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let alices = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("bob's account");

        insert_transaction(&pool, alice, alices.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, bob, bobs.id, eur, "777.00", date(2026, 1, 15)).await;

        let report = report(&pool, alice, "EUR").await.expect("a report");
        assert_eq!(report.total, dec("100.00"));
    }

    #[sqlx::test]
    async fn a_user_with_no_transactions_has_an_empty_report(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let report = report(&pool, alice, "EUR").await.expect("a report");
        assert!(report.lines.is_empty());
        assert_eq!(report.total, dec("0"));
        assert!(report.complete);
    }

    #[sqlx::test]
    async fn an_unknown_display_currency_is_rejected(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;

        let denied = report(&pool, alice, "ZZZ").await;
        assert!(matches!(denied, Err(NetWorthError::CurrencyNotFound)));
    }

    #[sqlx::test]
    async fn a_single_currency_report_needs_no_rates(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        insert_transaction(&pool, alice, account.id, eur, "42.00", date(2026, 1, 15)).await;

        // `asset_rates` is empty in a fresh test database.
        let report = report(&pool, alice, "EUR").await.expect("a report");
        assert!(report.complete);
        assert_eq!(report.total, dec("42.00"));
    }
}
