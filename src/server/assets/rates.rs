//! On-demand resolution of an exchange rate for a currency pair.
//!
//! [`resolve_rate`] (latest) and [`resolve_rate_as_of`] (as of a past date) ask
//! [`FxRateCache`] for the direct `source -> target` quote Frankfurter carries
//! for that date — no reciprocal, no cross-rate pivot. A pair Frankfurter does
//! not cover, or a source it cannot serve, is
//! [`RateResolutionError::Unavailable`]; callers decide whether that fails the
//! request or merely degrades a line.
//!
//! Rates are never stored. The returned [`ResolvedRate`] carries the exact value
//! Frankfurter quoted and the reference instant it is anchored to (which can
//! precede the requested day when it falls on a weekend or holiday). A
//! *conversion* still applies the caller's explicit rounding policy — see
//! [`crate::server::assets::conversion`].

use bigdecimal::BigDecimal;
use leptos::logging;
use sqlx::types::chrono::{DateTime, NaiveDate, Utc};

use crate::server::assets::fx_cache::FxRateCache;

/// Why a rate could not be resolved for a currency pair. The message is safe to
/// show a user.
#[derive(Debug, thiserror::Error)]
pub enum RateResolutionError {
    #[error("no exchange rate available to value this account")]
    Unavailable,

    #[error("something went wrong")]
    Internal,
}

/// A resolved exchange rate expressed the way
/// [`convert`](crate::server::assets::conversion::convert) expects — target
/// units per source unit — with the instant the valuation is anchored to.
#[derive(Clone)]
pub struct ResolvedRate {
    pub rate: BigDecimal,
    pub valuation_timestamp: DateTime<Utc>,
}

/// Resolve a `source_code -> target_code` rate ("target units per source unit")
/// from Frankfurter's latest data, via [`FxRateCache`]. A pair Frankfurter does
/// not carry is [`RateResolutionError::Unavailable`].
pub async fn resolve_rate(
    cache: &FxRateCache,
    source_code: &str,
    target_code: &str,
) -> Result<ResolvedRate, RateResolutionError> {
    let today = Utc::now().date_naive();
    resolve(cache, source_code, target_code, today, true).await
}

/// Resolve a `source_code -> target_code` rate as it stood on `as_of`, via
/// [`FxRateCache`]. A same-day or future `as_of` uses the latest data.
pub async fn resolve_rate_as_of(
    cache: &FxRateCache,
    source_code: &str,
    target_code: &str,
    as_of: NaiveDate,
) -> Result<ResolvedRate, RateResolutionError> {
    let latest = as_of >= Utc::now().date_naive();
    resolve(cache, source_code, target_code, as_of, latest).await
}

async fn resolve(
    cache: &FxRateCache,
    source_code: &str,
    target_code: &str,
    date: NaiveDate,
    latest: bool,
) -> Result<ResolvedRate, RateResolutionError> {
    // Defensive: callers skip an identity conversion before resolving.
    if source_code == target_code {
        return Ok(ResolvedRate {
            rate: BigDecimal::from(1),
            valuation_timestamp: Utc::now(),
        });
    }

    match cache
        .direct_rate(source_code, target_code, date, latest)
        .await
    {
        Ok(Some((rate, valuation_timestamp))) => Ok(ResolvedRate {
            rate,
            valuation_timestamp,
        }),
        Ok(None) => Err(RateResolutionError::Unavailable),
        Err(err) => {
            logging::error!("fx rates: {source_code}->{target_code}: {err}");
            Err(RateResolutionError::Unavailable)
        }
    }
}
