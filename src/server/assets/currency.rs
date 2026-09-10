//! Queries for the fiat currencies held in the `assets` / `fiat_assets` tables,
//! the domain error, and the fiat-specific input validation for the currency
//! server functions (the alphabetic and numeric code shapes, the minor-unit
//! range). Validation of the shared `assets` columns (name, symbol) lives in
//! [`crate::server::assets::validate`].
//!
//! A currency is an `assets` row with `asset_class = 'fiat'` joined to its
//! `fiat_assets` detail row (`numeric_code`, `minor_units`). All values arriving
//! from a server function are untrusted; the `validate_*` functions here are the
//! authoritative checks so a bad request gets a specific message instead of a
//! generic database error.

use leptos::logging;
use serde::Deserialize;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::assets::{rates, validate};

#[derive(Debug, thiserror::Error)]
pub enum CurrencyError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("a currency with this code already exists")]
    CodeTaken,

    #[error("currency not found")]
    NotFound,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for CurrencyError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("currency: database error: {err}");
        CurrencyError::Internal
    }
}

/// The columns needed to render a currency to the browser.
pub struct CurrencyRecord {
    pub id: Uuid,
    pub alphabetic_code: String,
    pub numeric_code: Option<String>,
    pub currency_name: String,
    pub symbol: Option<String>,
    pub minor_units: i16,
    pub is_active: bool,
}

/// Normalize and validate a 3-letter alphabetic currency code. This is the
/// authoritative shape check for a fiat asset's `code`; the `assets` table
/// only constrains `code` to be non-blank.
pub fn validate_alphabetic_code(input: &str) -> Result<String, CurrencyError> {
    let code = input.trim().to_uppercase();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(CurrencyError::InvalidInput(
            "alphabetic code must be exactly 3 letters",
        ));
    }
    Ok(code)
}

/// Normalize and validate an optional 3-digit numeric currency code (mirrors
/// the `fiat_assets_numeric_code_valid` check constraint). Blank input is
/// treated as absent.
pub fn validate_numeric_code(input: Option<&str>) -> Result<Option<String>, CurrencyError> {
    let Some(code) = input.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CurrencyError::InvalidInput(
            "numeric code must be exactly 3 digits",
        ));
    }
    Ok(Some(code.to_owned()))
}

/// Validate the minor-unit count is within the range the schema allows
/// (mirrors `fiat_assets_minor_units_valid`).
pub fn validate_minor_units(input: i16) -> Result<i16, CurrencyError> {
    if !(0..=18).contains(&input) {
        return Err(CurrencyError::InvalidInput(
            "minor units must be between 0 and 18",
        ));
    }
    Ok(input)
}

/// List active, non-deleted currencies ordered by alphabetic code.
pub async fn list_active(pool: &PgPool) -> Result<Vec<CurrencyRecord>, CurrencyError> {
    let records = sqlx::query_as!(
        CurrencyRecord,
        r#"
        SELECT
            a.id,
            a.code AS alphabetic_code,
            f.numeric_code,
            a.asset_name AS currency_name,
            a.symbol,
            f.minor_units AS "minor_units!",
            a.is_active
        FROM assets AS a
        INNER JOIN fiat_assets AS f ON f.asset_id = a.id
        WHERE a.asset_class = 'fiat' AND a.is_active = TRUE AND a.deleted_at IS NULL
        ORDER BY a.code
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Insert a new currency as a fiat `assets` row and its `fiat_assets` detail
/// row, in one transaction, and return the created record.
///
/// A unique-constraint violation on either the alphabetic or the numeric code
/// is mapped to [`CurrencyError::CodeTaken`]; the transaction is rolled back so
/// no orphan `assets` row is left behind.
pub async fn create(
    pool: &PgPool,
    alphabetic_code: &str,
    numeric_code: Option<&str>,
    currency_name: &str,
    symbol: Option<&str>,
    minor_units: i16,
) -> Result<CurrencyRecord, CurrencyError> {
    let mut tx = pool.begin().await?;

    let asset = match sqlx::query!(
        r#"
        INSERT INTO assets (asset_class, code, asset_name, symbol)
        VALUES ('fiat', $1, $2, $3)
        RETURNING id, is_active
        "#,
        alphabetic_code,
        currency_name,
        symbol,
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(CurrencyError::CodeTaken);
        }
        Err(err) => return Err(err.into()),
    };

    if let Err(err) = sqlx::query!(
        r#"
        INSERT INTO fiat_assets (asset_id, numeric_code, minor_units)
        VALUES ($1, $2, $3)
        "#,
        asset.id,
        numeric_code,
        minor_units,
    )
    .execute(&mut *tx)
    .await
    {
        return match err {
            sqlx::Error::Database(err) if err.is_unique_violation() => {
                Err(CurrencyError::CodeTaken)
            }
            err => Err(err.into()),
        };
    }

    tx.commit().await?;

    Ok(CurrencyRecord {
        id: asset.id,
        alphabetic_code: alphabetic_code.to_owned(),
        numeric_code: numeric_code.map(str::to_owned),
        currency_name: currency_name.to_owned(),
        symbol: symbol.map(str::to_owned),
        minor_units,
        is_active: asset.is_active,
    })
}

/// Update the mutable fields of a currency (name, symbol, minor units,
/// active flag) across its `assets` and `fiat_assets` rows, in one
/// transaction. The alphabetic and numeric codes are immutable after creation.
///
/// Returns [`CurrencyError::NotFound`] if the id does not match a
/// non-deleted fiat asset.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    currency_name: &str,
    symbol: Option<&str>,
    minor_units: i16,
    is_active: bool,
) -> Result<CurrencyRecord, CurrencyError> {
    let mut tx = pool.begin().await?;

    let asset = sqlx::query!(
        r#"
        UPDATE assets
        SET asset_name = $2,
            symbol = $3,
            is_active = $4,
            updated_at = now()
        WHERE id = $1 AND asset_class = 'fiat' AND deleted_at IS NULL
        RETURNING code
        "#,
        id,
        currency_name,
        symbol,
        is_active,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(asset) = asset else {
        return Err(CurrencyError::NotFound);
    };

    let fiat = sqlx::query!(
        r#"
        UPDATE fiat_assets
        SET minor_units = $2
        WHERE asset_id = $1
        RETURNING numeric_code
        "#,
        id,
        minor_units,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(CurrencyRecord {
        id,
        alphabetic_code: asset.code,
        numeric_code: fiat.numeric_code,
        currency_name: currency_name.to_owned(),
        symbol: symbol.map(str::to_owned),
        minor_units,
        is_active,
    })
}

/// Soft-delete a currency by setting `deleted_at` on its `assets` row.
///
/// Returns [`CurrencyError::NotFound`] if the id does not match a
/// non-deleted fiat asset. The `fiat_assets` detail row and any rows in other
/// tables that reference this asset are left untouched.
pub async fn soft_delete(pool: &PgPool, id: Uuid) -> Result<(), CurrencyError> {
    let result = sqlx::query!(
        r#"
        UPDATE assets
        SET deleted_at = now()
        WHERE id = $1 AND asset_class = 'fiat' AND deleted_at IS NULL
        "#,
        id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(CurrencyError::NotFound);
    }

    Ok(())
}

// --- Monthly sync from Frankfurter -----------------------------------------

/// One entry of the Frankfurter v2 `/v2/currencies` response. `start_date` /
/// `end_date` are present in the payload but not stored.
#[derive(Deserialize)]
struct CurrencyRow {
    iso_code: String,
    iso_numeric: Option<String>,
    name: String,
    symbol: Option<String>,
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
fn normalise(row: CurrencyRow) -> Option<NormalisedCurrency> {
    let code = validate_alphabetic_code(&row.iso_code).ok()?;
    let name = validate::name(&row.name).ok()?;
    let symbol = validate::symbol(row.symbol.as_deref());
    let numeric_code = validate_numeric_code(row.iso_numeric.as_deref()).unwrap_or(None);
    Some(NormalisedCurrency {
        code,
        numeric_code,
        name,
        symbol,
    })
}

/// What one [`sync_from_frankfurter`] run did, for the log line.
pub struct SyncSummary {
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Fetch `/v2/currencies` from Frankfurter and upsert every valid entry into
/// `assets` / `fiat_assets`: insert currencies not present yet, and refresh the
/// name, symbol, and numeric code of ones that are. `is_active`, `deleted_at`,
/// and `minor_units` are never touched here — a soft-deleted currency stays
/// deleted, and a newly inserted one takes the schema default `minor_units = 2`.
pub async fn sync_from_frankfurter(pool: &PgPool) -> Result<SyncSummary, CurrencyError> {
    let url = format!("{}/v2/currencies", rates::api_root());

    let client = reqwest::Client::builder()
        .timeout(rates::HTTP_TIMEOUT)
        .build()
        .map_err(|err| {
            logging::error!("currency sync: http client: {err}");
            CurrencyError::Internal
        })?;

    let body = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| {
            logging::error!("currency sync: fetch: {err}");
            CurrencyError::Internal
        })?
        .text()
        .await
        .map_err(|err| {
            logging::error!("currency sync: read body: {err}");
            CurrencyError::Internal
        })?;

    let rows: Vec<CurrencyRow> = serde_json::from_str(&body).map_err(|err| {
        logging::error!("currency sync: parse: {err}");
        CurrencyError::Internal
    })?;

    let currencies: Vec<NormalisedCurrency> = rows.into_iter().filter_map(normalise).collect();

    apply(pool, &currencies).await
}

/// Upsert the given currencies, one transaction each. Split out from
/// [`sync_from_frankfurter`] so a test can drive the DB effect without HTTP.
async fn apply(
    pool: &PgPool,
    currencies: &[NormalisedCurrency],
) -> Result<SyncSummary, CurrencyError> {
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
) -> Result<Option<bool>, CurrencyError> {
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

    fn row(code: &str, numeric: Option<&str>, name: &str, symbol: Option<&str>) -> CurrencyRow {
        CurrencyRow {
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
