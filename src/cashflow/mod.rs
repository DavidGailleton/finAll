//! Cashflow: the shared DTOs and the server function the browser calls.
//!
//! An income → expense Sankey diagram for one chosen period, restricted to
//! `Bank` and `Cash` accounts (this app's narrower notion of "cashflow" as
//! opposed to net worth's every-account-type scope).
//!
//! The server-side implementation lives in [`crate::server::cashflow`] and is
//! only compiled with the `ssr` feature.

pub mod api;
pub mod types;
