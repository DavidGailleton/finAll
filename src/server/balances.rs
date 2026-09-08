//! The converted single-number balance of one account.
//!
//! `account_balances` (the SQL view) gives the exact, unconverted balance for
//! each currency an account holds. This module values every one of those
//! sub-balances in the account's default currency, using a rate resolved by
//! [`crate::server::assets::rates`], and sums them. A missing rate fails the
//! whole request rather than producing a partial figure.
//!
//! Every query is scoped by `user_id` so one user can never read another's
//! balance.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, RoundingMode, Zero};
use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::accounts::{self, AccountError};
use crate::server::assets::conversion::{self, ConversionError};
use crate::server::assets::rates::{self, RateResolutionError, ResolvedRate};

/// Rounding applied when a sub-balance is converted into the default currency,
/// to that currency's minor-unit precision (inside [`conversion::convert`]).
const BALANCE_ROUNDING: RoundingMode = RoundingMode::HalfEven;

#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("no exchange rate available to value this account")]
    RateUnavailable,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for BalanceError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("balances: database error: {err}");
        BalanceError::Internal
    }
}

impl From<AccountError> for BalanceError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::Unauthorized => BalanceError::Unauthorized,
            AccountError::NotFound => BalanceError::NotFound,
            _ => BalanceError::Internal,
        }
    }
}

impl From<RateResolutionError> for BalanceError {
    fn from(err: RateResolutionError) -> Self {
        match err {
            RateResolutionError::Unavailable => BalanceError::RateUnavailable,
            RateResolutionError::Internal => BalanceError::Internal,
        }
    }
}

impl From<ConversionError> for BalanceError {
    fn from(err: ConversionError) -> Self {
        // `convert` only rejects caller-supplied inputs; here every one of them
        // is server-controlled, so a rejection is a bug, not user error.
        logging::error!("balances: conversion rejected a server-built input: {err}");
        BalanceError::Internal
    }
}

/// One `account_balances` row: the exact balance of a single currency the
/// account holds.
pub struct AccountBalanceRow {
    pub asset_id: Uuid,
    pub balance: BigDecimal,
}

/// The account's default currency — what its total is expressed in.
pub struct DefaultCurrency {
    pub asset_id: Uuid,
    pub alphabetic_code: String,
    pub minor_units: i16,
}

/// An account's balance as one number in its default currency.
pub struct TotalBalance {
    pub amount: BigDecimal,
    pub alphabetic_code: String,
}

/// Sum `rows` into `default`'s currency.
///
/// A row already in the default currency contributes its exact balance,
/// unrounded. Any other row is converted with the pre-resolved rate for its
/// `asset_id` in `resolved`; a row with no entry there is
/// [`BalanceError::RateUnavailable`]. No `rows` means a zero balance (an
/// account with no transactions).
pub fn sum_in_default_currency(
    rows: &[AccountBalanceRow],
    default: &DefaultCurrency,
    resolved: &HashMap<Uuid, ResolvedRate>,
) -> Result<BigDecimal, BalanceError> {
    let mut total = BigDecimal::zero();

    for row in rows {
        if row.asset_id == default.asset_id {
            total += &row.balance;
            continue;
        }

        let rate = resolved
            .get(&row.asset_id)
            .ok_or(BalanceError::RateUnavailable)?;

        let converted = conversion::convert(
            &row.balance,
            rate.rate.clone(),
            row.asset_id,
            default.asset_id,
            rate.valuation_timestamp,
            default.minor_units,
            BALANCE_ROUNDING,
        )?;

        total += converted.converted_amount;
    }

    Ok(total)
}

/// The current user's balance for one account, valued in the account's default
/// currency. Fails if any currency the account holds cannot be valued.
pub async fn account_total(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<TotalBalance, BalanceError> {
    let account = accounts::find_for_user(pool, user_id, account_id).await?;
    let default = default_currency(pool, account.default_asset_id).await?;
    let rows = balance_rows(pool, user_id, account_id).await?;

    let mut resolved: HashMap<Uuid, ResolvedRate> = HashMap::new();
    for row in &rows {
        if row.asset_id == default.asset_id || resolved.contains_key(&row.asset_id) {
            continue;
        }
        let rate = rates::resolve_rate(pool, row.asset_id, default.asset_id).await?;
        resolved.insert(row.asset_id, rate);
    }

    let amount = sum_in_default_currency(&rows, &default, &resolved)?;

    Ok(TotalBalance {
        amount,
        alphabetic_code: default.alphabetic_code,
    })
}

/// The `assets` / `fiat_assets` detail for one asset id.
async fn default_currency(pool: &PgPool, asset_id: Uuid) -> Result<DefaultCurrency, BalanceError> {
    let record = sqlx::query!(
        r#"
        SELECT
            a.id,
            a.code AS alphabetic_code,
            f.minor_units AS "minor_units!"
        FROM assets AS a
        INNER JOIN fiat_assets AS f ON f.asset_id = a.id
        WHERE a.id = $1
        "#,
        asset_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(DefaultCurrency {
        asset_id: record.id,
        alphabetic_code: record.alphabetic_code,
        minor_units: record.minor_units,
    })
}

/// The `account_balances` rows for one of the user's accounts.
async fn balance_rows(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<Vec<AccountBalanceRow>, BalanceError> {
    let rows = sqlx::query_as!(
        AccountBalanceRow,
        r#"
        SELECT
            asset_id AS "asset_id!",
            balance AS "balance!"
        FROM account_balances
        WHERE user_id = $1 AND account_id = $2
        "#,
        user_id,
        account_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use sqlx::types::chrono::{DateTime, Utc};

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn asset(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn ts() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid fixed timestamp")
    }

    fn eur() -> DefaultCurrency {
        DefaultCurrency {
            asset_id: asset(1),
            alphabetic_code: "EUR".to_owned(),
            minor_units: 2,
        }
    }

    fn rate(value: &str) -> ResolvedRate {
        ResolvedRate {
            rate: dec(value),
            valuation_timestamp: ts(),
        }
    }

    fn row(asset_n: u128, balance: &str) -> AccountBalanceRow {
        AccountBalanceRow {
            asset_id: asset(asset_n),
            balance: dec(balance),
        }
    }

    #[test]
    fn no_rows_is_a_zero_balance() {
        let total = sum_in_default_currency(&[], &eur(), &HashMap::new()).expect("sums");
        assert_eq!(total, dec("0"));
    }

    #[test]
    fn a_single_default_currency_row_is_exact_and_unrounded() {
        let rows = [row(1, "10.005")];
        let total = sum_in_default_currency(&rows, &eur(), &HashMap::new()).expect("sums");
        assert_eq!(total, dec("10.005"));
    }

    #[test]
    fn a_zero_default_currency_row_still_contributes() {
        let rows = [row(1, "0")];
        let total = sum_in_default_currency(&rows, &eur(), &HashMap::new()).expect("sums");
        assert_eq!(total, dec("0"));
    }

    #[test]
    fn a_foreign_row_is_converted_and_rounded_to_minor_units() {
        let rows = [row(2, "100")];
        let resolved = HashMap::from([(asset(2), rate("1.234"))]);
        let total = sum_in_default_currency(&rows, &eur(), &resolved).expect("sums");
        // 100 * 1.234 = 123.400, rounded to 2 minor units.
        assert_eq!(total, dec("123.40"));
    }

    #[test]
    fn a_negative_foreign_balance_keeps_its_sign() {
        let rows = [row(2, "-50")];
        let resolved = HashMap::from([(asset(2), rate("2"))]);
        let total = sum_in_default_currency(&rows, &eur(), &resolved).expect("sums");
        assert_eq!(total, dec("-100.00"));
    }

    #[test]
    fn default_and_foreign_rows_are_summed() {
        let rows = [row(1, "10"), row(2, "100"), row(3, "5")];
        let resolved = HashMap::from([(asset(2), rate("1.25")), (asset(3), rate("0.5"))]);
        let total = sum_in_default_currency(&rows, &eur(), &resolved).expect("sums");
        // 10 + (100 * 1.25 -> 125.00) + (5 * 0.5 -> 2.50)
        assert_eq!(total, dec("137.50"));
    }

    #[test]
    fn a_missing_rate_fails_the_whole_sum() {
        let rows = [row(1, "10"), row(2, "100")];
        let result = sum_in_default_currency(&rows, &eur(), &HashMap::new());
        assert!(matches!(result, Err(BalanceError::RateUnavailable)));
    }
}
