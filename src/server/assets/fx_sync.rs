//! Persistence for the opt-in Frankfurter integration: take what
//! [`crate::server::assets::frankfurter`] fetched and reconcile it into the
//! `assets` / `fiat_assets` / `asset_rates` tables.
//!
//! Two entry points, both driven by the calendar-aligned background tasks in
//! `main.rs` when `FETCH_FX_RATES` is set:
//!
//! - [`sync_rates`] — daily. For every currency the user actually holds (plus
//!   [`BASE_CODE`]) it fetches that base's latest rates and writes one
//!   `asset_rates` row per quote currency it can resolve to an active fiat
//!   asset. Each base is fetched and stored on its own: a base whose fetch keeps
//!   failing is logged and skipped without aborting the others, and each base's
//!   rows are one all-or-nothing transaction. Idempotent via `ON CONFLICT DO
//!   NOTHING` on the natural key `(base_asset_id, quote_asset_id, rate_source,
//!   observed_at)`, so a stored observation is never mutated.
//! - [`sync_currencies`] — monthly. Upserts every valid `/v2/currencies` entry
//!   into `assets` / `fiat_assets`: insert currencies not present yet, refresh
//!   the name, symbol, and numeric code of ones that are. `is_active`,
//!   `deleted_at`, and `minor_units` are never touched here — a soft-deleted
//!   currency stays deleted, and a newly inserted one takes the schema default
//!   `minor_units = 2`.
//!
//! Rates are only ever stored as Frankfurter quotes them (`base -> quote`);
//! cross-rates and reciprocals are derived at read time by
//! [`crate::server::assets::rates::resolve_rate`].

use std::collections::HashMap;
use std::time::Duration;

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::assets::{currency, frankfurter, validate};

/// The currency always fetched as a base in addition to the ones in use: it is
/// the read-time cross-rate pivot
/// ([`rates::pivot_asset_id`](crate::server::assets::rates)) and the historical
/// anchor.
const BASE_CODE: &str = "EUR";

/// Delay between successive per-base fetches, so a run of several bases does not
/// hammer the source.
const BASE_FETCH_SPACING: Duration = Duration::from_millis(300);

/// Why a sync run could not complete. The only callers are the background tasks,
/// which log and retry on the next tick.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("could not reach the exchange-rate source")]
    Source,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for SyncError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("fx sync: database error: {err}");
        SyncError::Internal
    }
}

impl From<frankfurter::FrankfurterError> for SyncError {
    fn from(_: frankfurter::FrankfurterError) -> Self {
        SyncError::Source
    }
}

// --- Daily rate sync -------------------------------------------------------

/// What one [`sync_rates`] run did, for the log line.
pub struct IngestSummary {
    /// Rows newly written to `asset_rates` (an observation that already existed
    /// counts zero).
    pub inserted: u64,
    /// Base currencies whose fetch and store both completed.
    pub bases_ok: usize,
    /// Base currencies skipped this run after their fetch kept failing.
    pub bases_failed: usize,
}

/// Fetch the latest rates for every currency the user holds (plus [`BASE_CODE`])
/// as the base, and store the ones that resolve to an active fiat asset.
///
/// Returns [`SyncError`] only for a database failure that stops the run before
/// it starts, or a total inability to build the HTTP client.
pub async fn sync_rates(pool: &PgPool) -> Result<IngestSummary, SyncError> {
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

    let client = frankfurter::client()?;

    let mut summary = IngestSummary {
        inserted: 0,
        bases_ok: 0,
        bases_failed: 0,
    };
    for (index, base) in bases.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(BASE_FETCH_SPACING).await;
        }

        let rates = match frankfurter::fetch_rates(&client, &base.code).await {
            Ok(rates) => rates,
            Err(err) => {
                logging::error!("fx rates: base {}: {err}", base.code);
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

/// Store one base's parsed rates in a single transaction; returns rows inserted.
async fn store_base(
    pool: &PgPool,
    base_id: Uuid,
    rates: &[frankfurter::ReferenceRate],
    ids: &HashMap<String, Uuid>,
) -> Result<u64, SyncError> {
    let mut tx = pool.begin().await?;

    let mut inserted = 0_u64;
    for frankfurter::ReferenceRate {
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
            frankfurter::RATE_SOURCE,
            observed_at,
        )
        .execute(&mut *tx)
        .await?;

        inserted += result.rows_affected();
    }

    tx.commit().await?;
    Ok(inserted)
}

// --- Monthly currency sync -----------------------------------------------

/// A `/v2/currencies` entry that passed validation, ready to upsert.
struct NormalisedCurrency {
    code: String,
    numeric_code: Option<String>,
    name: String,
    symbol: Option<String>,
}

/// Reduce a Frankfurter currency row to what our schema accepts, or `None` if
/// its code or name cannot be used. A malformed numeric code or a blank symbol
/// is cleared rather than rejecting the whole currency.
fn normalise(row: frankfurter::CurrencyRow) -> Option<NormalisedCurrency> {
    let code = currency::validate_alphabetic_code(&row.iso_code).ok()?;
    let name = validate::name(&row.name).ok()?;
    let symbol = validate::symbol(row.symbol.as_deref());
    let numeric_code = currency::validate_numeric_code(row.iso_numeric.as_deref()).unwrap_or(None);
    Some(NormalisedCurrency {
        code,
        numeric_code,
        name,
        symbol,
    })
}

/// What one [`sync_currencies`] run did, for the log line.
pub struct SyncSummary {
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Fetch `/v2/currencies` from Frankfurter and upsert every valid entry into
/// `assets` / `fiat_assets`. A fetch or parse failure fails the whole run; a
/// per-currency database failure is logged and counted as skipped.
pub async fn sync_currencies(pool: &PgPool) -> Result<SyncSummary, SyncError> {
    let client = frankfurter::client()?;
    let rows = frankfurter::fetch_currencies(&client).await?;
    let currencies: Vec<NormalisedCurrency> = rows.into_iter().filter_map(normalise).collect();

    apply(pool, &currencies).await
}

/// Upsert the given currencies, one transaction each. Split out from
/// [`sync_currencies`] so a test can drive the DB effect without HTTP.
async fn apply(pool: &PgPool, currencies: &[NormalisedCurrency]) -> Result<SyncSummary, SyncError> {
    let mut summary = SyncSummary {
        inserted: 0,
        updated: 0,
        skipped: 0,
    };

    for currency in currencies {
        match upsert_one(pool, currency).await {
            Ok(Some(true)) => summary.inserted += 1,
            Ok(Some(false)) => summary.updated += 1,
            Ok(None) => summary.skipped += 1,
            Err(err) => {
                logging::error!("currency sync: {}: {err}", currency.code);
                summary.skipped += 1;
            }
        }
    }

    Ok(summary)
}

/// Upsert one currency in its own transaction. `Ok(Some(true))` if the `assets`
/// row was inserted, `Ok(Some(false))` if it was refreshed, `Ok(None)` if it was
/// skipped because a soft-deleted row already holds that code.
async fn upsert_one(
    pool: &PgPool,
    currency: &NormalisedCurrency,
) -> Result<Option<bool>, SyncError> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query!(
        r#"
        SELECT id, deleted_at
        FROM assets
        WHERE asset_class = 'fiat' AND code = $1
        "#,
        currency.code,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let (asset_id, inserted) = match existing {
        Some(row) if row.deleted_at.is_some() => return Ok(None),
        Some(row) => {
            sqlx::query!(
                r#"
                UPDATE assets
                SET asset_name = $2, symbol = $3, updated_at = now()
                WHERE id = $1
                "#,
                row.id,
                currency.name,
                currency.symbol,
            )
            .execute(&mut *tx)
            .await?;
            (row.id, false)
        }
        None => {
            let id = sqlx::query_scalar!(
                r#"
                INSERT INTO assets (asset_class, code, asset_name, symbol)
                VALUES ('fiat', $1, $2, $3)
                RETURNING id
                "#,
                currency.code,
                currency.name,
                currency.symbol,
            )
            .fetch_one(&mut *tx)
            .await?;
            (id, true)
        }
    };

    sqlx::query!(
        r#"
        INSERT INTO fiat_assets (asset_id, numeric_code)
        VALUES ($1, $2)
        ON CONFLICT (asset_id) DO UPDATE
        SET numeric_code = EXCLUDED.numeric_code
        "#,
        asset_id,
        currency.numeric_code,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(inserted))
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    fn row(
        code: &str,
        numeric: Option<&str>,
        name: &str,
        symbol: Option<&str>,
    ) -> frankfurter::CurrencyRow {
        frankfurter::CurrencyRow {
            iso_code: code.to_owned(),
            iso_numeric: numeric.map(str::to_owned),
            name: name.to_owned(),
            symbol: symbol.map(str::to_owned),
        }
    }

    #[test]
    fn normalise_keeps_a_valid_row_and_trims_it() {
        let normalised =
            normalise(row("usd", Some("840"), "  US Dollar  ", Some(" $ "))).expect("valid row");
        assert_eq!(normalised.code, "USD");
        assert_eq!(normalised.numeric_code.as_deref(), Some("840"));
        assert_eq!(normalised.name, "US Dollar");
        assert_eq!(normalised.symbol.as_deref(), Some("$"));
    }

    #[test]
    fn normalise_drops_a_bad_code_or_blank_name() {
        assert!(normalise(row("US", None, "Dollar", None)).is_none());
        assert!(normalise(row("USD", None, "   ", None)).is_none());
    }

    #[test]
    fn normalise_clears_a_blank_symbol_and_a_malformed_numeric_code() {
        let normalised =
            normalise(row("USD", Some("84"), "US Dollar", Some("   "))).expect("still valid");
        assert_eq!(normalised.numeric_code, None);
        assert_eq!(normalised.symbol, None);
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;

    fn normalised(
        code: &str,
        numeric: Option<&str>,
        name: &str,
        symbol: Option<&str>,
    ) -> NormalisedCurrency {
        NormalisedCurrency {
            code: code.to_owned(),
            numeric_code: numeric.map(str::to_owned),
            name: name.to_owned(),
            symbol: symbol.map(str::to_owned),
        }
    }

    /// `(asset_name, numeric_code, minor_units, is_active, is_deleted)`. Uses the
    /// runtime `sqlx::query` form, not the macro: `cargo sqlx prepare` does not
    /// cache test-only queries.
    async fn fiat(pool: &PgPool, code: &str) -> (String, Option<String>, i16, bool, bool) {
        use sqlx::Row;

        let row = sqlx::query(
            r#"
            SELECT
                a.asset_name,
                f.numeric_code,
                f.minor_units,
                a.is_active,
                (a.deleted_at IS NOT NULL) AS deleted
            FROM assets AS a
            INNER JOIN fiat_assets AS f ON f.asset_id = a.id
            WHERE a.asset_class = 'fiat' AND a.code = $1
            "#,
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("currency row exists");

        (
            row.get("asset_name"),
            row.get("numeric_code"),
            row.get("minor_units"),
            row.get("is_active"),
            row.get("deleted"),
        )
    }

    #[sqlx::test]
    async fn sync_inserts_new_and_refreshes_existing_without_touching_lifecycle(pool: PgPool) {
        let summary = apply(
            &pool,
            &[
                normalised("EUR", Some("978"), "Euro (renamed)", Some("€")),
                normalised("ZZZ", Some("999"), "Test Currency", None),
            ],
        )
        .await
        .expect("sync runs");

        assert_eq!(summary.inserted, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped, 0);

        let (name, numeric, minor_units, active, deleted) = fiat(&pool, "EUR").await;
        assert_eq!(name, "Euro (renamed)");
        assert_eq!(numeric.as_deref(), Some("978"));
        assert_eq!(minor_units, 2);
        assert!(active && !deleted);

        let (name, numeric, minor_units, active, deleted) = fiat(&pool, "ZZZ").await;
        assert_eq!(name, "Test Currency");
        assert_eq!(numeric.as_deref(), Some("999"));
        assert_eq!(minor_units, 2);
        assert!(active && !deleted);
    }

    #[sqlx::test]
    async fn sync_does_not_resurrect_a_soft_deleted_currency(pool: PgPool) {
        sqlx::query(
            "UPDATE assets SET deleted_at = now() WHERE asset_class = 'fiat' AND code = 'USD'",
        )
        .execute(&pool)
        .await
        .expect("soft delete USD");

        let summary = apply(
            &pool,
            &[normalised("USD", Some("840"), "US Dollar CHANGED", None)],
        )
        .await
        .expect("sync runs");

        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.inserted, 0);
        assert_eq!(summary.updated, 0);

        let (name, _, _, _, deleted) = fiat(&pool, "USD").await;
        assert!(deleted);
        assert_ne!(name, "US Dollar CHANGED");
    }
}
