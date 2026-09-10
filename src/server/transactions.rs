//! Queries for a signed-in user's transactions (`transactions` joined to
//! `assets` for the currency code, `accounts` for the account name, and
//! `categories` / `merchants` for display names), the create / edit /
//! soft-delete writes, the input validation for the transaction server
//! functions, and the domain error.
//!
//! Every value arriving from a server function is untrusted. `validate_amount`,
//! `validate_booking_date`, `validate_value_date` and `validate_date_range`
//! parse the raw fields; `resolve_dates` then applies the cross-field rules (a
//! cash account keeps one date; a value date may not precede the booking date).
//! Every query is scoped by `user_id` so one user can never read or change
//! another's transactions.
//!
//! A foreign-currency transaction is also recorded in its account's currency
//! (`account_amount`), translated once at the FX rate on its booking date;
//! `backfill_account_amounts` fills in any left pending because the rate source
//! was unreachable at write time.

use std::str::FromStr;

use bigdecimal::{RoundingMode, Zero};
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

use crate::server::assets::conversion;
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::assets::rates;

/// Rounding applied when a foreign-currency transaction is translated into its
/// account's currency, to that currency's minor-unit precision (inside
/// [`conversion::convert`]). The same banker's-rounding policy as the report
/// modules.
const TRANSACTION_FX_ROUNDING: RoundingMode = RoundingMode::HalfEven;

/// The largest number of fractional digits `transactions.amount` can hold
/// (`NUMERIC(38, 18)`).
const AMOUNT_SCALE: i64 = 18;

/// The largest number of integer digits `transactions.amount` can hold
/// (`NUMERIC(38, 18)` leaves 38 - 18 for the integer part).
const AMOUNT_INTEGER_DIGITS: i64 = 20;

/// How many transactions one page of [`list`] holds.
pub const PAGE_SIZE: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("transaction not found")]
    NotFound,

    #[error("the selected currency does not exist")]
    UnknownAsset,

    #[error("the selected category does not exist")]
    UnknownCategory,

    #[error("the selected merchant does not exist")]
    UnknownMerchant,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for TransactionError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("transactions: database error: {err}");
        TransactionError::Internal
    }
}

/// The columns needed to render a transaction row to the browser. `amount` is
/// the exact stored decimal; the server function stringifies it.
pub struct TransactionRecord {
    pub id: Uuid,
    pub amount: BigDecimal,
    pub asset_id: Uuid,
    pub asset_code: String,
    pub account_id: Uuid,
    pub account_name: String,
    pub account_type: String,
    pub account_default_asset_id: Uuid,
    pub account_currency_code: String,
    /// The transaction's amount in the account's currency (§ `account_amount`
    /// column). `None` while a foreign-currency conversion is still pending.
    pub account_amount: Option<BigDecimal>,
    /// The rate used for that conversion; `None` when the transaction is already
    /// in the account currency or the conversion is pending.
    pub fx_rate: Option<BigDecimal>,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub merchant_id: Option<Uuid>,
    pub merchant_name: Option<String>,
    /// Set when this transaction is one leg of a (non-deleted) transfer; the
    /// row is then shown as a transfer.
    pub transfer_id: Option<Uuid>,
    /// The name of the account on the *other* leg of that transfer, for display.
    pub transfer_counterparty: Option<String>,
}

/// A keyset cursor: the `(booking_date, id)` of the last row already seen.
/// [`list`] returns rows strictly ordered after it.
pub type Cursor = (NaiveDate, Uuid);

/// The filter for [`list`]. Every field is optional; `from` / `to` bound
/// `booking_date` inclusively.
#[derive(Default)]
pub struct TransactionFilter {
    pub account_id: Option<Uuid>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub after: Option<Cursor>,
}

/// One page of [`list`] results, newest first, plus the cursor for the next
/// page (present only when more rows remain).
pub struct TransactionListPage {
    pub records: Vec<TransactionRecord>,
    pub next: Option<Cursor>,
}

/// One page of the user's non-deleted transactions matching `filter`, newest
/// first (`booking_date` then `id`, both descending). `id` is uuidv7 so it
/// orders by creation time and gives every row a stable total order.
///
/// Transactions on a soft-deleted account are excluded, matching the
/// `account_balances` view.
pub async fn list(
    pool: &PgPool,
    user_id: Uuid,
    filter: TransactionFilter,
) -> Result<TransactionListPage, TransactionError> {
    let (after_date, after_id) = match filter.after {
        Some((date, id)) => (Some(date), Some(id)),
        None => (None, None),
    };
    let limit = PAGE_SIZE as i64 + 1;

    let mut records = sqlx::query_as!(
        TransactionRecord,
        r#"
        SELECT
            t.id,
            t.amount,
            t.asset_id,
            a.code AS "asset_code!",
            t.account_id,
            acc.account_name AS "account_name!",
            acc.account_type AS "account_type!",
            acc.default_asset_id AS "account_default_asset_id!",
            acc_ccy.code AS "account_currency_code!",
            t.account_amount,
            t.fx_rate,
            t.booking_date,
            t.value_date,
            t.category_id,
            c.category_name AS "category_name?",
            t.merchant_id,
            m.merchant_name AS "merchant_name?",
            tr.id AS "transfer_id?",
            other_acc.account_name AS "transfer_counterparty?"
        FROM transactions AS t
        INNER JOIN assets AS a ON a.id = t.asset_id
        INNER JOIN accounts AS acc
            ON acc.user_id = t.user_id AND acc.id = t.account_id
        INNER JOIN assets AS acc_ccy ON acc_ccy.id = acc.default_asset_id
        LEFT JOIN categories AS c
            ON c.user_id = t.user_id AND c.id = t.category_id AND c.deleted_at IS NULL
        LEFT JOIN merchants AS m
            ON m.user_id = t.user_id AND m.id = t.merchant_id AND m.deleted_at IS NULL
        LEFT JOIN transfers AS tr
            ON tr.user_id = t.user_id
            AND (tr.source_transaction_id = t.id OR tr.destination_transaction_id = t.id)
            AND tr.deleted_at IS NULL
        LEFT JOIN transactions AS other
            ON other.id = CASE
                WHEN tr.source_transaction_id = t.id THEN tr.destination_transaction_id
                ELSE tr.source_transaction_id
            END
        LEFT JOIN accounts AS other_acc
            ON other_acc.user_id = t.user_id AND other_acc.id = other.account_id
        WHERE t.user_id = $1
          AND t.deleted_at IS NULL
          AND acc.deleted_at IS NULL
          AND ($2::uuid IS NULL OR t.account_id = $2)
          AND ($3::date IS NULL OR t.booking_date >= $3)
          AND ($4::date IS NULL OR t.booking_date <= $4)
          AND ($5::date IS NULL OR (t.booking_date, t.id) < ($5::date, $6::uuid))
        ORDER BY t.booking_date DESC, t.id DESC
        LIMIT $7
        "#,
        user_id,
        filter.account_id,
        filter.from,
        filter.to,
        after_date,
        after_id,
        limit,
    )
    .fetch_all(pool)
    .await?;

    let next = if records.len() > PAGE_SIZE {
        records.truncate(PAGE_SIZE);
        records
            .last()
            .map(|record| (record.booking_date, record.id))
    } else {
        None
    };

    Ok(TransactionListPage { records, next })
}

/// Encode a keyset [`Cursor`] as an opaque `"YYYY-MM-DD_<uuid>"` string for the
/// browser to hand back verbatim.
pub fn encode_cursor((booking_date, id): Cursor) -> String {
    format!("{booking_date}_{id}")
}

/// Decode a [`Cursor`] produced by [`encode_cursor`]. Any malformed value is
/// rejected rather than trusted.
pub fn parse_cursor(input: &str) -> Result<Cursor, TransactionError> {
    let invalid = || TransactionError::InvalidInput("invalid page cursor");
    let (date, id) = input.split_once('_').ok_or_else(invalid)?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| invalid())?;
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok((date, id))
}

/// Parse and validate a user-supplied `amount` string into an exact decimal.
///
/// Mirrors the `transactions_amount_nonzero` CHECK and the `NUMERIC(38, 18)`
/// column type: the value must be a number, must not be zero, and must fit the
/// column's precision. The sign is preserved exactly as written; nothing is
/// rounded.
pub fn validate_amount(input: &str) -> Result<BigDecimal, TransactionError> {
    let amount = BigDecimal::from_str(input.trim())
        .map_err(|_| TransactionError::InvalidInput("amount must be a number"))?;

    if amount.is_zero() {
        return Err(TransactionError::InvalidInput("amount must not be zero"));
    }

    // `scale` is the count of fractional digits (negative when the value is a
    // multiple of a power of ten); `digits` is the count of significant digits.
    let normalized = amount.normalized();
    let (_, scale) = normalized.as_bigint_and_exponent();
    if scale > AMOUNT_SCALE {
        return Err(TransactionError::InvalidInput(
            "amount has too many decimal places",
        ));
    }
    let integer_digits = i128::from(normalized.digits()) - i128::from(scale);
    if integer_digits > i128::from(AMOUNT_INTEGER_DIGITS) {
        return Err(TransactionError::InvalidInput("amount is too large"));
    }

    Ok(amount)
}

/// Parse a required ISO `YYYY-MM-DD` date (the shape `<input type="date">`
/// submits).
pub fn validate_booking_date(input: &str) -> Result<NaiveDate, TransactionError> {
    NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d")
        .map_err(|_| TransactionError::InvalidInput("booking date is invalid"))
}

/// Parse an optional ISO `YYYY-MM-DD` value date, treating a blank or absent
/// field as "no value date" (mirrors the nullable `value_date` column).
pub fn validate_value_date(input: Option<&str>) -> Result<Option<NaiveDate>, TransactionError> {
    match input.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| TransactionError::InvalidInput("value date is invalid")),
    }
}

/// Parse the `from` / `to` bounds of a date-range filter. A blank or absent
/// bound is "unbounded on that side"; both bounds are inclusive. Rejects a
/// range whose start is after its end.
pub fn validate_date_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>), TransactionError> {
    fn parse(value: &str) -> Result<NaiveDate, TransactionError> {
        NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
            .map_err(|_| TransactionError::InvalidInput("a filter date is invalid"))
    }

    let from = from
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse)
        .transpose()?;
    let to = to
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse)
        .transpose()?;

    if let (Some(start), Some(end)) = (from, to) {
        if start > end {
            return Err(TransactionError::InvalidInput(
                "the start date is after the end date",
            ));
        }
    }

    Ok((from, to))
}

/// Confirm `asset_id` is an active, non-deleted fiat currency (one that
/// `list_currencies` would return), not merely any row the foreign key allows.
/// Mirrors `crate::server::accounts::assert_currency_exists`.
async fn assert_currency_exists(pool: &PgPool, asset_id: Uuid) -> Result<(), TransactionError> {
    let found = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM assets
        WHERE id = $1
          AND asset_class = 'fiat'
          AND is_active = TRUE
          AND deleted_at IS NULL
        "#,
        asset_id,
    )
    .fetch_optional(pool)
    .await?;

    if found.is_none() {
        return Err(TransactionError::UnknownAsset);
    }

    Ok(())
}

/// Confirm `category_id` is one of the user's active (non-deleted) categories,
/// not merely any row the composite foreign key allows. Mirrors
/// [`assert_currency_exists`].
async fn assert_category_exists(
    pool: &PgPool,
    user_id: Uuid,
    category_id: Uuid,
) -> Result<(), TransactionError> {
    let found = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM categories
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        category_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    if found.is_none() {
        return Err(TransactionError::UnknownCategory);
    }

    Ok(())
}

/// Confirm `merchant_id` is one of the user's active (non-deleted) merchants.
/// Mirrors [`assert_category_exists`].
async fn assert_merchant_exists(
    pool: &PgPool,
    user_id: Uuid,
    merchant_id: Uuid,
) -> Result<(), TransactionError> {
    let found = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM merchants
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        merchant_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    if found.is_none() {
        return Err(TransactionError::UnknownMerchant);
    }

    Ok(())
}

/// The mutable fields of a transaction, shared by [`create`] and [`update`] so
/// neither grows a long list of positional arguments (in particular two
/// same-typed `Option<Uuid>` next to each other). The account is not here: a
/// transaction cannot be moved between accounts.
pub struct TransactionWrite {
    pub asset_id: Uuid,
    pub amount: BigDecimal,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub category_id: Option<Uuid>,
    pub merchant_id: Option<Uuid>,
}

/// Check that a `TransactionWrite`'s currency, category, and merchant are all
/// things this user could actually have picked (not merely rows the foreign
/// keys allow, and not soft-deleted ones).
async fn assert_write_targets_exist(
    pool: &PgPool,
    user_id: Uuid,
    write: &TransactionWrite,
) -> Result<(), TransactionError> {
    assert_currency_exists(pool, write.asset_id).await?;
    if let Some(category_id) = write.category_id {
        assert_category_exists(pool, user_id, category_id).await?;
    }
    if let Some(merchant_id) = write.merchant_id {
        assert_merchant_exists(pool, user_id, merchant_id).await?;
    }
    Ok(())
}

/// What the write path needs to know about a transaction's account to record it
/// in the account's currency.
struct AccountFxContext {
    default_asset_id: Uuid,
    is_cash: bool,
    currency_code: String,
    minor_units: i16,
}

/// The [`AccountFxContext`] for one of the user's non-deleted accounts, or
/// [`TransactionError::NotFound`].
async fn account_fx_context(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<AccountFxContext, TransactionError> {
    let row = sqlx::query!(
        r#"
        SELECT
            acc.default_asset_id,
            acc.account_type,
            a.code AS "currency_code!",
            f.minor_units AS "minor_units!"
        FROM accounts AS acc
        INNER JOIN assets AS a ON a.id = acc.default_asset_id
        INNER JOIN fiat_assets AS f ON f.asset_id = acc.default_asset_id
        WHERE acc.id = $1 AND acc.user_id = $2 AND acc.deleted_at IS NULL
        "#,
        account_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(TransactionError::NotFound)?;

    Ok(AccountFxContext {
        default_asset_id: row.default_asset_id,
        is_cash: row.account_type == "cash",
        currency_code: row.currency_code,
        minor_units: row.minor_units,
    })
}

/// The [`AccountFxContext`] for the account of one of the user's non-deleted
/// transactions, or [`TransactionError::NotFound`].
async fn account_fx_context_for_transaction(
    pool: &PgPool,
    user_id: Uuid,
    transaction_id: Uuid,
) -> Result<AccountFxContext, TransactionError> {
    let row = sqlx::query!(
        r#"
        SELECT
            acc.default_asset_id,
            acc.account_type,
            a.code AS "currency_code!",
            f.minor_units AS "minor_units!"
        FROM transactions AS t
        INNER JOIN accounts AS acc ON acc.id = t.account_id
        INNER JOIN assets AS a ON a.id = acc.default_asset_id
        INNER JOIN fiat_assets AS f ON f.asset_id = acc.default_asset_id
        WHERE t.id = $1 AND t.user_id = $2 AND t.deleted_at IS NULL
        "#,
        transaction_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(TransactionError::NotFound)?;

    Ok(AccountFxContext {
        default_asset_id: row.default_asset_id,
        is_cash: row.account_type == "cash",
        currency_code: row.currency_code,
        minor_units: row.minor_units,
    })
}

/// Reconcile a transaction's booking and value dates. A cash account keeps a
/// single date, so the value date is forced equal to the booking date;
/// otherwise a supplied value date must not precede the booking date.
fn resolve_dates(
    booking_date: NaiveDate,
    value_date: Option<NaiveDate>,
    is_cash: bool,
) -> Result<(NaiveDate, Option<NaiveDate>), TransactionError> {
    if is_cash {
        return Ok((booking_date, Some(booking_date)));
    }
    if let Some(value_date) = value_date {
        if value_date < booking_date {
            return Err(TransactionError::InvalidInput(
                "the value date cannot be before the booking date",
            ));
        }
    }
    Ok((booking_date, value_date))
}

/// The alphabetic code of a currency asset already asserted to exist.
async fn currency_code(pool: &PgPool, asset_id: Uuid) -> Result<String, TransactionError> {
    sqlx::query_scalar!("SELECT code FROM assets WHERE id = $1", asset_id)
        .fetch_optional(pool)
        .await?
        .ok_or(TransactionError::UnknownAsset)
}

/// The transaction currency (what `amount` is in).
struct SourceAmount<'a> {
    asset_id: Uuid,
    code: &'a str,
    amount: &'a BigDecimal,
}

/// The account currency (what the transaction is translated into).
struct TargetCurrency<'a> {
    asset_id: Uuid,
    code: &'a str,
    minor_units: i16,
}

/// The `(account_amount, fx_rate, fx_rate_date)` trio for a transaction:
/// - `(Some(amount), None, None)` when it is already in the account's currency;
/// - the converted amount at the `booking_date` rate otherwise;
/// - `(None, None, None)` — "conversion pending" — when that rate cannot be
///   fetched. The write is never blocked; the background backfill fills it later.
async fn compute_conversion(
    cache: &FxRateCache,
    source: SourceAmount<'_>,
    target: TargetCurrency<'_>,
    booking_date: NaiveDate,
) -> Result<(Option<BigDecimal>, Option<BigDecimal>, Option<NaiveDate>), TransactionError> {
    if source.asset_id == target.asset_id {
        return Ok((Some(source.amount.clone()), None, None));
    }

    match rates::resolve_rate_as_of(cache, source.code, target.code, booking_date).await {
        Ok(rate) => {
            let converted = conversion::convert(
                source.amount,
                rate.rate,
                source.asset_id,
                target.asset_id,
                rate.valuation_timestamp,
                target.minor_units,
                TRANSACTION_FX_ROUNDING,
            )
            .map_err(|err| {
                logging::error!("transactions: conversion rejected a server-built input: {err}");
                TransactionError::Internal
            })?;
            Ok((
                Some(converted.converted_amount),
                Some(converted.rate),
                Some(converted.valuation_timestamp.date_naive()),
            ))
        }
        Err(_) => {
            logging::log!(
                "transactions: no {} -> {} rate for {booking_date}; conversion pending",
                source.code,
                target.code,
            );
            Ok((None, None, None))
        }
    }
}

/// Insert a transaction on one of the user's accounts.
///
/// Returns [`TransactionError::NotFound`] if `account_id` is not one of this
/// user's non-deleted accounts, [`TransactionError::UnknownAsset`] if the
/// currency is not valid, and [`TransactionError::UnknownCategory`] /
/// [`TransactionError::UnknownMerchant`] if a supplied `category_id` /
/// `merchant_id` is not one of the user's active rows.
pub async fn create(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    account_id: Uuid,
    write: &TransactionWrite,
) -> Result<Uuid, TransactionError> {
    let account = account_fx_context(pool, user_id, account_id).await?;
    assert_write_targets_exist(pool, user_id, write).await?;

    let (booking_date, value_date) =
        resolve_dates(write.booking_date, write.value_date, account.is_cash)?;
    let source_code = currency_code(pool, write.asset_id).await?;
    let (account_amount, fx_rate, fx_rate_date) = compute_conversion(
        cache,
        SourceAmount {
            asset_id: write.asset_id,
            code: &source_code,
            amount: &write.amount,
        },
        TargetCurrency {
            asset_id: account.default_asset_id,
            code: &account.currency_code,
            minor_units: account.minor_units,
        },
        booking_date,
    )
    .await?;

    let row = match sqlx::query!(
        r#"
        INSERT INTO transactions
            (user_id, account_id, asset_id, amount, booking_date, value_date,
             category_id, merchant_id, account_amount, fx_rate, fx_rate_date)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id
        "#,
        user_id,
        account_id,
        write.asset_id,
        write.amount,
        booking_date,
        value_date,
        write.category_id,
        write.merchant_id,
        account_amount,
        fx_rate,
        fx_rate_date,
    )
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(TransactionError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(row.id)
}

/// Update the user's transaction: its amount, currency, dates, category, and
/// merchant. The account is left unchanged. `category_id` / `merchant_id` are
/// written on every call — `None` clears the column.
///
/// Returns [`TransactionError::NotFound`] if the id is not one of this user's
/// non-deleted transactions, [`TransactionError::UnknownAsset`] if the currency
/// is not valid, and [`TransactionError::UnknownCategory`] /
/// [`TransactionError::UnknownMerchant`] if a supplied `category_id` /
/// `merchant_id` is not one of the user's active rows.
///
/// A transaction that is one leg of a transfer is edited here like any other:
/// the transfer is only a link between the two rows and is left untouched.
pub async fn update(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    id: Uuid,
    write: &TransactionWrite,
) -> Result<(), TransactionError> {
    let account = account_fx_context_for_transaction(pool, user_id, id).await?;
    assert_write_targets_exist(pool, user_id, write).await?;

    let (booking_date, value_date) =
        resolve_dates(write.booking_date, write.value_date, account.is_cash)?;
    let source_code = currency_code(pool, write.asset_id).await?;
    let (account_amount, fx_rate, fx_rate_date) = compute_conversion(
        cache,
        SourceAmount {
            asset_id: write.asset_id,
            code: &source_code,
            amount: &write.amount,
        },
        TargetCurrency {
            asset_id: account.default_asset_id,
            code: &account.currency_code,
            minor_units: account.minor_units,
        },
        booking_date,
    )
    .await?;

    let row = match sqlx::query!(
        r#"
        UPDATE transactions
        SET asset_id = $3,
            amount = $4,
            booking_date = $5,
            value_date = $6,
            category_id = $7,
            merchant_id = $8,
            account_amount = $9,
            fx_rate = $10,
            fx_rate_date = $11,
            updated_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        RETURNING id
        "#,
        id,
        user_id,
        write.asset_id,
        write.amount,
        booking_date,
        value_date,
        write.category_id,
        write.merchant_id,
        account_amount,
        fx_rate,
        fx_rate_date,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(TransactionError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    if row.is_none() {
        return Err(TransactionError::NotFound);
    }

    Ok(())
}

/// Soft-delete the user's transaction by setting `deleted_at`. It then drops
/// out of the `account_balances` view like any other non-existent row.
///
/// If the transaction is one leg of a transfer, that transfer link is removed
/// first (in the same database transaction), leaving the other leg as an
/// ordinary transaction.
///
/// Returns [`TransactionError::NotFound`] if the id is not one of this user's
/// non-deleted transactions.
pub async fn soft_delete(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), TransactionError> {
    let mut tx = pool.begin().await?;

    let result = sqlx::query!(
        r#"
        UPDATE transactions
        SET deleted_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        return Err(TransactionError::NotFound);
    }

    sqlx::query!(
        r#"
        DELETE FROM transfers
        WHERE user_id = $1
          AND (source_transaction_id = $2 OR destination_transaction_id = $2)
        "#,
        user_id,
        id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}

/// Fill `account_amount` / `fx_rate` / `fx_rate_date` for every non-deleted
/// transaction still pending conversion (a foreign-currency row whose
/// booking-date rate had not been fetched — a write made while the rate source
/// was unreachable, or a row predating this feature). A row whose rate is still
/// unavailable stays pending. Returns the number filled.
pub async fn backfill_account_amounts(
    pool: &PgPool,
    cache: &FxRateCache,
) -> Result<usize, TransactionError> {
    let pending = sqlx::query!(
        r#"
        SELECT
            t.id,
            t.asset_id,
            t.amount,
            t.booking_date,
            acc.default_asset_id AS "default_asset_id!",
            src.code AS "source_code!",
            dst.code AS "account_code!",
            f.minor_units AS "minor_units!"
        FROM transactions AS t
        INNER JOIN accounts AS acc ON acc.id = t.account_id
        INNER JOIN assets AS src ON src.id = t.asset_id
        INNER JOIN assets AS dst ON dst.id = acc.default_asset_id
        INNER JOIN fiat_assets AS f ON f.asset_id = acc.default_asset_id
        WHERE t.deleted_at IS NULL AND t.account_amount IS NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut filled = 0;
    for row in pending {
        let (account_amount, fx_rate, fx_rate_date) = compute_conversion(
            cache,
            SourceAmount {
                asset_id: row.asset_id,
                code: &row.source_code,
                amount: &row.amount,
            },
            TargetCurrency {
                asset_id: row.default_asset_id,
                code: &row.account_code,
                minor_units: row.minor_units,
            },
            row.booking_date,
        )
        .await?;

        if account_amount.is_none() {
            continue; // still no rate for that date
        }

        sqlx::query!(
            r#"
            UPDATE transactions
            SET account_amount = $2, fx_rate = $3, fx_rate_date = $4
            WHERE id = $1
            "#,
            row.id,
            account_amount,
            fx_rate,
            fx_rate_date,
        )
        .execute(pool)
        .await?;
        filled += 1;
    }

    Ok(filled)
}

/// Pure-input validation tests.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_amount_accepts_a_negative_value() {
        assert_eq!(
            validate_amount(" -25.50 ").expect("valid"),
            BigDecimal::from_str("-25.50").unwrap()
        );
    }

    #[test]
    fn validate_amount_rejects_zero() {
        assert!(validate_amount("0").is_err());
        assert!(validate_amount("0.00").is_err());
        assert!(validate_amount("-0.0").is_err());
    }

    #[test]
    fn validate_amount_rejects_non_numeric() {
        assert!(validate_amount("").is_err());
        assert!(validate_amount("abc").is_err());
        assert!(validate_amount("1,00").is_err());
    }

    #[test]
    fn validate_amount_rejects_too_many_decimal_places() {
        // 18 fractional digits is the limit.
        assert!(validate_amount("0.123456789012345678").is_ok());
        assert!(validate_amount("0.1234567890123456789").is_err());
    }

    #[test]
    fn validate_amount_rejects_a_value_wider_than_the_column() {
        // 20 integer digits is the limit.
        assert!(validate_amount("99999999999999999999").is_ok());
        assert!(validate_amount("100000000000000000000").is_err());
    }

    #[test]
    fn validate_value_date_treats_blank_as_absent() {
        assert_eq!(validate_value_date(None).expect("valid"), None);
        assert_eq!(validate_value_date(Some("   ")).expect("valid"), None);
        assert_eq!(
            validate_value_date(Some("2026-01-15")).expect("valid"),
            Some(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
        );
        assert!(validate_value_date(Some("15/01/2026")).is_err());
    }

    #[test]
    fn validate_date_range_accepts_open_and_closed_ranges() {
        assert_eq!(
            validate_date_range(None, None).expect("valid"),
            (None, None)
        );
        assert_eq!(
            validate_date_range(Some("  "), Some("2026-01-31")).expect("valid"),
            (None, Some(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()))
        );
        let (from, to) =
            validate_date_range(Some("2026-01-01"), Some("2026-01-01")).expect("valid");
        assert_eq!(from, to);
    }

    #[test]
    fn validate_date_range_rejects_a_backwards_range_and_bad_dates() {
        assert!(validate_date_range(Some("2026-02-01"), Some("2026-01-01")).is_err());
        assert!(validate_date_range(Some("nonsense"), None).is_err());
    }

    #[test]
    fn cursor_round_trips_and_rejects_garbage() {
        let id = Uuid::parse_str("0193c0f0-1234-7abc-8def-0123456789ab").unwrap();
        let cursor = (NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(), id);
        assert_eq!(parse_cursor(&encode_cursor(cursor)).expect("valid"), cursor);
        assert!(parse_cursor("not-a-cursor").is_err());
        assert!(parse_cursor("2026-01-15_not-a-uuid").is_err());
        assert!(parse_cursor("15-01-2026_00000000-0000-0000-0000-000000000000").is_err());
    }

    #[test]
    fn resolve_dates_collapses_cash_and_rejects_a_backwards_value_date() {
        let booking = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let earlier = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let later = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();

        // Cash: value date is forced equal to the booking date, whatever came in.
        assert_eq!(
            resolve_dates(booking, Some(later), true).unwrap(),
            (booking, Some(booking))
        );
        assert_eq!(
            resolve_dates(booking, None, true).unwrap(),
            (booking, Some(booking))
        );

        // Non-cash: blank stays blank; on/after the booking date passes through.
        assert_eq!(
            resolve_dates(booking, None, false).unwrap(),
            (booking, None)
        );
        assert_eq!(
            resolve_dates(booking, Some(booking), false).unwrap(),
            (booking, Some(booking))
        );
        assert_eq!(
            resolve_dates(booking, Some(later), false).unwrap(),
            (booking, Some(later))
        );

        // Non-cash: before the booking date is rejected.
        assert!(matches!(
            resolve_dates(booking, Some(earlier), false),
            Err(TransactionError::InvalidInput(_))
        ));
    }
}

/// Authorization: every query here is scoped by `user_id`. `list` returns only
/// the caller's rows (an `account_id` filter that names someone else's account
/// just matches nothing); `create` / `update` / `soft_delete` report `NotFound`
/// for a row or account that is not the caller's, indistinguishable from a
/// missing one. Each `#[sqlx::test]` runs against its own freshly migrated
/// database.
///
/// Cross-user category or merchant names cannot leak through the two
/// `LEFT JOIN`s: `transactions` carries composite foreign keys to
/// `categories (user_id, id)` and `merchants (user_id, id)`, so a row pointing
/// at another user's category cannot be inserted in the first place.
#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::test_support::{
        create_user, currency_id, date, dec, insert_transaction, insert_transfer,
    };
    use crate::server::{accounts, categories, merchants};

    /// A rate cache with no HTTP reach. Every fixture below is single-currency,
    /// so `create` / `update` never resolve a rate; the cross-currency cases
    /// seed it directly.
    fn fx_cache() -> Arc<FxRateCache> {
        FxRateCache::new().expect("build fx cache")
    }

    /// The caller's transactions on one account, newest first (first page).
    async fn account_rows(
        pool: &PgPool,
        user_id: Uuid,
        account_id: Uuid,
    ) -> Vec<TransactionRecord> {
        list(
            pool,
            user_id,
            TransactionFilter {
                account_id: Some(account_id),
                ..TransactionFilter::default()
            },
        )
        .await
        .expect("list runs")
        .records
    }

    /// A `TransactionWrite` with the given currency / amount / booking date and
    /// nothing else set. Tests that exercise the value date, category, or
    /// merchant tweak the returned value.
    fn write(asset_id: Uuid, amount: &str, booking_date: NaiveDate) -> TransactionWrite {
        TransactionWrite {
            asset_id,
            amount: dec(amount),
            booking_date,
            value_date: None,
            category_id: None,
            merchant_id: None,
        }
    }

    #[sqlx::test]
    async fn list_returns_nothing_for_another_users_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, account.id, eur, "-25.50", date(2026, 1, 16)).await;

        let denied = account_rows(&pool, bob, account.id).await;
        assert!(denied.is_empty());

        // Control: the owner sees both rows, so the account really has data.
        let owned = account_rows(&pool, alice, account.id).await;
        assert_eq!(owned.len(), 2);
    }

    #[sqlx::test]
    async fn list_does_not_leak_the_callers_own_rows_for_a_foreign_account(pool: PgPool) {
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
        let bobs_transaction =
            insert_transaction(&pool, bob, bobs.id, eur, "42.00", date(2026, 1, 15)).await;

        // Bob filtering by Alice's account must get nothing at all -- in
        // particular not his own rows, which is what a dropped `user_id`
        // predicate would return.
        assert!(account_rows(&pool, bob, alices.id).await.is_empty());

        // Control: Bob's own account still returns his row.
        let owned = account_rows(&pool, bob, bobs.id).await;
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, bobs_transaction);
    }

    #[sqlx::test]
    async fn list_is_scoped_to_the_caller_across_all_accounts(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let alices = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("bob's account");

        insert_transaction(&pool, alice, alices.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, alices.id, eur, "20.00", date(2026, 1, 16)).await;
        insert_transaction(&pool, bob, bobs.id, eur, "99.00", date(2026, 1, 17)).await;

        let page = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(page.records.len(), 2);
        assert!(page
            .records
            .iter()
            .all(|record| record.account_id == alices.id));
        // Newest first.
        assert_eq!(page.records[0].booking_date, date(2026, 1, 16));
        assert_eq!(page.next, None);
    }

    #[sqlx::test]
    async fn list_filters_by_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let cash = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("cash account");
        let bank = accounts::create(&pool, alice, "Bank", "bank", eur)
            .await
            .expect("bank account");

        insert_transaction(&pool, alice, cash.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, bank.id, eur, "20.00", date(2026, 1, 15)).await;

        let page = list(
            &pool,
            alice,
            TransactionFilter {
                account_id: Some(bank.id),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].account_id, bank.id);
        assert_eq!(page.records[0].account_name, "Bank");
    }

    #[sqlx::test]
    async fn list_date_range_is_inclusive_on_both_ends(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("account");

        for day in [10, 15, 20, 25] {
            insert_transaction(&pool, alice, account.id, eur, "1.00", date(2026, 1, day)).await;
        }

        let page = list(
            &pool,
            alice,
            TransactionFilter {
                from: Some(date(2026, 1, 15)),
                to: Some(date(2026, 1, 20)),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");

        let days: Vec<_> = page
            .records
            .iter()
            .map(|record| record.booking_date)
            .collect();
        assert_eq!(days, vec![date(2026, 1, 20), date(2026, 1, 15)]);
    }

    #[sqlx::test]
    async fn list_paginates_without_gaps_or_repeats(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("account");

        // One more than a page; rows share booking dates so the `id` tiebreak
        // is what keeps the page boundary clean.
        let total = PAGE_SIZE + 1;
        for i in 0..total {
            let day = 1 + (i % 3) as u32;
            insert_transaction(&pool, alice, account.id, eur, "1.00", date(2026, 1, day)).await;
        }

        let first = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(first.records.len(), PAGE_SIZE);
        let cursor = first.next.expect("a second page");

        let second = list(
            &pool,
            alice,
            TransactionFilter {
                after: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.next, None);

        let seam: Vec<_> = first
            .records
            .iter()
            .chain(&second.records)
            .map(|record| (record.booking_date, record.id))
            .collect();

        // Strictly descending by (booking_date, id) across the page boundary…
        let mut ordered = seam.clone();
        ordered.sort_by(|a, b| b.cmp(a));
        assert_eq!(seam, ordered);

        // …and every row exactly once.
        let mut ids: Vec<_> = seam.iter().map(|(_, id)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total);
    }

    #[sqlx::test]
    async fn list_excludes_soft_deleted_transactions_and_accounts(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let live = accounts::create(&pool, alice, "Live", "cash", eur)
            .await
            .expect("live account");
        let gone = accounts::create(&pool, alice, "Gone", "cash", eur)
            .await
            .expect("gone account");

        let kept = insert_transaction(&pool, alice, live.id, eur, "1.00", date(2026, 1, 15)).await;
        let deleted_txn =
            insert_transaction(&pool, alice, live.id, eur, "2.00", date(2026, 1, 16)).await;
        insert_transaction(&pool, alice, gone.id, eur, "3.00", date(2026, 1, 17)).await;

        soft_delete(&pool, alice, deleted_txn)
            .await
            .expect("delete transaction");
        accounts::soft_delete(&pool, alice, gone.id)
            .await
            .expect("delete account");

        let page = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, kept);
    }

    #[sqlx::test]
    async fn create_assigns_the_calling_users_id_and_rejects_a_foreign_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &write(eur, "10.00", date(2026, 1, 15)),
        )
        .await
        .expect("alice records a transaction");

        // Bob cannot record a transaction on Alice's account.
        let denied = create(
            &pool,
            &fx_cache(),
            bob,
            account.id,
            &write(eur, "10.00", date(2026, 1, 15)),
        )
        .await;
        assert!(matches!(denied, Err(TransactionError::NotFound)));

        // Only Alice's row exists.
        assert!(account_rows(&pool, bob, account.id).await.is_empty());
        assert_eq!(account_rows(&pool, alice, account.id).await.len(), 1);
    }

    #[sqlx::test]
    async fn a_bank_account_stores_the_value_date_only_when_given(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("alice's account");

        create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &write(eur, "10.00", date(2026, 1, 15)),
        )
        .await
        .expect("no value date");
        create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &TransactionWrite {
                value_date: Some(date(2026, 1, 18)),
                ..write(eur, "20.00", date(2026, 1, 16))
            },
        )
        .await
        .expect("with value date");

        let rows = account_rows(&pool, alice, account.id).await;
        // Newest first: the 16th (with a value date), then the 15th (without).
        assert_eq!(rows[0].value_date, Some(date(2026, 1, 18)));
        assert_eq!(rows[1].value_date, None);
    }

    #[sqlx::test]
    async fn a_cash_account_forces_the_value_date_equal_to_the_booking_date(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        // Even a value date the caller supplied is overridden.
        create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &TransactionWrite {
                value_date: Some(date(2026, 1, 20)),
                ..write(eur, "10.00", date(2026, 1, 15))
            },
        )
        .await
        .expect("cash transaction");

        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].value_date, Some(date(2026, 1, 15)));
    }

    #[sqlx::test]
    async fn a_value_date_before_the_booking_date_is_rejected(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("alice's account");

        let denied = create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &TransactionWrite {
                value_date: Some(date(2026, 1, 10)),
                ..write(eur, "10.00", date(2026, 1, 15))
            },
        )
        .await;
        assert!(matches!(
            denied,
            Err(TransactionError::InvalidInput(
                "the value date cannot be before the booking date"
            ))
        ));
    }

    #[sqlx::test]
    async fn update_denies_another_users_transaction_and_leaves_it_unchanged(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        let denied = update(
            &pool,
            &fx_cache(),
            bob,
            transaction,
            &write(usd, "-999.00", date(2026, 2, 1)),
        )
        .await;
        assert!(matches!(denied, Err(TransactionError::NotFound)));

        let after = account_rows(&pool, alice, account.id).await;
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].amount, dec("100.00"));
        assert_eq!(after[0].asset_id, eur);
        assert_eq!(after[0].booking_date, date(2026, 1, 15));
    }

    #[sqlx::test]
    async fn soft_delete_denies_another_users_transaction(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        assert!(matches!(
            soft_delete(&pool, bob, transaction).await,
            Err(TransactionError::NotFound)
        ));
        assert_eq!(account_rows(&pool, alice, account.id).await.len(), 1);

        // The owner deletes it; a second delete is not a fresh success.
        soft_delete(&pool, alice, transaction)
            .await
            .expect("owner deletes it");
        assert!(matches!(
            soft_delete(&pool, alice, transaction).await,
            Err(TransactionError::NotFound)
        ));
    }

    #[sqlx::test]
    async fn soft_deleted_transaction_drops_out_of_account_balances(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        let balance: Option<BigDecimal> =
            sqlx::query_scalar("SELECT balance FROM account_balances WHERE account_id = $1")
                .bind(account.id)
                .fetch_optional(&pool)
                .await
                .expect("query runs");
        assert_eq!(balance, Some(dec("100.00")));

        soft_delete(&pool, alice, transaction)
            .await
            .expect("owner deletes it");

        let balance: Option<BigDecimal> =
            sqlx::query_scalar("SELECT balance FROM account_balances WHERE account_id = $1")
                .bind(account.id)
                .fetch_optional(&pool)
                .await
                .expect("query runs");
        assert_eq!(balance, None);
    }

    #[sqlx::test]
    async fn a_transfer_leg_is_edited_like_any_other_transaction(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings");

        let source =
            insert_transaction(&pool, alice, checking.id, eur, "-10.00", date(2026, 1, 15)).await;
        let destination =
            insert_transaction(&pool, alice, savings.id, eur, "10.00", date(2026, 1, 15)).await;
        let transfer_id = insert_transfer(&pool, alice, source, destination).await;

        update(
            &pool,
            &fx_cache(),
            alice,
            source,
            &write(eur, "-5.00", date(2026, 1, 16)),
        )
        .await
        .expect("editing a leg is allowed");

        let rows = account_rows(&pool, alice, checking.id).await;
        assert_eq!(rows[0].amount, dec("-5.00"));
        // The leg is still linked to the same transfer.
        assert_eq!(rows[0].transfer_id, Some(transfer_id));
    }

    #[sqlx::test]
    async fn deleting_a_transfer_leg_removes_the_link(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings");

        let source =
            insert_transaction(&pool, alice, checking.id, eur, "-10.00", date(2026, 1, 15)).await;
        let destination =
            insert_transaction(&pool, alice, savings.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transfer(&pool, alice, source, destination).await;

        soft_delete(&pool, alice, source)
            .await
            .expect("deleting a leg is allowed");

        // The deleted leg is gone; the surviving leg is a plain transaction.
        assert!(account_rows(&pool, alice, checking.id).await.is_empty());
        let survivor = account_rows(&pool, alice, savings.id).await;
        assert_eq!(survivor.len(), 1);
        assert_eq!(survivor[0].transfer_id, None);
    }

    #[sqlx::test]
    async fn list_marks_transfer_legs_with_the_counterparty(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings");
        insert_transaction(&pool, alice, checking.id, eur, "5.00", date(2026, 1, 10)).await;

        let source =
            insert_transaction(&pool, alice, checking.id, eur, "-30", date(2026, 1, 15)).await;
        let destination =
            insert_transaction(&pool, alice, savings.id, eur, "30", date(2026, 1, 15)).await;
        let transfer_id = insert_transfer(&pool, alice, source, destination).await;

        // The checking leg (newest row) names savings; the plain transaction has
        // no transfer id.
        let checking_rows = account_rows(&pool, alice, checking.id).await;
        assert_eq!(
            checking_rows[0].transfer_id.map(|id| id.to_string()),
            Some(transfer_id.to_string())
        );
        assert_eq!(
            checking_rows[0].transfer_counterparty.as_deref(),
            Some("Savings")
        );
        assert_eq!(checking_rows[1].transfer_id, None);

        let savings_rows = account_rows(&pool, alice, savings.id).await;
        assert_eq!(
            savings_rows[0].transfer_id.map(|id| id.to_string()),
            Some(transfer_id.to_string())
        );
        assert_eq!(
            savings_rows[0].transfer_counterparty.as_deref(),
            Some("Checking")
        );
    }

    #[sqlx::test]
    async fn create_and_update_set_the_category_and_merchant(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let groceries = categories::create(&pool, alice, "Groceries", "expense")
            .await
            .expect("category")
            .id;
        let salary = categories::create(&pool, alice, "Salary", "income")
            .await
            .expect("category")
            .id;
        let acme = merchants::create(&pool, alice, "Acme", None)
            .await
            .expect("merchant")
            .id;

        let id = create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &TransactionWrite {
                category_id: Some(groceries),
                merchant_id: Some(acme),
                ..write(eur, "-12.00", date(2026, 1, 15))
            },
        )
        .await
        .expect("create");

        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].category_id, Some(groceries));
        assert_eq!(rows[0].category_name.as_deref(), Some("Groceries"));
        assert_eq!(rows[0].merchant_id, Some(acme));
        assert_eq!(rows[0].merchant_name.as_deref(), Some("Acme"));

        // Update swaps the category and clears the merchant.
        update(
            &pool,
            &fx_cache(),
            alice,
            id,
            &TransactionWrite {
                category_id: Some(salary),
                merchant_id: None,
                ..write(eur, "-12.00", date(2026, 1, 15))
            },
        )
        .await
        .expect("update");

        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].category_id, Some(salary));
        assert_eq!(rows[0].merchant_id, None);
        assert_eq!(rows[0].merchant_name, None);
    }

    #[sqlx::test]
    async fn create_and_update_reject_a_foreign_or_deleted_category_or_merchant(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");

        let bobs_category = categories::create(&pool, bob, "Bob Cat", "expense")
            .await
            .expect("category")
            .id;
        let bobs_merchant = merchants::create(&pool, bob, "Bob Merchant", None)
            .await
            .expect("merchant")
            .id;
        let alices_category = categories::create(&pool, alice, "Food", "expense")
            .await
            .expect("category")
            .id;
        let alices_merchant = merchants::create(&pool, alice, "Store", None)
            .await
            .expect("merchant")
            .id;

        // Another user's category / merchant is not reachable.
        assert!(matches!(
            create(
                &pool,
                &fx_cache(),
                alice,
                account.id,
                &TransactionWrite {
                    category_id: Some(bobs_category),
                    ..write(eur, "-1.00", date(2026, 1, 15))
                },
            )
            .await,
            Err(TransactionError::UnknownCategory)
        ));
        assert!(matches!(
            create(
                &pool,
                &fx_cache(),
                alice,
                account.id,
                &TransactionWrite {
                    merchant_id: Some(bobs_merchant),
                    ..write(eur, "-1.00", date(2026, 1, 15))
                },
            )
            .await,
            Err(TransactionError::UnknownMerchant)
        ));

        // A soft-deleted own category / merchant is refused too.
        let id = create(
            &pool,
            &fx_cache(),
            alice,
            account.id,
            &write(eur, "-1.00", date(2026, 1, 15)),
        )
        .await
        .expect("plain create");
        categories::soft_delete(&pool, alice, alices_category)
            .await
            .expect("delete category");
        merchants::soft_delete(&pool, alice, alices_merchant)
            .await
            .expect("delete merchant");
        assert!(matches!(
            update(
                &pool,
                &fx_cache(),
                alice,
                id,
                &TransactionWrite {
                    category_id: Some(alices_category),
                    ..write(eur, "-1.00", date(2026, 1, 15))
                },
            )
            .await,
            Err(TransactionError::UnknownCategory)
        ));
        assert!(matches!(
            update(
                &pool,
                &fx_cache(),
                alice,
                id,
                &TransactionWrite {
                    merchant_id: Some(alices_merchant),
                    ..write(eur, "-1.00", date(2026, 1, 15))
                },
            )
            .await,
            Err(TransactionError::UnknownMerchant)
        ));
    }

    #[sqlx::test]
    async fn a_foreign_transaction_is_recorded_in_the_account_currency(pool: PgPool) {
        use sqlx::types::chrono::{DateTime, Utc};

        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");
        let booking = date(2026, 3, 4);
        let cache = FxRateCache::new().expect("cache");

        // No USD -> EUR rate: the transaction records with a pending conversion.
        create(
            &pool,
            &cache,
            alice,
            account.id,
            &write(usd, "100.00", booking),
        )
        .await
        .expect("pending");
        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].account_amount, None);
        assert_eq!(rows[0].fx_rate, None);

        // With a seeded rate, a foreign transaction is converted and rounded.
        let as_of =
            DateTime::<Utc>::from_naive_utc_and_offset(booking.and_hms_opt(0, 0, 0).unwrap(), Utc);
        cache.seed("USD", booking, as_of, &[("EUR", "0.905")]);
        create(
            &pool,
            &cache,
            alice,
            account.id,
            &write(usd, "100.00", booking),
        )
        .await
        .expect("converted");
        let rows = account_rows(&pool, alice, account.id).await;
        // 100 * 0.905 = 90.500 -> 90.50 at 2 minor units.
        assert_eq!(rows[0].account_amount, Some(dec("90.50")));
        assert_eq!(rows[0].fx_rate, Some(dec("0.905")));

        // A same-currency transaction just copies the amount, no rate.
        create(
            &pool,
            &cache,
            alice,
            account.id,
            &write(eur, "25.00", booking),
        )
        .await
        .expect("home currency");
        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].account_amount, Some(dec("25.00")));
        assert_eq!(rows[0].fx_rate, None);
    }

    #[sqlx::test]
    async fn backfill_fills_a_pending_conversion_once_a_rate_is_seeded(pool: PgPool) {
        use sqlx::types::chrono::{DateTime, Utc};

        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");
        let booking = date(2026, 5, 6);
        let cache = FxRateCache::new().expect("cache");

        create(
            &pool,
            &cache,
            alice,
            account.id,
            &write(usd, "10.00", booking),
        )
        .await
        .expect("pending");
        assert_eq!(
            backfill_account_amounts(&pool, &cache).await.expect("runs"),
            0
        );

        let as_of =
            DateTime::<Utc>::from_naive_utc_and_offset(booking.and_hms_opt(0, 0, 0).unwrap(), Utc);
        cache.seed("USD", booking, as_of, &[("EUR", "0.8")]);
        assert_eq!(
            backfill_account_amounts(&pool, &cache).await.expect("runs"),
            1
        );

        let rows = account_rows(&pool, alice, account.id).await;
        assert_eq!(rows[0].account_amount, Some(dec("8.00")));
    }
}
