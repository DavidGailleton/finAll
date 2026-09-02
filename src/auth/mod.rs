//! Authentication: shared DTOs and the server functions the browser calls.
//!
//! The server-side implementation (password hashing, sessions, cookies, database
//! access) lives in [`crate::server::auth`] and is only compiled with the `ssr`
//! feature.

pub mod api;
pub mod types;
