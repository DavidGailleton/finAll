//! Currencies: shared DTOs and the server functions the browser calls.
//!
//! The server-side implementation (queries against the `assets` / `fiat_assets`
//! tables) lives in [`crate::server::assets::currency`] and is only compiled
//! with the `ssr` feature.

pub mod api;
pub mod types;
