//! Net worth: the shared DTOs and the server function the browser calls.
//!
//! The signed-in user's balance across every account, re-aggregated to one
//! signed position per currency (the `account_balances` SQL view), each valued
//! into a chosen display currency with an explicitly resolved exchange rate and
//! valuation timestamp. A currency with no resolvable rate is reported as an
//! unvalued line and excluded from the total rather than guessed at.
//!
//! The server-side implementation lives in [`crate::server::net_worth`] and is
//! only compiled with the `ssr` feature.

pub mod api;
pub mod types;
