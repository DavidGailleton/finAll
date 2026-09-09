//! The server function the browser calls to create a transfer between two of
//! the signed-in user's accounts.
//!
//! The body runs only on the server (`ssr`); the server-only imports live
//! inside it so this module still compiles for the browser target, where the
//! function becomes a network call. Every argument is untrusted input.

use leptos::prelude::*;

/// Move money between two of the current user's accounts.
///
/// `source_amount` is the positive magnitude leaving the source account (in its
/// currency); `destination_amount` is the positive magnitude arriving at the
/// destination account (in its currency). Nothing is converted. `booking_date`
/// is required (ISO `YYYY-MM-DD`); `value_date` is optional. Both dates apply to
/// both legs.
#[server]
pub async fn create_transfer(
    source_account_id: String,
    destination_account_id: String,
    source_amount: String,
    destination_amount: String,
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

    let parse_account = |value: &str| {
        Uuid::parse_str(value).map_err(|_| TransferError::InvalidInput("invalid account id"))
    };

    let write = TransferWrite {
        source_account_id: parse_account(&source_account_id)?,
        destination_account_id: parse_account(&destination_account_id)?,
        source_amount: transfers::validate_amount(&source_amount)?,
        destination_amount: transfers::validate_amount(&destination_amount)?,
        booking_date: transactions::validate_booking_date(&booking_date)?,
        value_date: transactions::validate_value_date(value_date.as_deref())?,
    };

    transfers::create(&pool, user.user_id, &write).await?;

    Ok(())
}
