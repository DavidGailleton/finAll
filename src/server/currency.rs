//! Queries for the fiat currencies held in the `assets` / `fiat_assets` tables,
//! the domain error, and the input validation for the currency server
//! functions.
//!
//! A currency is an `assets` row with `asset_class = 'fiat'` joined to its
//! `fiat_assets` detail row (`numeric_code`, `minor_units`). All values arriving
//! from a server function are untrusted; the `validate_*` functions here are the
//! authoritative checks so a bad request gets a specific message instead of a
//! generic database error.

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

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

/// Trim and validate the currency name is non-blank (mirrors
/// `assets_name_not_empty`).
pub fn validate_currency_name(input: &str) -> Result<String, CurrencyError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(CurrencyError::InvalidInput("currency name is required"));
    }
    Ok(name.to_owned())
}

/// Trim an optional symbol, treating blank as absent (mirrors
/// `assets_symbol_not_empty`, which only constrains a non-null value).
pub fn validate_symbol(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
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
