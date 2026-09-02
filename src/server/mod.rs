//! Server-only code. This module is compiled only with the `ssr` feature and
//! must never be reachable from hydration code.

pub mod auth;
pub mod db;
pub mod error;
