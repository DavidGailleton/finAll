//! Query for a signed-in user's transactions on one account (`transactions`
//! joined to `assets` for the currency code and to `categories` / `merchants`
//! for display names) and the domain error.
//!
//! The query is scoped by `user_id` so one user can never read another's
//! transactions.

use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("you must be signed in to do this")]
    Unauthorized,

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
    pub asset_code: String,
    pub booking_date: NaiveDate,
    pub category_name: Option<String>,
    pub merchant_name: Option<String>,
}

/// The 50 most recent non-deleted transactions on one of the user's accounts,
/// newest first (`booking_date` then `created_at`).
///
/// The account is not checked for existence here; a `account_id` that is not
/// one of this user's accounts simply matches no rows.
pub async fn recent_for_account(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<Vec<TransactionRecord>, TransactionError> {
    let records = sqlx::query_as!(
        TransactionRecord,
        r#"
        SELECT
            t.id,
            t.amount,
            a.code AS "asset_code!",
            t.booking_date,
            c.category_name AS "category_name?",
            m.merchant_name AS "merchant_name?"
        FROM transactions AS t
        INNER JOIN assets AS a ON a.id = t.asset_id
        LEFT JOIN categories AS c
            ON c.user_id = t.user_id AND c.id = t.category_id AND c.deleted_at IS NULL
        LEFT JOIN merchants AS m
            ON m.user_id = t.user_id AND m.id = t.merchant_id AND m.deleted_at IS NULL
        WHERE t.user_id = $1 AND t.account_id = $2 AND t.deleted_at IS NULL
        ORDER BY t.booking_date DESC, t.created_at DESC
        LIMIT 50
        "#,
        user_id,
        account_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}
