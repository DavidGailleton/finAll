//! Accounts: shared DTOs and the server functions the browser calls.
//!
//! An account is a container the signed-in user holds transactions in (a bank
//! account, a wallet, a card, ...). The server-side implementation (queries
//! against the `accounts` table) lives in [`crate::server::accounts`] and is
//! only compiled with the `ssr` feature.

pub mod api;
pub mod types;
