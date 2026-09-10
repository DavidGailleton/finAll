//! Opt-in ingestion of reference rates from Frankfurter (`api.frankfurter.dev`,
//! v2 API; override the host with `FRANKFURTER_URL`).
//!
//! [`ingest`] is the single entry point, called daily by the background task in
//! `main.rs` when `FETCH_FX_RATES` is set. For every currency the user actually
//! holds (plus `EUR`) it fetches `GET /v2/rates?base=<code>` and writes one
//! `asset_rates` row per quote currency it can resolve to an active fiat asset:
//! `base_asset_id` is that base currency, `quote_asset_id` the other, `rate` the
//! exact value Frankfurter returned (parsed straight from the response text into
//! a [`BigDecimal`], never through `f64`).
//!
//! `observed_at` is a rate's reference *date* at `00:00:00Z` — a date, not a
//! wall-clock publication instant. The v2 API returns each currency's latest
//! available date independently, so a single run can produce several distinct
//! `observed_at` values. Re-running for a reference date already stored is a
//! no-op: the natural key `(base_asset_id, quote_asset_id, rate_source,
//! observed_at)` already holds the row and the insert is `ON CONFLICT DO
//! NOTHING`, so a stored observation is never mutated.
//!
//! Rates are only ever stored as Frankfurter quotes them (`base -> quote`);
//! cross-rates and reciprocals are derived at read time by [`resolve_rate`],
//! never precomputed here, because a conversion needs an explicit rounding
//! policy the caller supplies (see [`crate::server::assets::conversion`]).

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::str::FromStr;
use std::time::Duration;

use bigdecimal::{BigDecimal, Context, RoundingMode, Signed, Zero};
use leptos::logging;
use sqlx::types::chrono::{DateTime, NaiveDate, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;

/// `rate_source` tag stored on every row this module writes.
const RATE_SOURCE: &str = "frankfurter.dev";
/// The Frankfurter API host used unless `FRANKFURTER_URL` overrides it.
const DEFAULT_API_ROOT: &str = "https://api.frankfurter.dev";
/// The currency always fetched as a base, in addition to the ones in use: it is
/// the read-time cross-rate pivot ([`pivot_asset_id`]) and the historical anchor.
const BASE_CODE: &str = "EUR";

/// The Frankfurter API root (no trailing slash): `FRANKFURTER_URL` when set and
/// non-blank — point it at a self-hosted instance or mirror serving the same v2
/// `/v2/rates` and `/v2/currencies` shapes — otherwise [`DEFAULT_API_ROOT`].
/// Shared with [`crate::server::assets::currency`]'s currency sync.
pub fn api_root() -> String {
    let root = std::env::var("FRANKFURTER_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_ROOT.to_owned());
    root.trim_end_matches('/').to_owned()
}

/// Upper bound on a single fetch; a slow or hanging source must not wedge the task.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay between successive per-base fetches, so a run of several bases does not
/// hammer the source.
const BASE_FETCH_SPACING: Duration = Duration::from_millis(300);

/// Backoff before each retry of one base's fetch (a timeout or 5xx); its length
/// is the retry count, so a base gets `1 + len` attempts before it is skipped.
const FETCH_RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

/// Why an ingestion run could not complete.
///
/// The messages are safe to surface; today the only caller is the background
/// task, which logs and retries on the next tick.
#[derive(Debug, thiserror::Error)]
pub enum RatesError {
    #[error("could not reach the exchange-rate source")]
    Fetch,

    #[error("{0}")]
    Parse(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<reqwest::Error> for RatesError {
    fn from(err: reqwest::Error) -> Self {
        logging::error!("fx rates: fetch error: {err}");
        RatesError::Fetch
    }
}

impl From<sqlx::Error> for RatesError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("fx rates: database error: {err}");
        RatesError::Internal
    }
}

/// One row of the Frankfurter v2 `/v2/rates` response, which is a JSON array of
/// these. `rate` is kept as its raw JSON text so it can be parsed into an exact
/// decimal rather than through `f64`.
#[derive(serde::Deserialize)]
struct RateRow {
    date: String,
    base: String,
    quote: String,
    rate: Box<serde_json::value::RawValue>,
}

/// A validated rate row: its reference date (at `00:00:00Z`), the quote currency
/// code, and an exact positive `base -> quote` rate. The base is tracked by the
/// caller — [`ingest`] fetches one base at a time.
struct ReferenceRate {
    observed_at: DateTime<Utc>,
    quote_code: String,
    rate: BigDecimal,
}

/// What one [`ingest`] run did, for the log line.
pub struct IngestSummary {
    /// Rows newly written to `asset_rates` (an observation that already existed
    /// counts zero).
    pub inserted: u64,
    /// Base currencies whose fetch and store both completed.
    pub bases_ok: usize,
    /// Base currencies skipped this run after their fetch kept failing.
    pub bases_failed: usize,
}

/// Fetch the latest rates for every currency the user holds (plus `EUR`) as the
/// base, and store the ones that resolve to an active fiat asset.
///
/// Each base is fetched and stored on its own: a base whose fetch keeps failing
/// is logged and skipped without aborting the others, and each base's rows are
/// one all-or-nothing transaction. Idempotent via `ON CONFLICT DO NOTHING`.
/// Returns [`RatesError`] only for a database failure that stops the run before
/// it starts.
pub async fn ingest(pool: &PgPool) -> Result<IngestSummary, RatesError> {
    let bases = sqlx::query!(
        r#"
        SELECT a.id, a.code
        FROM assets AS a
        WHERE a.asset_class = 'fiat'
          AND a.is_active = TRUE
          AND a.deleted_at IS NULL
          AND (
              a.code = $1
              OR a.id IN (
                  SELECT default_asset_id FROM accounts WHERE deleted_at IS NULL
                  UNION
                  SELECT asset_id FROM transactions WHERE deleted_at IS NULL
              )
          )
        ORDER BY a.code
        "#,
        BASE_CODE,
    )
    .fetch_all(pool)
    .await?;

    if bases.is_empty() {
        logging::log!("fx rates: no active base currency; skipping run");
        return Ok(IngestSummary {
            inserted: 0,
            bases_ok: 0,
            bases_failed: 0,
        });
    }

    // Any active fiat asset can be the quote side of a stored rate.
    let quote_assets = sqlx::query!(
        r#"
        SELECT id, code
        FROM assets
        WHERE asset_class = 'fiat' AND is_active = TRUE AND deleted_at IS NULL
        "#,
    )
    .fetch_all(pool)
    .await?;
    let ids: HashMap<String, Uuid> = quote_assets
        .into_iter()
        .map(|row| (row.code, row.id))
        .collect();

    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
    let root = api_root();

    let mut summary = IngestSummary {
        inserted: 0,
        bases_ok: 0,
        bases_failed: 0,
    };
    for (index, base) in bases.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(BASE_FETCH_SPACING).await;
        }

        let rates = match fetch_base(&client, &root, &base.code).await {
            Ok(body) => match parse_payload(&body, &base.code) {
                Ok(rates) => rates,
                Err(err) => {
                    logging::error!("fx rates: base {}: {err}", base.code);
                    summary.bases_failed += 1;
                    continue;
                }
            },
            Err(_) => {
                logging::error!("fx rates: base {} unreachable this run", base.code);
                summary.bases_failed += 1;
                continue;
            }
        };

        match store_base(pool, base.id, &rates, &ids).await {
            Ok(inserted) => {
                summary.inserted += inserted;
                summary.bases_ok += 1;
            }
            Err(err) => {
                logging::error!("fx rates: base {}: {err}", base.code);
                summary.bases_failed += 1;
            }
        }
    }

    Ok(summary)
}

/// `GET {root}/v2/rates?base={code}`, retrying a timeout or 5xx a few times with
/// backoff before giving up on this base for the run.
async fn fetch_base(
    client: &reqwest::Client,
    root: &str,
    code: &str,
) -> Result<String, RatesError> {
    let url = format!("{root}/v2/rates?base={code}");
    let mut attempt = 0;
    loop {
        match client
            .get(url.as_str())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => return Ok(response.text().await?),
            Err(err) => {
                let retryable =
                    err.is_timeout() || err.status().is_some_and(|status| status.is_server_error());
                if retryable && attempt < FETCH_RETRY_BACKOFF.len() {
                    logging::error!("fx rates: base {code}: {err}; retrying");
                    tokio::time::sleep(FETCH_RETRY_BACKOFF[attempt]).await;
                    attempt += 1;
                    continue;
                }
                return Err(err.into());
            }
        }
    }
}

/// Store one base's parsed rates in a single transaction; returns rows inserted.
async fn store_base(
    pool: &PgPool,
    base_id: Uuid,
    rates: &[ReferenceRate],
    ids: &HashMap<String, Uuid>,
) -> Result<u64, RatesError> {
    let mut tx = pool.begin().await?;

    let mut inserted = 0_u64;
    for ReferenceRate {
        observed_at,
        quote_code,
        rate,
    } in rates
    {
        let Some(&quote_id) = ids.get(quote_code) else {
            continue;
        };
        if quote_id == base_id {
            continue;
        }

        let result = sqlx::query!(
            r#"
            INSERT INTO asset_rates
                (base_asset_id, quote_asset_id, rate, rate_source, observed_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (base_asset_id, quote_asset_id, rate_source, observed_at)
            DO NOTHING
            "#,
            base_id,
            quote_id,
            rate,
            RATE_SOURCE,
            observed_at,
        )
        .execute(&mut *tx)
        .await?;

        inserted += result.rows_affected();
    }

    tx.commit().await?;
    Ok(inserted)
}

/// Parse a Frankfurter v2 `/v2/rates` response body into validated rates. Every
/// row must carry `expected_base` as its base, a valid date, and a positive
/// decimal rate (mirroring the `asset_rates_rate_positive` check); the rates
/// arrive as one block, so a single bad value fails this base's run rather than
/// being silently dropped. A row quoting the base against itself is skipped (it
/// would violate `asset_rates_distinct_assets`).
fn parse_payload(body: &str, expected_base: &str) -> Result<Vec<ReferenceRate>, RatesError> {
    let rows: Vec<RateRow> = serde_json::from_str(body)
        .map_err(|_| RatesError::Parse("could not parse the exchange-rate response"))?;

    if rows.is_empty() {
        return Err(RatesError::Parse(
            "exchange-rate response contained no rates",
        ));
    }

    let mut rates = Vec::with_capacity(rows.len());
    for row in rows {
        if row.base != expected_base {
            return Err(RatesError::Parse(
                "exchange-rate response had an unexpected base currency",
            ));
        }
        if row.quote == row.base {
            continue;
        }

        let observed_at = parse_reference_date(&row.date)?;

        let rate = BigDecimal::from_str(row.rate.get().trim())
            .map_err(|_| RatesError::Parse("exchange-rate response had a non-numeric rate"))?;
        if !rate.is_positive() {
            return Err(RatesError::Parse(
                "exchange-rate response had a non-positive rate",
            ));
        }

        rates.push(ReferenceRate {
            observed_at,
            quote_code: row.quote,
            rate,
        });
    }

    Ok(rates)
}

/// Interpret a `YYYY-MM-DD` reference date as an instant at `00:00:00Z`.
fn parse_reference_date(date: &str) -> Result<DateTime<Utc>, RatesError> {
    let naive = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .ok_or(RatesError::Parse(
            "exchange-rate response had an invalid reference date",
        ))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

// --- Rate resolution ---------------------------------------------------------

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
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
        { "date": "2024-06-14", "base": "EUR", "quote": "USD", "rate": 1.0715 },
        { "date": "2024-06-14", "base": "EUR", "quote": "GBP", "rate": 0.84395 },
        { "date": "2024-06-13", "base": "EUR", "quote": "JPY", "rate": 168.79 }
    ]"#;

    fn find<'a>(rates: &'a [ReferenceRate], code: &str) -> &'a ReferenceRate {
        rates
            .iter()
            .find(|rate| rate.quote_code == code)
            .expect("code present in parsed rates")
    }

    fn midnight_utc(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    }

    #[test]
    fn parses_each_row_reference_date_as_midnight_utc() {
        let rates = parse_payload(SAMPLE, "EUR").expect("sample parses");
        assert_eq!(find(&rates, "USD").observed_at, midnight_utc(2024, 6, 14));
        assert_eq!(find(&rates, "JPY").observed_at, midnight_utc(2024, 6, 13));
    }

    #[test]
    fn parses_rates_without_a_float_round_trip() {
        let rates = parse_payload(SAMPLE, "EUR").expect("sample parses");
        assert_eq!(rates.len(), 3);
        assert_eq!(
            find(&rates, "USD").rate,
            BigDecimal::from_str("1.0715").unwrap()
        );
        assert_eq!(
            find(&rates, "GBP").rate,
            BigDecimal::from_str("0.84395").unwrap()
        );
    }

    #[test]
    fn preserves_a_high_precision_rate() {
        let body = r#"[{"date":"2024-06-14","base":"EUR","quote":"USD",
            "rate":1.234567890123456789}]"#;
        let rates = parse_payload(body, "EUR").expect("body parses");
        assert_eq!(
            find(&rates, "USD").rate,
            BigDecimal::from_str("1.234567890123456789").unwrap()
        );
    }

    #[test]
    fn parses_an_integer_rate() {
        let body = r#"[{"date":"2024-06-14","base":"EUR","quote":"AOA","rate":818976}]"#;
        let rates = parse_payload(body, "EUR").expect("body parses");
        assert_eq!(
            find(&rates, "AOA").rate,
            BigDecimal::from_str("818976").unwrap()
        );
    }

    #[test]
    fn accepts_any_base_as_long_as_it_is_the_one_requested() {
        let body = r#"[
            {"date":"2024-06-14","base":"USD","quote":"EUR","rate":0.93},
            {"date":"2024-06-14","base":"USD","quote":"USD","rate":1.0}
        ]"#;
        let rates = parse_payload(body, "USD").expect("body parses");
        // The USD self-quote is dropped; the EUR quote is kept.
        assert_eq!(rates.len(), 1);
        assert_eq!(
            find(&rates, "EUR").rate,
            BigDecimal::from_str("0.93").unwrap()
        );
    }

    #[test]
    fn skips_a_base_self_quote() {
        let body = r#"[{"date":"2024-06-14","base":"EUR","quote":"EUR","rate":1.0}]"#;
        assert!(parse_payload(body, "EUR").expect("body parses").is_empty());
    }

    #[test]
    fn rejects_a_non_positive_rate() {
        for rate in ["0", "-1.5"] {
            let body =
                format!(r#"[{{"date":"2024-06-14","base":"EUR","quote":"USD","rate":{rate}}}]"#);
            assert!(matches!(
                parse_payload(&body, "EUR"),
                Err(RatesError::Parse(_))
            ));
        }
    }

    #[test]
    fn rejects_a_base_other_than_the_one_requested() {
        let body = r#"[{"date":"2024-06-14","base":"USD","quote":"EUR","rate":0.93}]"#;
        assert!(matches!(
            parse_payload(body, "EUR"),
            Err(RatesError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_invalid_reference_date() {
        let body = r#"[{"date":"not-a-date","base":"EUR","quote":"USD","rate":1.07}]"#;
        assert!(matches!(
            parse_payload(body, "EUR"),
            Err(RatesError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_empty_response() {
        assert!(matches!(
            parse_payload("[]", "EUR"),
            Err(RatesError::Parse(_))
        ));
    }
}

#[cfg(test)]
mod as_of_tests {
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
