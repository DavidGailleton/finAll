//! DTOs exchanged between the browser and the income-vs-expense server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Money is
//! an exact decimal string, never a float; dates are ISO `YYYY-MM-DD` strings.

use serde::{Deserialize, Serialize};

use crate::categories::types::CategoryKind;

/// The whole income-vs-expense report for one period, in one display currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncomeExpenseReportDto {
    /// Period start, echoed as `YYYY-MM-DD`, inclusive.
    pub from: String,
    /// Period end, `YYYY-MM-DD`, inclusive. Also the valuation date of every
    /// conversion in this report.
    pub to: String,
    /// Normalized upper-case display currency code.
    pub display_currency_code: String,
    /// Minor-unit precision of the display currency. Per-currency converted
    /// amounts are rounded to this; the group nets and totals are not.
    pub display_minor_units: i16,

    /// Groups whose converted net is positive, in encounter order (category
    /// name, then the uncategorised line last).
    pub income_lines: Vec<IncomeExpenseLineDto>,
    /// Groups whose converted net is negative.
    pub expense_lines: Vec<IncomeExpenseLineDto>,
    /// Groups that could not be given a sign: every currency part unvalued and
    /// more than one currency. Empty in the common case.
    pub unvalued_lines: Vec<IncomeExpenseLineDto>,

    /// Sum of every income group's converted net, exact decimal string, display
    /// currency (`>= 0`).
    pub total_income: String,
    /// Sum of every expense group's converted net (signed; `<= 0`).
    pub total_expense: String,
    /// `total_income + total_expense`. Not re-rounded, so it can carry more than
    /// `display_minor_units` decimals when a display-currency part does.
    pub net: String,

    /// `false` when at least one `(category, currency)` subtotal had no exchange
    /// rate as of `to` and is therefore excluded from the nets and totals.
    pub complete: bool,
    /// The valuation date every conversion used: the period-end date,
    /// `YYYY-MM-DD`.
    pub rates_as_of: String,
}

/// One group: a single category across all its currencies, or the uncategorised
/// bucket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncomeExpenseLineDto {
    /// Category id as a string; `None` for the uncategorised line.
    pub category_id: Option<String>,
    /// Category name; `None` when uncategorised or when the category has since
    /// been soft-deleted.
    pub category_name: Option<String>,
    /// The category's declared kind — a display badge only, not what decides
    /// the side. `None` for the uncategorised line.
    pub category_kind: Option<CategoryKind>,
    /// `true` when the category exists but is soft-deleted.
    pub category_deleted: bool,

    /// Group net in the display currency: the sum of the per-currency converted
    /// nets. `None` only when no currency part could be valued as of `to`. When
    /// some parts were valued and some were not, this is the partial sum and
    /// `complete` is `false`.
    pub converted_net: Option<String>,
    /// `false` when at least one currency part of this group had no as-of rate.
    pub complete: bool,

    /// Per-currency detail, one entry per currency the group has activity in.
    pub currencies: Vec<IncomeExpenseCurrencyDto>,
}

/// One currency's contribution to a group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncomeExpenseCurrencyDto {
    pub currency_code: String,
    /// Signed native net of this group's in-period, non-transfer transactions
    /// in this currency, exact decimal string.
    pub amount: String,
    /// `amount` valued in the display currency as of `to`, or `None` when no
    /// rate was available. Equals `amount` for the display currency itself.
    pub converted_amount: Option<String>,
    /// Target-units-per-source-unit rate used (as of `to`); `None` for the
    /// display currency and for an unvalued part.
    pub rate: Option<String>,
}
