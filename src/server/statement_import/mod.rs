//! Extracts candidate transaction rows from a bank-statement PDF for user
//! review, and inserts the reviewed rows exactly like a manually entered
//! transaction. Nothing is written to the database until the user confirms
//! (see [`extract_deblock_rows`] vs [`insert_confirmed_rows`]). Only Deblock
//! is supported today; see `deblock`'s doc comment for how a second bank
//! would plug into `extraction`.
//!
//! PDF content is untrusted, external data (`src/server/CLAUDE.md`): a
//! structurally unrecognized file is a hard error; a bad individual row is
//! skipped and reported, never assumed. Transaction direction (debit vs.
//! credit) is decided only by which PDF column the amount was drawn in --
//! never by parsing words like "Frais"/"Virement".

pub mod deblock;
mod extraction;

use std::collections::HashMap;

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::accounts::{self, AccountError};
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::merchants;
use crate::server::transaction_import::ImportOutcome;
use crate::server::transactions::{self, TransactionWrite};
use crate::transactions::api::parse_optional_id;

#[derive(Debug, thiserror::Error)]
pub enum StatementImportError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for StatementImportError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("statement import: database error: {err}");
        StatementImportError::Internal
    }
}

/// A row ready for review, plus the merchant suggestion resolved against the
/// user's active merchants. The suggestion is never authoritative -- the
/// user can change or clear it before confirming.
pub struct ResolvedRow {
    pub row_number: usize,
    pub booking_date: sqlx::types::chrono::NaiveDate,
    pub value_date: sqlx::types::chrono::NaiveDate,
    pub amount: sqlx::types::BigDecimal,
    pub description: String,
    pub suggested_merchant_id: Option<Uuid>,
    pub suggested_merchant_name: Option<String>,
}

pub struct StatementExtraction {
    pub rows: Vec<ResolvedRow>,
    pub rejected: Vec<crate::server::transaction_import::RowError>,
}

/// Parse `bytes` as a Deblock statement and resolve each row's merchant
/// suggestion. See [`deblock::parse`] for the two-tier error model
/// (structurally unrecognized file = hard error; a bad row = a rejection).
/// `statement_year` is only used for an English-language statement, whose
/// dates never print a year; it's ignored for a French one.
pub async fn extract_deblock_rows(
    pool: &PgPool,
    user_id: Uuid,
    bytes: &[u8],
    statement_year: Option<i32>,
) -> Result<StatementExtraction, StatementImportError> {
    // name (lowercased) -> (id, display name); first match wins on a
    // case-only collision, same documented tie-break as the CSV import's
    // merchant/category name index.
    let mut merchant_index: HashMap<String, (Uuid, String)> = HashMap::new();
    for merchant in merchants::list_active_for_user(pool, user_id)
        .await
        .map_err(|_| StatementImportError::Internal)?
    {
        merchant_index
            .entry(merchant.merchant_name.to_lowercase())
            .or_insert((merchant.id, merchant.merchant_name));
    }

    let parsed = deblock::parse(bytes, statement_year)?;

    let rows = parsed
        .rows
        .into_iter()
        .map(|row| {
            let suggestion = merchant_index
                .get(&row.merchant_candidate.to_lowercase())
                .cloned();
            ResolvedRow {
                row_number: row.row_number,
                booking_date: row.booking_date,
                value_date: row.value_date,
                amount: row.amount,
                description: row.description,
                suggested_merchant_id: suggestion.as_ref().map(|(id, _)| *id),
                suggested_merchant_name: suggestion.map(|(_, name)| name),
            }
        })
        .collect();

    Ok(StatementExtraction {
        rows,
        rejected: parsed.rejected,
    })
}

/// One reviewed row as the browser sends it back, shaped like
/// `create_transaction`'s own arguments (plain strings; a blank id means
/// none).
pub struct ConfirmedRowInput {
    pub booking_date: String,
    pub value_date: Option<String>,
    pub amount: String,
    pub category_id: String,
    pub merchant_id: String,
}

fn resolve_confirmed_row(
    row: &ConfirmedRowInput,
    asset_id: Uuid,
) -> Result<TransactionWrite, String> {
    Ok(TransactionWrite {
        asset_id,
        amount: transactions::validate_amount(&row.amount).map_err(|err| err.to_string())?,
        booking_date: transactions::validate_booking_date(&row.booking_date)
            .map_err(|err| err.to_string())?,
        value_date: transactions::validate_value_date(row.value_date.as_deref())
            .map_err(|err| err.to_string())?,
        category_id: parse_optional_id(&row.category_id, "invalid category id")
            .map_err(|err| err.to_string())?,
        merchant_id: parse_optional_id(&row.merchant_id, "invalid merchant id")
            .map_err(|err| err.to_string())?,
    })
}

/// Insert reviewed rows onto `account_id`, exactly like [`transactions::create`]
/// -- every field is re-validated here; nothing from the extraction step is
/// trusted merely because it was shown to the user. Every row is recorded in
/// the account's own default currency (a Deblock statement has no per-row
/// currency), like the CSV import.
pub async fn insert_confirmed_rows(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    account_id: Uuid,
    rows: Vec<ConfirmedRowInput>,
) -> Result<ImportOutcome, StatementImportError> {
    let account = accounts::find_for_user(pool, user_id, account_id)
        .await
        .map_err(|err| match err {
            AccountError::NotFound => StatementImportError::NotFound,
            _ => StatementImportError::Internal,
        })?;

    let mut outcome = ImportOutcome {
        imported: 0,
        rejected: Vec::new(),
    };
    for (offset, row) in rows.iter().enumerate() {
        let row_number = offset + 1;
        match resolve_confirmed_row(row, account.default_asset_id) {
            Ok(write) => match transactions::create(pool, cache, user_id, account_id, &write).await
            {
                Ok(_) => outcome.imported += 1,
                Err(err) => outcome
                    .rejected
                    .push(crate::server::transaction_import::RowError {
                        row_number,
                        reason: err.to_string(),
                    }),
            },
            Err(reason) => outcome
                .rejected
                .push(crate::server::transaction_import::RowError { row_number, reason }),
        }
    }
    Ok(outcome)
}

/// Authorization and end-to-end row outcomes. Each `#[sqlx::test]` runs
/// against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::test_support::{create_user, currency_id};

    fn fx_cache() -> Arc<FxRateCache> {
        FxRateCache::new().expect("build fx cache")
    }

    #[sqlx::test]
    async fn insert_confirmed_rows_inserts_valid_rows_and_reports_rejected_ones(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let cache = fx_cache();

        let rows = vec![
            ConfirmedRowInput {
                booking_date: "2024-04-29".to_owned(),
                value_date: Some("2024-04-29".to_owned()),
                amount: "50.00".to_owned(),
                category_id: String::new(),
                merchant_id: String::new(),
            },
            ConfirmedRowInput {
                booking_date: "not-a-date".to_owned(),
                value_date: None,
                amount: "-9.99".to_owned(),
                category_id: String::new(),
                merchant_id: String::new(),
            },
        ];

        let outcome = insert_confirmed_rows(&pool, &cache, alice, account.id, rows)
            .await
            .expect("insert runs");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.rejected.len(), 1);
        assert_eq!(outcome.rejected[0].row_number, 2);
    }

    #[sqlx::test]
    async fn insert_confirmed_rows_denies_an_account_that_is_not_the_callers(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let bobs_account = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("account");
        let cache = fx_cache();

        let result = insert_confirmed_rows(
            &pool,
            &cache,
            alice,
            bobs_account.id,
            vec![ConfirmedRowInput {
                booking_date: "2024-04-29".to_owned(),
                value_date: None,
                amount: "10.00".to_owned(),
                category_id: String::new(),
                merchant_id: String::new(),
            }],
        )
        .await;

        assert!(matches!(result, Err(StatementImportError::NotFound)));
    }
}
