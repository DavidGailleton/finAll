//! Income vs expense over an explicit period, grouped by category.
//!
//! For a signed-in user and an inclusive `[from, to]` booking-date window, the
//! signed sum of every non-transfer transaction is aggregated per category and
//! per currency and valued into a chosen display currency **at the exchange
//! rate in force on the period-end date**. Each group is then placed by the
//! sign of its converted net: a positive net is income, a negative net is an
//! expense — the category's own `kind` is only a label. A `(category, currency)`
//! subtotal with no as-of rate is reported without a converted value and left
//! out of the totals rather than guessed at.
//!
//! Server-side implementation: [`crate::server::income_expense`] (`ssr` only).

pub mod api;
pub mod types;
