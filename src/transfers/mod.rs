//! Transfers: the server-function surface for linking two of the user's
//! existing transactions as a movement of money between accounts. The database
//! work itself lives in `crate::server::transfers`.

pub mod types;

pub mod api;
