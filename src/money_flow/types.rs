//! DTOs exchanged between the browser and the money-flow server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Money is
//! an exact decimal string, never a float; `month` is a `YYYY-MM` string.

use serde::{Deserialize, Serialize};

/// The whole money-flow report: a 12-calendar-month trend plus one period's
/// income/expense/net totals, all in one display currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoneyFlowReportDto {
    /// Normalized upper-case display currency code.
    pub display_currency_code: String,
    /// Minor-unit precision of the display currency.
    pub display_minor_units: i16,
    /// The 12 calendar months ending at the chosen `to` date's month, oldest
    /// first.
    pub months: Vec<MoneyFlowMonthDto>,
    /// Sum of the chosen `[from, to]` period's income groups, display
    /// currency (`>= 0`). Identical to the income-vs-expense report's own
    /// total for the same period.
    pub total_income: String,
    /// Sum of the chosen `[from, to]` period's expense groups (`<= 0`).
    pub total_expense: String,
    /// `total_income + total_expense`.
    pub net: String,
    /// `false` when at least one trend month or the period total had an
    /// unvalued (missing-rate or still-pending) amount excluded.
    pub complete: bool,
}

/// One month's income and expense magnitudes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoneyFlowMonthDto {
    /// `YYYY-MM`.
    pub month: String,
    /// Sum of that month's positive amounts, display currency (`>= 0`).
    pub income: String,
    /// Sum of that month's negative amounts, display currency (`<= 0`).
    pub expense: String,
}
