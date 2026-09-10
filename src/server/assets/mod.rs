//! Server-only code backing the `assets` spine: the fiat-currency queries, the
//! asset-to-asset conversion primitive, on-demand exchange-rate resolution, and
//! the Frankfurter integration (its API client, the in-memory rate cache, and
//! the currency-list sync). Compiled only with the `ssr` feature and never
//! reachable from hydration code.

pub mod conversion;
pub mod currency;
pub mod frankfurter;
pub mod fx_cache;
pub mod fx_sync;
pub mod rates;
pub mod schedule;
pub mod validate;
