//! Merchants: shared DTOs and the server functions the browser calls.
//!
//! A merchant is a user-owned label, unique by name per user, with an optional
//! `default_category_id` pointing at one of the user's categories. The
//! server-side implementation (queries against the `merchants` table) lives in
//! [`crate::server::merchants`] and is only compiled with the `ssr` feature.

pub mod api;
pub mod types;
