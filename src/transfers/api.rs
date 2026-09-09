//! The server function the browser calls to create a transfer between two of
//! the signed-in user's accounts.
//!
//! The body runs only on the server (`ssr`); the server-only imports live
//! inside it so this module still compiles for the browser target, where the
//! function becomes a network call. Every argument is untrusted input.

use leptos::prelude::*;

use crate::transfers::types::{TransferDetailDto, TransferLeg};

/// Move money between two of the current user's accounts.
///
/// Each leg names its own account, currency, and positive amount; `source` is
/// debited and `destination` is credited. Nothing is converted — the two legs
/// may be in different currencies. `booking_date` is required (ISO
/// `YYYY-MM-DD`); `value_date` is optional. Both dates apply to both legs.
#[server]
pub async fn create_transfer(
    source: TransferLeg,
    destination: TransferLeg,
    booking_date: String,
    value_date: Option<String>,
) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions;
    use crate::server::transfers::{self, TransferError, TransferWrite};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let account_id = |value: &str| {
        Uuid::parse_str(value).map_err(|_| TransferError::InvalidInput("invalid account id"))
    };
    let asset_id = |value: &str| {
        Uuid::parse_str(value).map_err(|_| TransferError::InvalidInput("invalid currency id"))
    };

    let write = TransferWrite {
        source_account_id: account_id(&source.account_id)?,
        destination_account_id: account_id(&destination.account_id)?,
        source_asset_id: asset_id(&source.asset_id)?,
        destination_asset_id: asset_id(&destination.asset_id)?,
        source_amount: transfers::validate_amount(&source.amount)?,
        destination_amount: transfers::validate_amount(&destination.amount)?,
        booking_date: transactions::validate_booking_date(&booking_date)?,
        value_date: transactions::validate_value_date(value_date.as_deref())?,
    };

    transfers::create(&pool, user.user_id, &write).await?;

    Ok(())
}

/// Load one of the current user's transfers by id, for the edit form.
#[server]
pub async fn get_transfer(id: String) -> Result<TransferDetailDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transfers::{self, TransferError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| TransferError::InvalidInput("invalid transfer id"))?;

    let record = transfers::get(&pool, user.user_id, id).await?;

    Ok(TransferDetailDto {
        id: record.id.to_string(),
        source: TransferLeg {
            account_id: record.source_account_id.to_string(),
            asset_id: record.source_asset_id.to_string(),
            amount: record.source_amount.to_string(),
        },
        destination: TransferLeg {
            account_id: record.destination_account_id.to_string(),
            asset_id: record.destination_asset_id.to_string(),
            amount: record.destination_amount.to_string(),
        },
        booking_date: record.booking_date.to_string(),
        value_date: record.value_date.map(|date| date.to_string()),
    })
}

/// Update both legs of one of the current user's transfers: their currencies,
/// amounts, and dates. The two accounts cannot be changed here — void the
/// transfer and make a new one to move it.
#[server]
pub async fn update_transfer(
    id: String,
    source_asset_id: String,
    source_amount: String,
    destination_asset_id: String,
    destination_amount: String,
    booking_date: String,
    value_date: Option<String>,
) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transactions;
    use crate::server::transfers::{self, TransferEdit, TransferError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| TransferError::InvalidInput("invalid transfer id"))?;
    let asset_id = |value: &str| {
        Uuid::parse_str(value).map_err(|_| TransferError::InvalidInput("invalid currency id"))
    };

    let edit = TransferEdit {
        source_asset_id: asset_id(&source_asset_id)?,
        destination_asset_id: asset_id(&destination_asset_id)?,
        source_amount: transfers::validate_amount(&source_amount)?,
        destination_amount: transfers::validate_amount(&destination_amount)?,
        booking_date: transactions::validate_booking_date(&booking_date)?,
        value_date: transactions::validate_value_date(value_date.as_deref())?,
    };

    transfers::update(&pool, user.user_id, id, &edit).await?;

    Ok(())
}

/// Void one of the current user's transfers: both legs and the link are
/// soft-deleted together and the balances revert.
#[server]
pub async fn void_transfer(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transfers::{self, TransferError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| TransferError::InvalidInput("invalid transfer id"))?;

    transfers::void(&pool, user.user_id, id).await?;

    Ok(())
}
