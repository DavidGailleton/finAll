//! The shared view over the `assets` spine: DTOs and the server functions the
//! browser calls. A "currency" is the fiat view over the spine; crypto and
//! securities will sit beside it here.
//!
//! The server-side implementation lives in [`crate::server::assets`] and is only
//! compiled with the `ssr` feature.

pub mod currency;
