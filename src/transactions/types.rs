//! DTOs exchanged between the browser and the transaction server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type. Following the rest of the app, ids are strings; `amount`
//! is the exact stored decimal rendered as a string (never a float), and
//! `booking_date` is an ISO `YYYY-MM-DD` string.

use serde::{Deserialize, Serialize};

/// One of an account's transactions, in the shape the browser is allowed to
/// see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionDto {
    /// Transaction id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    /// The signed transaction amount, exactly as stored (`NUMERIC(38, 18)`),
    /// rendered as a decimal string. Never rounded, truncated, or parsed
    /// through a floating-point value.
    pub amount: String,
    /// Alphabetic code of the transaction's own currency, which may differ
    /// from the account's default currency.
    pub asset_code: String,
    /// Booking date as an ISO `YYYY-MM-DD` string.
    pub booking_date: String,
    /// The category name, if the transaction is categorised.
    pub category_name: Option<String>,
    /// The merchant name, if the transaction has one.
    pub merchant_name: Option<String>,
}
