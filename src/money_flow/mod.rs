//! Money flow: the shared DTOs and the server function the browser calls.
//!
//! A 12-calendar-month income/expense trend ending at the chosen `to` date's
//! month, plus the same income/expense/net totals as
//! [`crate::income_expense`] for one chosen period — both valued in one
//! display currency.
//!
//! The server-side implementation lives in [`crate::server::money_flow`] and
//! is only compiled with the `ssr` feature.

pub mod api;
pub mod types;
