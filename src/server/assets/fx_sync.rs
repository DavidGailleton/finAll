//! Sync the fiat-currency list from Frankfurter into `assets` / `fiat_assets`.
//!
//! [`sync_currencies`] fetches `/v2/currencies`, normalises each entry with the
//! shared validators, and upserts it: insert currencies not present yet, and —
//! only when the name, symbol, or numeric code actually changed — refresh an
//! existing one. `is_active`, `deleted_at`, and `minor_units` are never touched
//! here: a soft-deleted currency stays deleted, and a newly inserted one takes
//! the schema default `minor_units = 2`. Run once at startup and then monthly by
//! the background task in `main.rs`.
//!
//! Exchange rates are not persisted — they are fetched on demand and memoised in
//! [`crate::server::assets::fx_cache`].

use leptos::logging;
use sqlx::PgPool;

use crate::server::assets::{currency, frankfurter, validate};

/// Why a currency sync could not complete. The only caller is the background
/// task, which logs and retries on the next tick.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("could not reach the exchange-rate source")]
    Source,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for SyncError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("currency sync: database error: {err}");
        SyncError::Internal
    }
}

impl From<frankfurter::FrankfurterError> for SyncError {
    fn from(_: frankfurter::FrankfurterError) -> Self {
        SyncError::Source
    }
}

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

/// What upserting one currency did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpsertOutcome {
    Inserted,
    Updated,
    Unchanged,
    SkippedDeleted,
}

/// What one [`sync_currencies`] run did, for the log line.
pub struct SyncSummary {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
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
        unchanged: 0,
        skipped: 0,
    };

    for currency in currencies {
        match upsert_one(pool, currency).await {
            Ok(UpsertOutcome::Inserted) => summary.inserted += 1,
            Ok(UpsertOutcome::Updated) => summary.updated += 1,
            Ok(UpsertOutcome::Unchanged) => summary.unchanged += 1,
            Ok(UpsertOutcome::SkippedDeleted) => summary.skipped += 1,
            Err(err) => {
                logging::error!("currency sync: {}: {err}", currency.code);
                summary.skipped += 1;
            }
        }
    }

    Ok(summary)
}

/// Upsert one currency in its own transaction. A soft-deleted row holding the
/// code is left untouched ([`UpsertOutcome::SkippedDeleted`]); an existing row
/// whose name, symbol, and numeric code already all match is left untouched too
/// ([`UpsertOutcome::Unchanged`] — no `updated_at` bump).
async fn upsert_one(
    pool: &PgPool,
    currency: &NormalisedCurrency,
) -> Result<UpsertOutcome, SyncError> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query!(
        r#"
        SELECT
            a.id AS "id!",
            a.deleted_at,
            a.asset_name AS "asset_name!",
            a.symbol,
            f.numeric_code
        FROM assets AS a
        LEFT JOIN fiat_assets AS f ON f.asset_id = a.id
        WHERE a.asset_class = 'fiat' AND a.code = $1
        "#,
        currency.code,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let (asset_id, outcome) = match existing {
        Some(row) if row.deleted_at.is_some() => return Ok(UpsertOutcome::SkippedDeleted),
        Some(row)
            if row.asset_name == currency.name
                && row.symbol.as_deref() == currency.symbol.as_deref()
                && row.numeric_code.as_deref() == currency.numeric_code.as_deref() =>
        {
            return Ok(UpsertOutcome::Unchanged);
        }
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
            (row.id, UpsertOutcome::Updated)
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
            (id, UpsertOutcome::Inserted)
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
    Ok(outcome)
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

    /// `(asset_name, numeric_code, minor_units, is_active, is_deleted, updated_at)`.
    /// Uses the runtime `sqlx::query` form, not the macro: `cargo sqlx prepare`
    /// does not cache test-only queries.
    async fn fiat(
        pool: &PgPool,
        code: &str,
    ) -> (
        String,
        Option<String>,
        i16,
        bool,
        bool,
        sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    ) {
        use sqlx::Row;

        let row = sqlx::query(
            r#"
            SELECT
                a.asset_name,
                f.numeric_code,
                f.minor_units,
                a.is_active,
                (a.deleted_at IS NOT NULL) AS deleted,
                a.updated_at
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
            row.get("updated_at"),
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
        assert_eq!(summary.unchanged, 0);
        assert_eq!(summary.skipped, 0);

        let (name, numeric, minor_units, active, deleted, _) = fiat(&pool, "EUR").await;
        assert_eq!(name, "Euro (renamed)");
        assert_eq!(numeric.as_deref(), Some("978"));
        assert_eq!(minor_units, 2);
        assert!(active && !deleted);

        let (name, numeric, minor_units, active, deleted, _) = fiat(&pool, "ZZZ").await;
        assert_eq!(name, "Test Currency");
        assert_eq!(numeric.as_deref(), Some("999"));
        assert_eq!(minor_units, 2);
        assert!(active && !deleted);
    }

    #[sqlx::test]
    async fn an_unchanged_currency_is_not_rewritten(pool: PgPool) {
        // A rename is an update.
        let first = apply(
            &pool,
            &[normalised(
                "EUR",
                Some("978"),
                "Euro Zone Currency",
                Some("€"),
            )],
        )
        .await
        .expect("first sync");
        assert_eq!(first.updated, 1);
        let (_, _, _, _, _, after_update) = fiat(&pool, "EUR").await;

        // The same data a second time changes nothing — no `updated_at` bump.
        let second = apply(
            &pool,
            &[normalised(
                "EUR",
                Some("978"),
                "Euro Zone Currency",
                Some("€"),
            )],
        )
        .await
        .expect("second sync");
        assert_eq!(second.unchanged, 1);
        assert_eq!(second.updated, 0);
        assert_eq!(second.inserted, 0);

        let (_, _, _, _, _, after_noop) = fiat(&pool, "EUR").await;
        assert_eq!(after_update, after_noop);
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
        assert_eq!(summary.unchanged, 0);

        let (name, _, _, _, deleted, _) = fiat(&pool, "USD").await;
        assert!(deleted);
        assert_ne!(name, "US Dollar CHANGED");
    }
}
