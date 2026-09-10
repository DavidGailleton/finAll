//! Process-level in-memory cache of Frankfurter rate tables, keyed by
//! `(base currency, requested date)`. Exchange rates are never persisted; this
//! cache is the only thing that keeps one report from firing a Frankfurter
//! request per currency, and a second report the same day from firing any.
//!
//! [`FxRateCache`] holds one `reqwest::Client` for the process and, per key, the
//! full `base -> {quote -> rate}` table Frankfurter returned. A miss fetches
//! `base=<code>` — the latest endpoint, or the single-date endpoint for a past
//! `as_of` — with the HTTP call made outside the lock. Past-date entries are
//! immutable; the "today" entry is simply superseded by tomorrow's key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bigdecimal::BigDecimal;
use leptos::logging;
use sqlx::types::chrono::{DateTime, NaiveDate, Utc};

use crate::server::assets::frankfurter::{self, FrankfurterError};

/// One base currency's quotes for one requested date, as Frankfurter served
/// them. `as_of` is the reference date actually returned (it can precede the
/// requested day on weekends and holidays) and becomes the valuation timestamp.
#[derive(Clone)]
struct CachedTable {
    as_of: DateTime<Utc>,
    quotes: Arc<HashMap<String, BigDecimal>>,
}

impl CachedTable {
    /// Build from a parsed response. Every row shares one reference date, so the
    /// first row's `observed_at` is the table's `as_of`; an empty response
    /// yields no table.
    fn from_rows(rows: Vec<frankfurter::ReferenceRate>) -> Option<Self> {
        let as_of = rows.first()?.observed_at;
        let quotes = rows
            .into_iter()
            .map(|row| (row.quote_code, row.rate))
            .collect();
        Some(Self {
            as_of,
            quotes: Arc::new(quotes),
        })
    }

    fn lookup(&self, quote_code: &str) -> Option<(BigDecimal, DateTime<Utc>)> {
        self.quotes
            .get(quote_code)
            .map(|rate| (rate.clone(), self.as_of))
    }
}

/// The shared Frankfurter rate cache. Construct once at startup and hand to
/// server functions via `provide_context`.
pub struct FxRateCache {
    #[cfg(not(test))]
    client: reqwest::Client,
    tables: Mutex<HashMap<(String, NaiveDate), CachedTable>>,
}

impl FxRateCache {
    /// Build the cache and its one HTTP client.
    pub fn new() -> Result<Arc<Self>, FrankfurterError> {
        Ok(Arc::new(Self {
            #[cfg(not(test))]
            client: frankfurter::client()?,
            tables: Mutex::new(HashMap::new()),
        }))
    }

    /// The `base_code -> quote_code` rate for `date`, with the reference instant
    /// it is anchored to. On a cache miss, fetch the whole `base=<base_code>`
    /// table from Frankfurter — the latest endpoint when `latest`, else the
    /// single-date endpoint — cache it, and return the one quote. `Ok(None)`
    /// when Frankfurter carries no such quote; `Err` on a fetch or parse failure
    /// (nothing is cached in either case, so the next call retries).
    pub async fn direct_rate(
        &self,
        base_code: &str,
        quote_code: &str,
        date: NaiveDate,
        latest: bool,
    ) -> Result<Option<(BigDecimal, DateTime<Utc>)>, FrankfurterError> {
        let key = (base_code.to_owned(), date);

        if let Some(table) = self.get(&key) {
            return Ok(table.lookup(quote_code));
        }

        let Some(table) = self.fetch(base_code, date, latest).await? else {
            return Ok(None);
        };
        self.insert(key, table.clone());
        Ok(table.lookup(quote_code))
    }

    /// Fetch one base's table from Frankfurter on a cache miss. Under `cfg(test)`
    /// this is a no-op returning `None`, so the suite never reaches the network —
    /// tests seed [`FxRateCache`] directly.
    #[cfg(not(test))]
    async fn fetch(
        &self,
        base_code: &str,
        date: NaiveDate,
        latest: bool,
    ) -> Result<Option<CachedTable>, FrankfurterError> {
        let rows = if latest {
            frankfurter::fetch_rates(&self.client, base_code).await?
        } else {
            frankfurter::fetch_rates_on(&self.client, base_code, date).await?
        };
        Ok(CachedTable::from_rows(rows))
    }

    #[cfg(test)]
    async fn fetch(
        &self,
        _base_code: &str,
        _date: NaiveDate,
        _latest: bool,
    ) -> Result<Option<CachedTable>, FrankfurterError> {
        Ok(None)
    }

    fn get(&self, key: &(String, NaiveDate)) -> Option<CachedTable> {
        match self.tables.lock() {
            Ok(guard) => guard.get(key).cloned(),
            Err(_) => {
                logging::error!("fx cache: lock poisoned; treating as a miss");
                None
            }
        }
    }

    fn insert(&self, key: (String, NaiveDate), table: CachedTable) {
        match self.tables.lock() {
            Ok(mut guard) => {
                guard.insert(key, table);
            }
            Err(_) => logging::error!("fx cache: lock poisoned; skipping insert"),
        }
    }

    /// Pre-populate a table so a test never reaches the network.
    #[cfg(test)]
    pub fn seed(
        &self,
        base_code: &str,
        date: NaiveDate,
        as_of: DateTime<Utc>,
        quotes: &[(&str, &str)],
    ) {
        use std::str::FromStr;

        let quotes = quotes
            .iter()
            .map(|(code, rate)| {
                (
                    (*code).to_owned(),
                    BigDecimal::from_str(rate).expect("valid decimal literal"),
                )
            })
            .collect();
        self.insert(
            (base_code.to_owned(), date),
            CachedTable {
                as_of,
                quotes: Arc::new(quotes),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    }

    fn row(quote: &str, value: &str, y: i32, m: u32, d: u32) -> frankfurter::ReferenceRate {
        frankfurter::ReferenceRate {
            observed_at: at(y, m, d),
            quote_code: quote.to_owned(),
            rate: BigDecimal::from_str(value).unwrap(),
        }
    }

    #[test]
    fn table_takes_its_as_of_from_the_first_row_and_maps_every_quote() {
        let table = CachedTable::from_rows(vec![
            row("USD", "1.07", 2026, 6, 12),
            row("GBP", "0.84", 2026, 6, 12),
        ])
        .expect("non-empty response");

        assert_eq!(table.as_of, at(2026, 6, 12));
        assert_eq!(
            table.lookup("USD"),
            Some((BigDecimal::from_str("1.07").unwrap(), at(2026, 6, 12)))
        );
        assert_eq!(
            table.lookup("GBP").map(|(rate, _)| rate),
            Some(BigDecimal::from_str("0.84").unwrap())
        );
    }

    #[test]
    fn a_quote_the_table_does_not_carry_is_a_miss() {
        let table =
            CachedTable::from_rows(vec![row("USD", "1.07", 2026, 6, 12)]).expect("non-empty");
        assert_eq!(table.lookup("JPY"), None);
    }

    #[test]
    fn an_empty_response_builds_no_table() {
        assert!(CachedTable::from_rows(vec![]).is_none());
    }

    #[test]
    fn a_seeded_table_is_found_only_under_its_own_key() {
        let cache = FxRateCache::new().expect("client builds");
        let queried = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        cache.seed("USD", queried, at(2026, 6, 30), &[("EUR", "0.8")]);

        let table = cache
            .get(&("USD".to_owned(), queried))
            .expect("seeded entry present");
        assert_eq!(
            table.lookup("EUR"),
            Some((BigDecimal::from_str("0.8").unwrap(), at(2026, 6, 30)))
        );

        assert!(cache
            .get(&(
                "USD".to_owned(),
                NaiveDate::from_ymd_opt(2026, 6, 29).unwrap()
            ))
            .is_none());
        assert!(cache.get(&("GBP".to_owned(), queried)).is_none());
    }
}
