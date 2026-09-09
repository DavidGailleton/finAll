//! The server function the browser calls to create a transfer between two of
//! the signed-in user's accounts.
//!
//! The body runs only on the server (`ssr`); the server-only imports live
//! inside it so this module still compiles for the browser target, where the
//! function becomes a network call. Every argument is untrusted input.

use leptos::prelude::*;

use crate::transfers::types::TransferLeg;

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
