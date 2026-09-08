//! Server functions the browser calls to read the signed-in user's
//! transactions.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::transactions::types::TransactionDto;

/// The 50 most recent non-deleted transactions on one of the current user's
/// accounts, newest first (by booking date, then creation time).
#[server]
pub async fn list_account_transactions(
    account_id: String,
) -> Result<Vec<TransactionDto>, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| TransactionError::InvalidInput("invalid account id"))?;

    let records = transactions::recent_for_account(&pool, user.user_id, account_id).await?;

    Ok(records
        .into_iter()
        .map(|record| TransactionDto {
            id: record.id.to_string(),
            amount: record.amount.to_string(),
            asset_code: record.asset_code,
            booking_date: record.booking_date.to_string(),
            category_name: record.category_name,
            merchant_name: record.merchant_name,
        })
        .collect())
}
