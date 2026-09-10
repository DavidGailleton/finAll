//! The Frankfurter v2 API client (`api.frankfurter.dev`; override the host with
//! `FRANKFURTER_URL`). Fetch and parse only — this module never touches the
//! database. Persisting what it returns is [`crate::server::assets::fx_sync`]'s
//! job; combining stored rates into a conversion is
//! [`crate::server::assets::rates`]'s.
//!
//! Two endpoints are used:
//!
//! - `GET /v2/rates?base=<code>` → a JSON array of `{date, base, quote, rate}`.
//!   [`fetch_rates`] retries a timeout or 5xx a few times, then parses each row
//!   into a [`ReferenceRate`] (an exact positive `base -> quote` rate at its
//!   reference date, parsed straight from the response text into a
//!   [`BigDecimal`], never through `f64`).
//! - `GET /v2/currencies` → a JSON array of `{iso_code, iso_numeric, name,
//!   symbol, …}`. [`fetch_currencies`] returns the rows as-is; reducing them to
//!   what our schema accepts is `fx_sync`'s concern (it applies the domain
//!   validators).

use std::str::FromStr;
use std::time::Duration;

use bigdecimal::{BigDecimal, Signed};
use leptos::logging;
use sqlx::types::chrono::{DateTime, NaiveDate, Utc};

/// `rate_source` tag stored on every `asset_rates` row sourced from Frankfurter.
pub const RATE_SOURCE: &str = "frankfurter.dev";

/// The Frankfurter API host used unless `FRANKFURTER_URL` overrides it.
const DEFAULT_API_ROOT: &str = "https://api.frankfurter.dev";

/// Upper bound on a single fetch; a slow or hanging source must not wedge the task.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Backoff before each retry of one `/v2/rates` fetch (a timeout or 5xx); its
/// length is the retry count, so a base gets `1 + len` attempts before it fails.
const FETCH_RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

/// The Frankfurter API root (no trailing slash): `FRANKFURTER_URL` when set and
/// non-blank — point it at a self-hosted instance or mirror serving the same v2
/// `/v2/rates` and `/v2/currencies` shapes — otherwise [`DEFAULT_API_ROOT`].
pub fn api_root() -> String {
    let root = std::env::var("FRANKFURTER_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_ROOT.to_owned());
    root.trim_end_matches('/').to_owned()
}

/// Why a Frankfurter request could not be completed. The messages are safe to
/// surface; today the only callers are the background tasks, which log and retry
/// on the next tick.
#[derive(Debug, thiserror::Error)]
pub enum FrankfurterError {
    #[error("could not reach the exchange-rate source")]
    Fetch,

    #[error("{0}")]
    Parse(&'static str),
}

impl From<reqwest::Error> for FrankfurterError {
    fn from(err: reqwest::Error) -> Self {
        logging::error!("frankfurter: request error: {err}");
        FrankfurterError::Fetch
    }
}

/// A shared `reqwest` client with [`HTTP_TIMEOUT`], reused across the requests of
/// one sync run.
pub fn client() -> Result<reqwest::Client, FrankfurterError> {
    Ok(reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?)
}

/// One row of the `/v2/rates` response, which is a JSON array of these. `rate` is
/// kept as its raw JSON text so it can be parsed into an exact decimal rather
/// than through `f64`.
#[derive(serde::Deserialize)]
struct RateRow {
    date: String,
    base: String,
    quote: String,
    rate: Box<serde_json::value::RawValue>,
}

/// A validated rate row: its reference date (at `00:00:00Z`), the quote currency
/// code, and an exact positive `base -> quote` rate. The base is tracked by the
/// caller — [`fetch_rates`] takes one base at a time.
pub struct ReferenceRate {
    pub observed_at: DateTime<Utc>,
    pub quote_code: String,
    pub rate: BigDecimal,
}

/// One entry of the `/v2/currencies` response. `start_date` / `end_date` are
/// present in the payload but not deserialized.
#[derive(serde::Deserialize)]
pub struct CurrencyRow {
    pub iso_code: String,
    pub iso_numeric: Option<String>,
    pub name: String,
    pub symbol: Option<String>,
}

/// Fetch and validate the latest rates for one base currency:
/// `GET {api_root()}/v2/rates?base={base_code}`, retrying a timeout or 5xx a few
/// times with backoff before giving up.
pub async fn fetch_rates(
    client: &reqwest::Client,
    base_code: &str,
) -> Result<Vec<ReferenceRate>, FrankfurterError> {
    let body = fetch_rates_body(client, &api_root(), base_code).await?;
    parse_payload(&body, base_code)
}

/// Fetch the currency list: `GET {api_root()}/v2/currencies`. Returns the rows
/// unmodified.
pub async fn fetch_currencies(
    client: &reqwest::Client,
) -> Result<Vec<CurrencyRow>, FrankfurterError> {
    let url = format!("{}/v2/currencies", api_root());
    let body = client
        .get(url.as_str())
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)?
        .text()
        .await?;

    serde_json::from_str(&body)
        .map_err(|_| FrankfurterError::Parse("could not parse the currency list response"))
}

/// `GET {root}/v2/rates?base={code}` as text, retrying a timeout or 5xx.
async fn fetch_rates_body(
    client: &reqwest::Client,
    root: &str,
    code: &str,
) -> Result<String, FrankfurterError> {
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
                    logging::error!("frankfurter: base {code}: {err}; retrying");
                    tokio::time::sleep(FETCH_RETRY_BACKOFF[attempt]).await;
                    attempt += 1;
                    continue;
                }
                return Err(err.into());
            }
        }
    }
}

/// Parse a `/v2/rates` response body into validated rates. Every row must carry
/// `expected_base` as its base, a valid date, and a positive decimal rate
/// (mirroring the `asset_rates_rate_positive` check); the rates arrive as one
/// block, so a single bad value fails this base's run rather than being silently
/// dropped. A row quoting the base against itself is skipped (it would violate
/// `asset_rates_distinct_assets`).
fn parse_payload(body: &str, expected_base: &str) -> Result<Vec<ReferenceRate>, FrankfurterError> {
    let rows: Vec<RateRow> = serde_json::from_str(body)
        .map_err(|_| FrankfurterError::Parse("could not parse the exchange-rate response"))?;

    if rows.is_empty() {
        return Err(FrankfurterError::Parse(
            "exchange-rate response contained no rates",
        ));
    }

    let mut rates = Vec::with_capacity(rows.len());
    for row in rows {
        if row.base != expected_base {
            return Err(FrankfurterError::Parse(
                "exchange-rate response had an unexpected base currency",
            ));
        }
        if row.quote == row.base {
            continue;
        }

        let observed_at = parse_reference_date(&row.date)?;

        let rate = BigDecimal::from_str(row.rate.get().trim()).map_err(|_| {
            FrankfurterError::Parse("exchange-rate response had a non-numeric rate")
        })?;
        if !rate.is_positive() {
            return Err(FrankfurterError::Parse(
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
fn parse_reference_date(date: &str) -> Result<DateTime<Utc>, FrankfurterError> {
    let naive = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .ok_or(FrankfurterError::Parse(
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
                Err(FrankfurterError::Parse(_))
            ));
        }
    }

    #[test]
    fn rejects_a_base_other_than_the_one_requested() {
        let body = r#"[{"date":"2024-06-14","base":"USD","quote":"EUR","rate":0.93}]"#;
        assert!(matches!(
            parse_payload(body, "EUR"),
            Err(FrankfurterError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_invalid_reference_date() {
        let body = r#"[{"date":"not-a-date","base":"EUR","quote":"USD","rate":1.07}]"#;
        assert!(matches!(
            parse_payload(body, "EUR"),
            Err(FrankfurterError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_empty_response() {
        assert!(matches!(
            parse_payload("[]", "EUR"),
            Err(FrankfurterError::Parse(_))
        ));
    }
}
