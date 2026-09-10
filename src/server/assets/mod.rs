//! Server-only code backing the `assets` spine: the fiat-currency queries, the
//! asset-to-asset conversion primitive, and the opt-in ECB reference-rate
//! ingestion. Compiled only with the `ssr` feature and never reachable from
//! hydration code.

pub mod conversion;
pub mod currency;
pub mod rates;
pub mod schedule;
pub mod validate;
