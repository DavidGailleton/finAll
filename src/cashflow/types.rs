//! DTOs exchanged between the browser and the cashflow server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Money
//! (`value`, `total_income`, `total_expense`) is an exact decimal string,
//! never a float. Layout geometry (`x`, `y`, `width`, `height`, `path`) is
//! display-only pixel/SVG data, computed server-side — not a financial value,
//! so plain `f64`/`String` is fine for it (see
//! [`crate::server::cashflow::build_layout`]).

use serde::{Deserialize, Serialize};

/// An income → expense Sankey diagram for one period, restricted to `Bank` +
/// `Cash` accounts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashflowSankeyDto {
    pub display_currency_code: String,
    /// Sum of the shown income nodes' values, display currency (`>= 0`).
    pub total_income: String,
    /// Sum of the shown expense nodes' values, display currency (`<= 0`).
    pub total_expense: String,
    /// `false` when at least one category was excluded from the diagram for
    /// lack of an exchange rate.
    pub complete: bool,
    pub nodes: Vec<SankeyNodeDto>,
    pub links: Vec<SankeyLinkDto>,
    /// The `<svg viewBox>` size the layout was computed for.
    pub width: f64,
    pub height: f64,
}

/// Which side of the diagram a node or link belongs to (for coloring).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SankeySide {
    Income,
    Hub,
    Expense,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SankeyNodeDto {
    pub label: String,
    /// The category's magnitude, exact decimal string (`>= 0`); the hub node
    /// has no single amount of its own and uses `"0"`.
    pub value: String,
    pub side: SankeySide,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SankeyLinkDto {
    /// The link's magnitude, exact decimal string (`>= 0`).
    pub value: String,
    pub side: SankeySide,
    /// A closed SVG path (`d` attribute) for the filled ribbon.
    pub path: String,
}
