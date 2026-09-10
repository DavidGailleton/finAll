//! The server functions the browser calls to link two existing transactions as
//! a transfer, to break that link, and to list the transactions one could be
//! linked to.
//!
//! Each body runs only on the server (`ssr`); the server-only imports live
//! inside it so this module still compiles for the browser target, where the
//! function becomes a network call. Every argument is untrusted input.

use leptos::prelude::*;

use crate::transfers::types::TransferLinkOptions;

/// The transactions the edit form's "Paired transaction" field can offer for one
/// of the current user's transactions: the one it is already linked to (if any)
/// and the ones it could be linked to instead.
#[server]
pub async fn paired_transaction_options(
    transaction_id: String,
) -> Result<TransferLinkOptions, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transfers::{self, CandidateRow, TransferError};
    use crate::transfers::types::LinkCandidate;

    fn to_dto(row: CandidateRow) -> LinkCandidate {
        LinkCandidate {
            transaction_id: row.transaction_id.to_string(),
            label: row.label,
        }
    }

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let transaction_id = Uuid::parse_str(&transaction_id)
        .map_err(|_| TransferError::InvalidInput("invalid transaction id"))?;

    let options = transfers::link_options(&pool, user.user_id, transaction_id).await?;

    Ok(TransferLinkOptions {
        current_pair: options.current_pair.map(to_dto),
        candidates: options.candidates.into_iter().map(to_dto).collect(),
    })
}

/// Link two of the current user's existing transactions as the two legs of a
/// transfer. The outgoing (negative) transaction becomes the source and the
/// incoming (positive) one the destination.
#[server]
pub async fn link_transfer(
    transaction_id: String,
    paired_transaction_id: String,
) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transfers::{self, TransferError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let parse = |value: &str| {
        Uuid::parse_str(value).map_err(|_| TransferError::InvalidInput("invalid transaction id"))
    };

    transfers::link(
        &pool,
        user.user_id,
        parse(&transaction_id)?,
        parse(&paired_transaction_id)?,
    )
    .await?;

    Ok(())
}

/// Break the transfer that one of the current user's transactions is a leg of.
/// Both transactions stay in the ledger. Doing this when the transaction is not
/// part of a transfer is a no-op success.
#[server]
pub async fn unlink_transfer(transaction_id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::transfers::{self, TransferError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(TransferError::Unauthorized)?;

    let transaction_id = Uuid::parse_str(&transaction_id)
        .map_err(|_| TransferError::InvalidInput("invalid transaction id"))?;

    transfers::unlink(&pool, user.user_id, transaction_id).await?;

    Ok(())
}
