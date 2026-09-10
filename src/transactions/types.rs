//! DTOs exchanged between the browser and the transaction server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type. Following the rest of the app, ids are strings; `amount`
//! is the exact stored decimal rendered as a string (never a float), and the
//! dates are ISO `YYYY-MM-DD` strings.

use serde::{Deserialize, Serialize};

/// One of the user's transactions, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionDto {
    /// Transaction id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    /// Id of the account this transaction is on, rendered as a string.
    pub account_id: String,
    /// Name of that account. Shown on the global transaction list; the
    /// per-account list ignores it.
    pub account_name: String,
    /// The signed transaction amount, exactly as stored (`NUMERIC(38, 18)`),
    /// rendered as a decimal string. Never rounded, truncated, or parsed
    /// through a floating-point value.
    pub amount: String,
    /// Id of the transaction's own currency (a fiat `assets` row), rendered as
    /// a string for the same reason as `id`. May differ from the account's
    /// default currency; used to pre-select the edit form's currency field.
    pub asset_id: String,
    /// Alphabetic code of the transaction's own currency, which may differ
    /// from the account's default currency.
    pub asset_code: String,
    /// Booking date as an ISO `YYYY-MM-DD` string.
    pub booking_date: String,
    /// Value date as an ISO `YYYY-MM-DD` string, if one is recorded.
    pub value_date: Option<String>,
    /// Id of the transaction's category, rendered as a string, if it is
    /// categorised. Used to pre-select the edit form's category field.
    pub category_id: Option<String>,
    /// The category name, if the transaction is categorised and the category is
    /// still active.
    pub category_name: Option<String>,
    /// Id of the transaction's merchant, rendered as a string, if it has one.
    /// Used to pre-select the edit form's merchant field.
    pub merchant_id: Option<String>,
    /// The merchant name, if the transaction has one and the merchant is still
    /// active.
    pub merchant_name: Option<String>,
    /// Id of the transfer this transaction is a leg of, if any. When set, the
    /// row is shown as a transfer (the transaction is still edited and deleted
    /// on its own).
    pub transfer_id: Option<String>,
    /// Name of the account on the other leg of that transfer, for display.
    pub transfer_counterparty: Option<String>,
}

/// One page of a transaction list, plus the opaque cursor to fetch the next
/// page (absent when the last row has been reached).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionPage {
    pub transactions: Vec<TransactionDto>,
    pub next_cursor: Option<String>,
}
