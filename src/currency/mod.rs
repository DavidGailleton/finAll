//! Currencies: shared DTOs and the server functions the browser calls.
//!
//! The server-side implementation (queries against the `currencies` table)
//! lives in [`crate::server::currency`] and is only compiled with the `ssr`
//! feature.

pub mod api;
pub mod types;
