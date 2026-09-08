//! The DTO exchanged between the browser and the balance server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Following
//! the rest of the app, `amount` is an exact decimal string, never a float.

use serde::{Deserialize, Serialize};

/// An account's balance as a single number in the account's default currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountBalanceDto {
    /// The balance, valued in the account's default currency and rendered as an
    /// exact decimal string. A part already in the default currency is exact; a
    /// converted part is rounded to that currency's minor units.
    pub amount: String,
    /// Alphabetic code of the account's default currency.
    pub currency_code: String,
}
