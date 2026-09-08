//! Transactions: shared DTOs and the server functions the browser calls.
//!
//! A transaction is a dated, signed movement of one asset on one of the user's
//! accounts. The server-side implementation (queries against the `transactions`
//! table) lives in [`crate::server::transactions`] and is only compiled with
//! the `ssr` feature.

pub mod api;
pub mod types;
