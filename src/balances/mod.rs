//! Balances: the shared DTO and the server function the browser calls.
//!
//! An account's balance is the signed sum of its transactions per currency (the
//! `account_balances` SQL view), valued into the account's default currency.
//! The server-side implementation lives in [`crate::server::balances`] and is
//! only compiled with the `ssr` feature.

pub mod api;
pub mod types;
