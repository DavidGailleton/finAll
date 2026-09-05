//! DTOs exchanged between the browser and the currency server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type.

use serde::{Deserialize, Serialize};

/// A currency, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurrencyDto {
    /// Currency id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    pub alphabetic_code: String,
    pub numeric_code: Option<String>,
    pub currency_name: String,
    pub symbol: Option<String>,
    pub minor_units: i16,
    pub is_active: bool,
}
