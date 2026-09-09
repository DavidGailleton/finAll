//! Server functions the browser calls to read and manage the signed-in user's
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
            asset_id: record.asset_id.to_string(),
            asset_code: record.asset_code,
            booking_date: record.booking_date.to_string(),
            value_date: record.value_date.map(|date| date.to_string()),
            category_name: record.category_name,
            merchant_name: record.merchant_name,
        })
        .collect())
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
) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| TransactionError::InvalidInput("invalid account id"))?;
    let asset_id = Uuid::parse_str(&asset_id)
        .map_err(|_| TransactionError::InvalidInput("invalid currency id"))?;
    let amount = transactions::validate_amount(&amount)?;
    let booking_date = transactions::validate_booking_date(&booking_date)?;
    let value_date = transactions::validate_value_date(value_date.as_deref())?;

    transactions::create(
        &pool,
        user.user_id,
        account_id,
        asset_id,
        &amount,
        booking_date,
        value_date,
    )
    .await?;

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
) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions::{self, TransactionError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransactionError::Unauthorized)?;

    let id = Uuid::parse_str(&id)
        .map_err(|_| TransactionError::InvalidInput("invalid transaction id"))?;
    let asset_id = Uuid::parse_str(&asset_id)
        .map_err(|_| TransactionError::InvalidInput("invalid currency id"))?;
    let amount = transactions::validate_amount(&amount)?;
    let booking_date = transactions::validate_booking_date(&booking_date)?;
    let value_date = transactions::validate_value_date(value_date.as_deref())?;

    transactions::update(
        &pool,
        user.user_id,
        id,
        asset_id,
        &amount,
        booking_date,
        value_date,
    )
    .await?;

    Ok(())
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
