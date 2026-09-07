//! Server-only code backing the `assets` spine: the fiat-currency queries and
//! the asset-to-asset conversion primitive. Compiled only with the `ssr`
//! feature and never reachable from hydration code.

pub mod conversion;
pub mod currency;
pub mod validate;
