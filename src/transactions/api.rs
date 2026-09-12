//! Server functions the browser calls to read and manage the signed-in user's
//! transactions.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::transactions::types::TransactionPage;

/// Parse an optional id form field: a blank value is `None`, otherwise it must
/// be a valid UUID. Ownership of the referenced row is enforced server-side.
#[cfg(feature = "ssr")]
pub(crate) fn parse_optional_id(
    value: &str,
    message: &'static str,
) -> Result<Option<sqlx::types::Uuid>, crate::server::transactions::TransactionError> {
    match value.trim() {
        "" => Ok(None),
        id => sqlx::types::Uuid::parse_str(id)
            .map(Some)
            .map_err(|_| crate::server::transactions::TransactionError::InvalidInput(message)),
    }
}

/// One page (newest first, `booking_date` then `id`) of the current user's
/// non-deleted transactions.
///
/// `account_id` narrows to a single account; `from` / `to` are inclusive
/// `booking_date` bounds (ISO `YYYY-MM-DD`, blank for open-ended); `cursor` is
/// the `next_cursor` of the previous page. All are optional.
#[server]
pub async fn list_transactions(
    account_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    cursor: Option<String>,
) -> Result<TransactionPage, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::accounts::types::AccountType;
    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError, TransactionFilter};
    use crate::transactions::types::TransactionDto;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let account_id = account_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| TransactionError::InvalidInput("invalid account id"))?;
    let (from, to) = transactions::validate_date_range(from.as_deref(), to.as_deref())?;
    let after = cursor
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(transactions::parse_cursor)
        .transpose()?;

    let page = transactions::list(
        &pool,
        user.user_id,
        TransactionFilter {
            account_id,
            from,
            to,
            after,
        },
    )
    .await?;

    Ok(TransactionPage {
        transactions: page
            .records
            .into_iter()
            .map(|record| TransactionDto {
                id: record.id.to_string(),
                account_id: record.account_id.to_string(),
                account_name: record.account_name,
                account_type: AccountType::from_db_str(&record.account_type)
                    .unwrap_or(AccountType::Other),
                account_default_asset_id: record.account_default_asset_id.to_string(),
                account_currency_code: record.account_currency_code,
                amount: record.amount.to_string(),
                account_amount: record.account_amount.map(|amount| amount.to_string()),
                fx_rate: record.fx_rate.map(|rate| rate.to_string()),
                asset_id: record.asset_id.to_string(),
                asset_code: record.asset_code,
                booking_date: record.booking_date.to_string(),
                value_date: record.value_date.map(|date| date.to_string()),
                category_id: record.category_id.map(|id| id.to_string()),
                category_name: record.category_name,
                merchant_id: record.merchant_id.map(|id| id.to_string()),
                merchant_name: record.merchant_name,
                transfer_id: record.transfer_id.map(|id| id.to_string()),
                transfer_counterparty: record.transfer_counterparty,
            })
            .collect(),
        next_cursor: page.next.map(transactions::encode_cursor),
    })
}

/// Record a new transaction on one of the current user's accounts. The amount
/// is a signed decimal string; `value_date` is optional.
#[server]
pub async fn create_transaction(
    account_id: String,
    asset_id: String,
    amount: String,
    booking_date: String,
    value_date: Option<String>,
    category_id: String,
    merchant_id: String,
) -> Result<(), ServerFnError> {
    use std::sync::Arc;

    use sqlx::types::Uuid;

    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError, TransactionWrite};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| TransactionError::InvalidInput("invalid account id"))?;
    let write = TransactionWrite {
        asset_id: Uuid::parse_str(&asset_id)
            .map_err(|_| TransactionError::InvalidInput("invalid currency id"))?,
        amount: transactions::validate_amount(&amount)?,
        booking_date: transactions::validate_booking_date(&booking_date)?,
        value_date: transactions::validate_value_date(value_date.as_deref())?,
        category_id: parse_optional_id(&category_id, "invalid category id")?,
        merchant_id: parse_optional_id(&merchant_id, "invalid merchant id")?,
    };

    transactions::create(&pool, &cache, user.user_id, account_id, &write).await?;

    Ok(())
}

/// Update one of the current user's transactions: its amount, currency, and
/// dates.
#[server]
pub async fn update_transaction(
    id: String,
    asset_id: String,
    amount: String,
    booking_date: String,
    value_date: Option<String>,
    category_id: String,
    merchant_id: String,
) -> Result<(), ServerFnError> {
    use std::sync::Arc;

    use sqlx::types::Uuid;

    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError, TransactionWrite};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let id = Uuid::parse_str(&id)
        .map_err(|_| TransactionError::InvalidInput("invalid transaction id"))?;
    let write = TransactionWrite {
        asset_id: Uuid::parse_str(&asset_id)
            .map_err(|_| TransactionError::InvalidInput("invalid currency id"))?,
        amount: transactions::validate_amount(&amount)?,
        booking_date: transactions::validate_booking_date(&booking_date)?,
        value_date: transactions::validate_value_date(value_date.as_deref())?,
        category_id: parse_optional_id(&category_id, "invalid category id")?,
        merchant_id: parse_optional_id(&merchant_id, "invalid merchant id")?,
    };

    transactions::update(&pool, &cache, user.user_id, id, &write).await?;

    Ok(())
}

/// Import transactions from a CSV file onto one of the current user's
/// accounts. Every row is validated and inserted exactly like
/// [`create_transaction`] (through the same [`transactions::create`]); a row
/// that fails is skipped and reported in `ImportSummary::rejected` rather than
/// aborting the whole import. Every imported row is recorded in the account's
/// own default currency -- a CSV row cannot record a foreign-currency
/// transaction.
///
/// Expected columns (header row required, case-insensitive names): `date`
/// (`YYYY-MM-DD`, required), `amount` (signed decimal, required), `category`
/// (name of an existing category, optional), `merchant` (name of an existing
/// merchant, optional).
#[server]
pub async fn import_transactions(
    account_id: String,
    csv_content: String,
) -> Result<crate::transactions::types::ImportSummary, ServerFnError> {
    use std::sync::Arc;

    use sqlx::types::Uuid;

    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::transaction_import::{self, TransactionImportError};
    use crate::transactions::types::{ImportRowError, ImportSummary};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionImportError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| TransactionImportError::InvalidInput("invalid account id"))?;

    let outcome =
        transaction_import::import_csv(&pool, &cache, user.user_id, account_id, &csv_content)
            .await?;

    Ok(ImportSummary {
        imported: outcome.imported,
        rejected: outcome
            .rejected
            .into_iter()
            .map(|row| ImportRowError {
                row_number: row.row_number,
                reason: row.reason,
            })
            .collect(),
    })
}

/// Extract candidate transaction rows from an uploaded bank-statement PDF for
/// review -- nothing is written to the database. Only born-digital Deblock
/// statements are currently supported (no OCR, no scanned statements).
/// `suggested_merchant_id` is filled in only when a row's extracted
/// counterparty text case-insensitively exact-matches one of the user's
/// active merchants; category is never suggested.
#[server]
pub async fn extract_statement_pdf(
    pdf_bytes: Vec<u8>,
    /// Only needed for an English-language statement, whose dates never
    /// print a year anywhere in the file; ignored for a French one. Blank
    /// means "not supplied".
    statement_year: Option<String>,
) -> Result<crate::transactions::types::StatementExtractionDto, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::statement_import::{self, StatementImportError};
    use crate::transactions::types::{ExtractedRowDto, ImportRowError, StatementExtractionDto};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(StatementImportError::Unauthorized)?;

    let statement_year = statement_year
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|_| StatementImportError::InvalidInput("statement year must be a number"))
        })
        .transpose()?;

    let extraction =
        statement_import::extract_deblock_rows(&pool, user.user_id, &pdf_bytes, statement_year)
            .await?;

    Ok(StatementExtractionDto {
        rows: extraction
            .rows
            .into_iter()
            .map(|row| ExtractedRowDto {
                row_number: row.row_number,
                booking_date: row.booking_date.to_string(),
                value_date: Some(row.value_date.to_string()),
                amount: row.amount.to_string(),
                description: row.description,
                suggested_merchant_id: row.suggested_merchant_id.map(|id| id.to_string()),
                suggested_merchant_name: row.suggested_merchant_name,
            })
            .collect(),
        rejected: extraction
            .rejected
            .into_iter()
            .map(|row| ImportRowError {
                row_number: row.row_number,
                reason: row.reason,
            })
            .collect(),
    })
}

/// Insert reviewed rows from a previously extracted statement PDF onto one of
/// the current user's accounts, exactly like [`create_transaction`] (through
/// [`transactions::create`]) -- every field is re-validated server-side,
/// nothing from the extraction step is trusted merely because it was shown
/// to the user. A row that fails is skipped and reported in
/// `ImportSummary::rejected` rather than aborting the rest.
#[server]
pub async fn confirm_statement_import(
    account_id: String,
    rows: Vec<crate::transactions::types::ConfirmedRowDto>,
) -> Result<crate::transactions::types::ImportSummary, ServerFnError> {
    use std::sync::Arc;

    use sqlx::types::Uuid;

    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::statement_import::{self, ConfirmedRowInput, StatementImportError};
    use crate::transactions::types::{ImportRowError, ImportSummary};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(StatementImportError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| StatementImportError::InvalidInput("invalid account id"))?;

    let rows = rows
        .into_iter()
        .map(|row| ConfirmedRowInput {
            booking_date: row.booking_date,
            value_date: row.value_date,
            amount: row.amount,
            category_id: row.category_id,
            merchant_id: row.merchant_id,
        })
        .collect();

    let outcome =
        statement_import::insert_confirmed_rows(&pool, &cache, user.user_id, account_id, rows)
            .await?;

    Ok(ImportSummary {
        imported: outcome.imported,
        rejected: outcome
            .rejected
            .into_iter()
            .map(|row| ImportRowError {
                row_number: row.row_number,
                reason: row.reason,
            })
            .collect(),
    })
}

/// Soft-delete one of the current user's transactions.
#[server]
pub async fn delete_transaction(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let id = Uuid::parse_str(&id)
        .map_err(|_| TransactionError::InvalidInput("invalid transaction id"))?;

    transactions::soft_delete(&pool, user.user_id, id).await?;

    Ok(())
}
