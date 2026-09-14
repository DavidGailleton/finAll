//! DTOs exchanged between the browser and the net-worth server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Following
//! the rest of the app, money is an exact decimal string, never a float, and
//! timestamps are strings.

use serde::{Deserialize, Serialize};

use crate::accounts::types::{AccountType, Classification};

/// The whole net-worth report, expressed in one display currency and grouped
/// by balance-sheet classification (assets vs liabilities).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetWorthReportDto {
    /// Alphabetic code the total is expressed in (normalized, upper-case).
    pub display_currency_code: String,
    /// Minor-unit precision of the display currency. Converted lines are
    /// rounded to this; the total is not (see `total`).
    pub display_minor_units: i16,
    /// Sum of every account that could be valued, as an exact decimal string.
    /// The position already in the display currency is added exact and
    /// unrounded; every other account contributes its converted amount. An
    /// account with no rate is excluded. Unaffected by classification.
    pub total: String,
    /// `false` when at least one account held a currency that could not be
    /// converted for lack of a rate and is therefore excluded from `total`.
    pub complete: bool,
    /// Earliest valuation timestamp among the converted accounts, as an RFC
    /// 3339 string, or `None` when no conversion was needed.
    pub rates_as_of: Option<String>,
    /// One group per balance-sheet side (assets, then liabilities).
    pub classifications: Vec<ClassificationGroupDto>,
}

/// One balance-sheet side, grouped further by account type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassificationGroupDto {
    pub classification: Classification,
    /// Sum of this classification's valued accounts, exact decimal string.
    pub total: String,
    /// One group per account type present in this classification, in
    /// `AccountType::ALL` order.
    pub account_groups: Vec<AccountGroupDto>,
}

/// One account type's accounts within a classification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountGroupDto {
    pub account_type: AccountType,
    /// Sum of this group's valued accounts, exact decimal string.
    pub total: String,
    pub accounts: Vec<NetWorthAccountLineDto>,
}

/// One account's contribution to net worth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetWorthAccountLineDto {
    /// Account id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub account_id: String,
    pub account_name: String,
    /// The native signed balance in the account's own currency, as an exact
    /// decimal string.
    pub amount: String,
    /// Alphabetic code of the currency this account is denominated in.
    pub currency_code: String,
    /// The balance valued in the display currency, or `None` when no rate was
    /// available. For an account already in the display currency this equals
    /// `amount`.
    pub converted_amount: Option<String>,
    /// Target-units-per-source-unit rate actually used, as an exact string.
    /// `None` for a display-currency account and for an unvalued account.
    pub rate: Option<String>,
    /// Valuation timestamp of `rate`, as an RFC 3339 string. `None` in the
    /// same cases as `rate`.
    pub valuation_timestamp: Option<String>,
    /// `true` when this account is already denominated in the display
    /// currency.
    pub is_display_currency: bool,
}
