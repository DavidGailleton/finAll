//! Server functions the browser calls to read and manage the signed-in user's
//! transactions.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::transactions::types::TransactionPage;

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
                amount: record.amount.to_string(),
                asset_id: record.asset_id.to_string(),
                asset_code: record.asset_code,
                booking_date: record.booking_date.to_string(),
                value_date: record.value_date.map(|date| date.to_string()),
                category_name: record.category_name,
                merchant_name: record.merchant_name,
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
