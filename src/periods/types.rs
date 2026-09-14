//! DTOs and the preset list exchanged between the browser and the
//! period-resolution server function.
//!
//! Compiled for both targets, so no server-only type may appear here. Dates
//! are `YYYY-MM-DD` strings, same convention as every other report.

use serde::{Deserialize, Serialize};

/// A quick-pick relative period, resolved against the server's current date
/// (never the browser's clock — see [`crate::server::periods::resolve`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodPreset {
    ThisWeek,
    Last7Days,
    ThisMonth,
    Last30Days,
    LastMonth,
    Last90Days,
}

impl PeriodPreset {
    /// Every variant, in display order.
    pub const ALL: [PeriodPreset; 6] = [
        PeriodPreset::ThisWeek,
        PeriodPreset::Last7Days,
        PeriodPreset::ThisMonth,
        PeriodPreset::Last30Days,
        PeriodPreset::LastMonth,
        PeriodPreset::Last90Days,
    ];

    /// A label for display in the UI.
    pub fn label(&self) -> &'static str {
        match self {
            PeriodPreset::ThisWeek => "This week",
            PeriodPreset::Last7Days => "Last 7 days",
            PeriodPreset::ThisMonth => "This month",
            PeriodPreset::Last30Days => "Last 30 days",
            PeriodPreset::LastMonth => "Last month",
            PeriodPreset::Last90Days => "Last 90 days",
        }
    }
}

/// The resolved, inclusive `[from, to]` bounds for a preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodRangeDto {
    pub from: String,
    pub to: String,
}
