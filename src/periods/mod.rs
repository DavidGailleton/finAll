//! Period presets: the shared preset list and the server function the
//! browser calls to resolve one into concrete dates.
//!
//! The server-side date arithmetic lives in [`crate::server::periods`] and is
//! only compiled with the `ssr` feature.

pub mod api;
pub mod types;
