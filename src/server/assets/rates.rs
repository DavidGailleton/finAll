//! Read-time resolution of an exchange rate for a currency pair from the
//! observations stored in `asset_rates`.
//!
//! [`resolve_rate`] (latest) and [`resolve_rate_as_of`] (as of a date) pick the
//! observations that could form a `source -> target` rate and hand them to the
//! pure [`derive_rate`], which applies a fixed precedence: a direct observation
//! used as stored, else an inverse observation reciprocated, else an `EUR` pivot
//! (`(EUR -> target) / (EUR -> source)`). Deriving a rate needs a rounding
//! policy, so the reciprocal and the pivot division are done at a named working
//! precision here; a *conversion* still takes the caller's explicit policy (see
//! [`crate::server::assets::conversion`]).
//!
//! The observations themselves are written by
//! [`crate::server::assets::fx_sync`] from data fetched by
//! [`crate::server::assets::frankfurter`]; this module only reads them.

use std::num::NonZeroU64;

use bigdecimal::{BigDecimal, Context, RoundingMode, Zero};
use leptos::logging;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;

/// The active fiat asset used as the cross-rate pivot ([`pivot_asset_id`]).
const BASE_CODE: &str = "EUR";

/// Working precision for derived-rate arithmetic (the reciprocal and the EUR
/// pivot division): 34 significant digits, banker's rounding. `bigdecimal`
/// 0.4's build-time default rounding mode is not guaranteed, so every rounded
/// step below names its mode explicitly rather than relying on `Context::default`.
const RATE_WORKING_PRECISION: NonZeroU64 = match NonZeroU64::new(34) {
    Some(precision) => precision,
    None => unreachable!(),
};
const RATE_WORKING_ROUNDING: RoundingMode = RoundingMode::HalfEven;

/// Why a rate could not be resolved for a currency pair. The message is safe to
/// show a user.
#[derive(Debug, thiserror::Error)]
pub enum RateResolutionError {
    #[error("no exchange rate available to value this account")]
    Unavailable,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for RateResolutionError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("fx rates: database error: {err}");
        RateResolutionError::Internal
    }
}

/// One `asset_rates` observation for a currency pair, already picked as the
/// latest by [`latest_rate`] (`ORDER BY observed_at DESC, created_at DESC`).
pub struct RateObservation {
    /// Quote units per base unit, exactly as stored (`asset_rates.rate`),
    /// guaranteed `> 0` by the `asset_rates_rate_positive` check.
    pub rate: BigDecimal,
    pub observed_at: DateTime<Utc>,
}

/// A resolved exchange rate expressed the way
/// [`convert`](crate::server::assets::conversion::convert) expects — target
/// units per source unit — with the instant the valuation is anchored to.
pub struct ResolvedRate {
    pub rate: BigDecimal,
    pub valuation_timestamp: DateTime<Utc>,
}

/// Combine the observations available for a `source -> target` pair into a
/// single "target units per source unit" rate, applying the precedence:
///
/// 1. a direct `source -> target` observation, used exactly as stored;
/// 2. an inverse `target -> source` observation, reciprocated;
/// 3. an EUR pivot: `(EUR -> target) / (EUR -> source)`.
///
/// Returns `None` when none of those can be formed (including a zero
/// observation, which cannot be inverted or divided by). Pure: the caller
/// fetches the observations — which is also where "latest", and later an as-of
/// date, is decided — and this only does the arithmetic.
pub fn derive_rate(
    direct: Option<RateObservation>,
    inverse: Option<RateObservation>,
    pivot_to_source: Option<RateObservation>,
    pivot_to_target: Option<RateObservation>,
) -> Option<ResolvedRate> {
    if let Some(direct) = direct {
        return Some(ResolvedRate {
            rate: direct.rate,
            valuation_timestamp: direct.observed_at,
        });
    }

    if let Some(inverse) = inverse {
        if inverse.rate.is_zero() {
            return None;
        }
        let context = Context::new(RATE_WORKING_PRECISION, RATE_WORKING_ROUNDING);
        return Some(ResolvedRate {
            rate: inverse.rate.inverse_with_context(&context),
            valuation_timestamp: inverse.observed_at,
        });
    }

    let (pivot_to_source, pivot_to_target) = (pivot_to_source?, pivot_to_target?);
    if pivot_to_source.rate.is_zero() {
        return None;
    }
    let rate = (&pivot_to_target.rate / &pivot_to_source.rate)
        .with_precision_round(RATE_WORKING_PRECISION, RATE_WORKING_ROUNDING);

    Some(ResolvedRate {
        rate,
        valuation_timestamp: pivot_to_source.observed_at.min(pivot_to_target.observed_at),
    })
}

/// Resolve a `source_asset_id -> target_asset_id` rate ("target units per
/// source unit"). Reads the latest observations that could form the rate and
/// applies [`derive_rate`]; a pair with no usable observation is
/// [`RateResolutionError::Unavailable`].
pub async fn resolve_rate(
    pool: &PgPool,
    source_asset_id: Uuid,
    target_asset_id: Uuid,
) -> Result<ResolvedRate, RateResolutionError> {
    let direct = latest_rate(pool, source_asset_id, target_asset_id).await?;
    let inverse = latest_rate(pool, target_asset_id, source_asset_id).await?;

    let (pivot_to_source, pivot_to_target) = match pivot_asset_id(pool).await? {
        Some(pivot_id) => (
            latest_rate(pool, pivot_id, source_asset_id).await?,
            latest_rate(pool, pivot_id, target_asset_id).await?,
        ),
        None => (None, None),
    };

    derive_rate(direct, inverse, pivot_to_source, pivot_to_target)
        .ok_or(RateResolutionError::Unavailable)
}

/// Resolve a `source_asset_id -> target_asset_id` rate as it stood on `as_of`:
/// the latest observations dated on or before `as_of` that could form the rate,
/// combined by [`derive_rate`] with the same direct / inverse / EUR-pivot
/// precedence as [`resolve_rate`]. A pair with no usable observation in that
/// window is [`RateResolutionError::Unavailable`].
///
/// `as_of` is compared against `asset_rates.observed_at`, which is stored at
/// `00:00:00Z` of a reference date; pass a period boundary at `00:00:00Z` too
/// so an observation dated exactly on that day is included.
pub async fn resolve_rate_as_of(
    pool: &PgPool,
    source_asset_id: Uuid,
    target_asset_id: Uuid,
    as_of: DateTime<Utc>,
) -> Result<ResolvedRate, RateResolutionError> {
    let direct = latest_rate_as_of(pool, source_asset_id, target_asset_id, as_of).await?;
    let inverse = latest_rate_as_of(pool, target_asset_id, source_asset_id, as_of).await?;

    let (pivot_to_source, pivot_to_target) = match pivot_asset_id(pool).await? {
        Some(pivot_id) => (
            latest_rate_as_of(pool, pivot_id, source_asset_id, as_of).await?,
            latest_rate_as_of(pool, pivot_id, target_asset_id, as_of).await?,
        ),
        None => (None, None),
    };

    derive_rate(direct, inverse, pivot_to_source, pivot_to_target)
        .ok_or(RateResolutionError::Unavailable)
}

/// The latest non-deleted observation for one `(base, quote)` pair, or `None`.
async fn latest_rate(
    pool: &PgPool,
    base_asset_id: Uuid,
    quote_asset_id: Uuid,
) -> Result<Option<RateObservation>, sqlx::Error> {
    sqlx::query_as!(
        RateObservation,
        r#"
        SELECT rate, observed_at
        FROM asset_rates
        WHERE
            base_asset_id = $1
            AND quote_asset_id = $2
            AND deleted_at IS NULL
        ORDER BY observed_at DESC, created_at DESC
        LIMIT 1
        "#,
        base_asset_id,
        quote_asset_id,
    )
    .fetch_optional(pool)
    .await
}

/// The latest non-deleted observation for one `(base, quote)` pair dated on or
/// before `as_of`, or `None`. Time-bounded sibling of [`latest_rate`] — the
/// "as-of date" that [`derive_rate`]'s contract anticipates.
async fn latest_rate_as_of(
    pool: &PgPool,
    base_asset_id: Uuid,
    quote_asset_id: Uuid,
    as_of: DateTime<Utc>,
) -> Result<Option<RateObservation>, sqlx::Error> {
    sqlx::query_as!(
        RateObservation,
        r#"
        SELECT rate, observed_at
        FROM asset_rates
        WHERE
            base_asset_id = $1
            AND quote_asset_id = $2
            AND deleted_at IS NULL
            AND observed_at <= $3
        ORDER BY observed_at DESC, created_at DESC
        LIMIT 1
        "#,
        base_asset_id,
        quote_asset_id,
        as_of,
    )
    .fetch_optional(pool)
    .await
}

/// The id of the active, non-deleted fiat asset used as the cross-rate pivot
/// (`EUR`), or `None` if it is not present. `assets_class_code_unique` makes
/// the match unique.
async fn pivot_asset_id(pool: &PgPool) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT id
        FROM assets
        WHERE
            asset_class = 'fiat'
            AND code = $1
            AND is_active = TRUE
            AND deleted_at IS NULL
        "#,
        BASE_CODE,
    )
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod resolution_tests {
    use std::str::FromStr;

    use super::*;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid fixed timestamp")
    }

    fn obs(rate: &str, secs: i64) -> RateObservation {
        RateObservation {
            rate: dec(rate),
            observed_at: at(secs),
        }
    }

    #[test]
    fn direct_rate_is_preferred_and_used_as_is() {
        let resolved = derive_rate(
            Some(obs("1.25", 100)),
            Some(obs("9", 200)),
            Some(obs("3", 300)),
            Some(obs("4", 400)),
        )
        .expect("a rate is derived");
        assert_eq!(resolved.rate, dec("1.25"));
        assert_eq!(resolved.valuation_timestamp, at(100));
    }

    #[test]
    fn inverse_rate_is_the_reciprocal() {
        let resolved =
            derive_rate(None, Some(obs("2", 200)), None, None).expect("a rate is derived");
        assert_eq!(resolved.rate, dec("0.5"));
        assert_eq!(resolved.valuation_timestamp, at(200));
    }

    #[test]
    fn inverse_is_preferred_over_the_pivot() {
        let resolved = derive_rate(
            None,
            Some(obs("4", 200)),
            Some(obs("1.1", 300)),
            Some(obs("1.32", 400)),
        )
        .expect("a rate is derived");
        assert_eq!(resolved.rate, dec("0.25"));
    }

    #[test]
    fn pivot_divides_eur_to_target_by_eur_to_source() {
        let resolved = derive_rate(None, None, Some(obs("1.10", 300)), Some(obs("1.32", 400)))
            .expect("a rate is derived");
        assert_eq!(resolved.rate, dec("1.2"));
        assert_eq!(resolved.valuation_timestamp, at(300));
    }

    #[test]
    fn pivot_anchors_to_the_earlier_observation() {
        let resolved = derive_rate(None, None, Some(obs("1.10", 500)), Some(obs("1.32", 250)))
            .expect("a rate is derived");
        assert_eq!(resolved.valuation_timestamp, at(250));
    }

    #[test]
    fn a_reciprocal_that_does_not_terminate_is_rounded_down_at_the_working_precision() {
        // 1 / 3 to 34 significant digits: the 35th digit is 3, so it rounds down.
        let resolved =
            derive_rate(None, Some(obs("3", 10)), None, None).expect("a rate is derived");
        assert_eq!(resolved.rate, dec("0.3333333333333333333333333333333333"));
    }

    #[test]
    fn a_pivot_quotient_that_does_not_terminate_is_rounded_up_at_the_working_precision() {
        // 2 / 3 to 34 significant digits: the 35th digit is 6, so it rounds up.
        let resolved = derive_rate(None, None, Some(obs("3", 1)), Some(obs("2", 2)))
            .expect("a rate is derived");
        assert_eq!(resolved.rate, dec("0.6666666666666666666666666666666667"));
    }

    #[test]
    fn no_observations_yields_no_rate() {
        assert!(derive_rate(None, None, None, None).is_none());
    }

    #[test]
    fn a_pivot_needs_both_legs() {
        assert!(derive_rate(None, None, Some(obs("1.1", 1)), None).is_none());
        assert!(derive_rate(None, None, None, Some(obs("1.3", 1))).is_none());
    }

    #[test]
    fn a_zero_observation_is_rejected_without_panicking() {
        assert!(derive_rate(None, Some(obs("0", 1)), None, None).is_none());
        assert!(derive_rate(None, None, Some(obs("0", 1)), Some(obs("1.3", 2))).is_none());
    }
}

#[cfg(test)]
mod as_of_tests {
    use sqlx::types::chrono::NaiveDate;

    use super::*;
    use crate::server::test_support::{currency_id, dec};

    fn midnight(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(year, month, day)
            .expect("valid calendar date")
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time");
        DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    }

    async fn insert_observation(
        pool: &PgPool,
        base_asset_id: Uuid,
        quote_asset_id: Uuid,
        rate: &str,
        observed_at: DateTime<Utc>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO asset_rates
                (base_asset_id, quote_asset_id, rate, rate_source, observed_at)
            VALUES ($1, $2, $3, 'test', $4)
            "#,
        )
        .bind(base_asset_id)
        .bind(quote_asset_id)
        .bind(dec(rate))
        .bind(observed_at)
        .execute(pool)
        .await
        .expect("insert test observation");
    }

    #[sqlx::test]
    async fn picks_the_latest_observation_on_or_before_the_cutoff(pool: PgPool) {
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        insert_observation(&pool, eur, usd, "1.10", midnight(2026, 1, 1)).await;
        insert_observation(&pool, eur, usd, "1.20", midnight(2026, 2, 1)).await;

        let resolved = resolve_rate_as_of(&pool, eur, usd, midnight(2026, 1, 15))
            .await
            .expect("a rate as of mid-January");

        assert_eq!(resolved.rate, dec("1.10"));
        assert_eq!(resolved.valuation_timestamp, midnight(2026, 1, 1));
    }

    #[sqlx::test]
    async fn an_observation_dated_exactly_on_the_cutoff_is_included(pool: PgPool) {
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        insert_observation(&pool, eur, usd, "1.15", midnight(2026, 3, 31)).await;

        let resolved = resolve_rate_as_of(&pool, eur, usd, midnight(2026, 3, 31))
            .await
            .expect("the boundary observation is usable");
        assert_eq!(resolved.rate, dec("1.15"));
    }

    #[sqlx::test]
    async fn nothing_before_the_cutoff_is_unavailable(pool: PgPool) {
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        insert_observation(&pool, eur, usd, "1.20", midnight(2026, 2, 1)).await;

        let result = resolve_rate_as_of(&pool, eur, usd, midnight(2026, 1, 1)).await;
        assert!(matches!(result, Err(RateResolutionError::Unavailable)));
    }

    #[sqlx::test]
    async fn an_inverse_only_pair_resolves_to_the_reciprocal(pool: PgPool) {
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        // Only EUR -> USD is stored; ask for USD -> EUR as of a later date.
        insert_observation(&pool, eur, usd, "2", midnight(2026, 1, 1)).await;

        let resolved = resolve_rate_as_of(&pool, usd, eur, midnight(2026, 6, 1))
            .await
            .expect("the reciprocal is usable");
        assert_eq!(resolved.rate, dec("0.5"));
        assert_eq!(resolved.valuation_timestamp, midnight(2026, 1, 1));
    }
}
