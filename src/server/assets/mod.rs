//! Server-only code backing the `assets` spine: the fiat-currency queries, the
//! asset-to-asset conversion primitive, read-time exchange-rate resolution, and
//! the opt-in Frankfurter integration (its API client and the persistence that
//! feeds `asset_rates` / the currency list). Compiled only with the `ssr`
//! feature and never reachable from hydration code.

pub mod conversion;
pub mod currency;
pub mod frankfurter;
pub mod fx_sync;
pub mod rates;
pub mod schedule;
pub mod validate;
