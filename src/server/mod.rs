//! Server-only code. This module is compiled only with the `ssr` feature and
//! must never be reachable from hydration code.

pub mod accounts;
pub mod assets;
pub mod auth;
pub mod balances;
pub mod categories;
pub mod db;
pub mod error;
pub mod income_expense;
pub mod merchants;
pub mod net_worth;
pub mod seed;
pub mod transactions;
pub mod transfers;

#[cfg(test)]
mod test_support;
