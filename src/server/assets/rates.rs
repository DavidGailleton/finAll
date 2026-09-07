//! Opt-in ingestion of euro reference rates from `frankfurter.dev` (v2 API).
//!
//! [`fetch_and_store`] is the single entry point, called on an interval by the
//! background task in `main.rs` only when `FETCH_FX_RATES` is set. It fetches the
//! latest rates with `EUR` as the base and writes one `asset_rates` row per
//! currency it can resolve to an active fiat asset: `base_asset_id` is the `EUR`
//! asset, `quote_asset_id` is the other currency, `rate` is the exact value
//! Frankfurter returned (parsed straight from the response text into a
//! [`BigDecimal`], never through `f64`). The v2 API covers the European Central
//! Bank reference currencies plus a wider set of others.
//!
//! `observed_at` is a rate's reference *date* at `00:00:00Z` — a date, not a
//! wall-clock publication instant. The v2 API returns each currency's latest
//! available date independently, so a single run can produce several distinct
//! `observed_at` values. Re-running for a reference date already stored is a
//! no-op: the natural key `(base_asset_id, quote_asset_id, rate_source,
//! observed_at)` already holds the row and the insert is `ON CONFLICT DO
//! NOTHING`, so a stored observation is never mutated.
//!
//! Cross-rates and reciprocals (anything not `EUR -> X`) are intentionally not
//! derived or stored here; a conversion needs an explicit rounding policy that
//! the caller supplies (see [`crate::server::assets::conversion`]).

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use bigdecimal::{BigDecimal, Signed};
use leptos::logging;
use sqlx::types::chrono::{DateTime, NaiveDate, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;

/// `rate_source` tag stored on every row this module writes.
const RATE_SOURCE: &str = "frankfurter.dev";
/// Latest reference rates, quoted against the euro (Frankfurter v2 API).
const LATEST_URL: &str = "https://api.frankfurter.dev/v2/rates?base=EUR";
/// The base currency this module ingests against.
const BASE_CODE: &str = "EUR";
/// Upper bound on a single fetch; a slow or hanging source must not wedge the task.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

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

/// A validated rate: the reference date (at `00:00:00Z`), the quote currency
/// code, and an exact positive `EUR -> quote_code` rate.
struct ReferenceRate {
    observed_at: DateTime<Utc>,
    quote_code: String,
    rate: BigDecimal,
}

/// Fetch the latest EUR-based reference rates and store the ones that resolve to
/// an active fiat asset. Returns the number of rows actually inserted (rows that
/// already existed for their reference date are not counted).
pub async fn fetch_and_store(pool: &PgPool) -> Result<u64, RatesError> {
    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
    let body = client
        .get(LATEST_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let rates = parse_payload(&body)?;

    let mut wanted: Vec<String> = Vec::with_capacity(rates.len() + 1);
    wanted.push(BASE_CODE.to_owned());
    wanted.extend(rates.iter().map(|rate| rate.quote_code.clone()));

    let assets = sqlx::query!(
        r#"
        SELECT id, code
        FROM assets
        WHERE asset_class = 'fiat'
          AND is_active = TRUE
          AND deleted_at IS NULL
          AND code = ANY($1)
        "#,
        &wanted,
    )
    .fetch_all(pool)
    .await?;

    let ids: HashMap<String, Uuid> = assets.into_iter().map(|row| (row.code, row.id)).collect();

    let Some(&base_id) = ids.get(BASE_CODE) else {
        logging::log!("fx rates: no active {BASE_CODE} asset; skipping run");
        return Ok(0);
    };

    let mut inserted = 0_u64;
    let mut skipped = 0_usize;
    for ReferenceRate {
        observed_at,
        quote_code,
        rate,
    } in &rates
    {
        let Some(&quote_id) = ids.get(quote_code) else {
            skipped += 1;
            continue;
        };

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
        .execute(pool)
        .await?;

        inserted += result.rows_affected();
    }

    if skipped > 0 {
        logging::log!("fx rates: skipped {skipped} currency code(s) with no active fiat asset");
    }

    Ok(inserted)
}

/// Parse a Frankfurter v2 `/v2/rates` response body into validated rates. Every
/// row must be `EUR`-based with a valid date and a positive decimal rate
/// (mirroring the `asset_rates_rate_positive` check); the rates arrive as one
/// block, so a single bad value fails the whole run rather than being silently
/// dropped. A row quoting `EUR` against itself is skipped (it would violate
/// `asset_rates_distinct_assets`).
fn parse_payload(body: &str) -> Result<Vec<ReferenceRate>, RatesError> {
    let rows: Vec<RateRow> = serde_json::from_str(body)
        .map_err(|_| RatesError::Parse("could not parse the exchange-rate response"))?;

    if rows.is_empty() {
        return Err(RatesError::Parse(
            "exchange-rate response contained no rates",
        ));
    }

    let mut rates = Vec::with_capacity(rows.len());
    for row in rows {
        if row.base != BASE_CODE {
            return Err(RatesError::Parse(
                "exchange-rate response had an unexpected base currency",
            ));
        }
        if row.quote == BASE_CODE {
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
        let rates = parse_payload(SAMPLE).expect("sample parses");
        assert_eq!(find(&rates, "USD").observed_at, midnight_utc(2024, 6, 14));
        assert_eq!(find(&rates, "JPY").observed_at, midnight_utc(2024, 6, 13));
    }

    #[test]
    fn parses_rates_without_a_float_round_trip() {
        let rates = parse_payload(SAMPLE).expect("sample parses");
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
        let rates = parse_payload(body).expect("body parses");
        assert_eq!(
            find(&rates, "USD").rate,
            BigDecimal::from_str("1.234567890123456789").unwrap()
        );
    }

    #[test]
    fn parses_an_integer_rate() {
        let body = r#"[{"date":"2024-06-14","base":"EUR","quote":"AOA","rate":818976}]"#;
        let rates = parse_payload(body).expect("body parses");
        assert_eq!(
            find(&rates, "AOA").rate,
            BigDecimal::from_str("818976").unwrap()
        );
    }

    #[test]
    fn skips_a_euro_self_quote() {
        let body = r#"[{"date":"2024-06-14","base":"EUR","quote":"EUR","rate":1.0}]"#;
        assert!(parse_payload(body).expect("body parses").is_empty());
    }

    #[test]
    fn rejects_a_non_positive_rate() {
        for rate in ["0", "-1.5"] {
            let body =
                format!(r#"[{{"date":"2024-06-14","base":"EUR","quote":"USD","rate":{rate}}}]"#);
            assert!(matches!(parse_payload(&body), Err(RatesError::Parse(_))));
        }
    }

    #[test]
    fn rejects_an_unexpected_base_currency() {
        let body = r#"[{"date":"2024-06-14","base":"USD","quote":"EUR","rate":0.93}]"#;
        assert!(matches!(parse_payload(body), Err(RatesError::Parse(_))));
    }

    #[test]
    fn rejects_an_invalid_reference_date() {
        let body = r#"[{"date":"not-a-date","base":"EUR","quote":"USD","rate":1.07}]"#;
        assert!(matches!(parse_payload(body), Err(RatesError::Parse(_))));
    }

    #[test]
    fn rejects_an_empty_response() {
        assert!(matches!(parse_payload("[]"), Err(RatesError::Parse(_))));
    }
}
