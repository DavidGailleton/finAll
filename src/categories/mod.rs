//! Categories: shared DTOs and the server functions the browser calls.
//!
//! A category is a user-owned label with an `income` or `expense` kind, unique
//! by name per user. The server-side implementation (queries against the
//! `categories` table) lives in [`crate::server::categories`] and is only
//! compiled with the `ssr` feature.

pub mod api;
pub mod types;
