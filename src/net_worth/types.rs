//! DTOs exchanged between the browser and the net-worth server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Following
//! the rest of the app, money is an exact decimal string, never a float, and
//! timestamps are strings.

use serde::{Deserialize, Serialize};

/// The whole net-worth report, expressed in one display currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetWorthReportDto {
    /// Alphabetic code the total is expressed in (normalized, upper-case).
    pub display_currency_code: String,
    /// Minor-unit precision of the display currency. Converted lines are
    /// rounded to this; the total is not (see `total`).
    pub display_minor_units: i16,
    /// Sum of every line that could be valued, as an exact decimal string. The
    /// position already in the display currency is added exact and unrounded;
    /// every other line contributes its `converted_amount`. A line with no
    /// rate is excluded.
    pub total: String,
    /// `false` when at least one currency held could not be converted for lack
    /// of a rate and is therefore excluded from `total`.
    pub complete: bool,
    /// Earliest valuation timestamp among the converted lines, as an RFC 3339
    /// string, or `None` when no conversion was needed.
    pub rates_as_of: Option<String>,
    /// One line per currency the user holds, ordered by currency code.
    pub lines: Vec<NetWorthLineDto>,
}

/// One currency's contribution to net worth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetWorthLineDto {
    /// Alphabetic code of the currency this line is denominated in.
    pub currency_code: String,
    /// The native signed position in this currency, as an exact decimal string.
    pub amount: String,
    /// The position valued in the display currency, or `None` when no rate was
    /// available. For the display-currency line this equals `amount`.
    pub converted_amount: Option<String>,
    /// Target-units-per-source-unit rate actually used, as an exact string.
    /// `None` for the display-currency line and for an unvalued line.
    pub rate: Option<String>,
    /// Valuation timestamp of `rate`, as an RFC 3339 string. `None` in the same
    /// cases as `rate`.
    pub valuation_timestamp: Option<String>,
    /// `true` for the single line already in the display currency.
    pub is_display_currency: bool,
}
