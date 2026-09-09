//! DTOs for the transfer server function. Compiled for both targets, so no
//! server-only types. Ids and amounts cross the wire as strings, like the rest
//! of the app.

use serde::{Deserialize, Serialize};

/// One side of a transfer: which account, in which currency, and how much
/// (a positive magnitude). `asset_id` must be an active fiat currency but may
/// differ from the account's default, exactly like any transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferLeg {
    pub account_id: String,
    pub asset_id: String,
    pub amount: String,
}

/// One of the user's transfers, assembled from its two legs, for the edit form.
/// The `amount` on each leg is a positive magnitude.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferDetailDto {
    pub id: String,
    pub source: TransferLeg,
    pub destination: TransferLeg,
    pub booking_date: String,
    pub value_date: Option<String>,
}
