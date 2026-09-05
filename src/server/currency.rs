//! Queries against the `currencies` table, its domain error, and the input
//! validation for the currency server functions.
//!
//! All values arriving from a server function are untrusted; the `validate_*`
//! functions here are the authoritative checks and mirror the `currencies`
//! table's `CHECK` constraints so a bad request gets a specific message
//! instead of a generic database error.

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

/// Normalize and validate a 3-letter alphabetic currency code (mirrors the
/// `currencies_alphabetic_code_valid` check constraint).
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
/// the `currencies_numeric_code_valid` check constraint). Blank input is
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
/// `currencies_name_not_empty`).
pub fn validate_currency_name(input: &str) -> Result<String, CurrencyError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(CurrencyError::InvalidInput("currency name is required"));
    }
    Ok(name.to_owned())
}

/// Trim an optional symbol, treating blank as absent (mirrors
/// `currencies_symbol_not_empty`, which only constrains a non-null value).
pub fn validate_symbol(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Validate the minor-unit count is within the range the schema allows
/// (mirrors `currencies_minor_units_valid`).
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
        SELECT id, alphabetic_code, numeric_code, currency_name, symbol, minor_units, is_active
        FROM currencies
        WHERE is_active = TRUE AND deleted_at IS NULL
        ORDER BY alphabetic_code
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Insert a new currency and return the created row.
///
/// A unique-constraint violation on either code is mapped to
/// [`CurrencyError::CodeTaken`].
pub async fn create(
    pool: &PgPool,
    alphabetic_code: &str,
    numeric_code: Option<&str>,
    currency_name: &str,
    symbol: Option<&str>,
    minor_units: i16,
) -> Result<CurrencyRecord, CurrencyError> {
    let result = sqlx::query_as!(
        CurrencyRecord,
        r#"
        INSERT INTO currencies (alphabetic_code, numeric_code, currency_name, symbol, minor_units)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING
            id,
            alphabetic_code,
            numeric_code,
            currency_name,
            symbol,
            minor_units,
            is_active
        "#,
        alphabetic_code,
        numeric_code,
        currency_name,
        symbol,
        minor_units,
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(record) => Ok(record),
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            Err(CurrencyError::CodeTaken)
        }
        Err(err) => Err(err.into()),
    }
}

/// Update the mutable fields of a currency (name, symbol, minor units,
/// active flag). The alphabetic and numeric codes are immutable after
/// creation.
///
/// Returns [`CurrencyError::NotFound`] if the id does not match a
/// non-deleted currency.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    currency_name: &str,
    symbol: Option<&str>,
    minor_units: i16,
    is_active: bool,
) -> Result<CurrencyRecord, CurrencyError> {
    let record = sqlx::query_as!(
        CurrencyRecord,
        r#"
        UPDATE currencies
        SET currency_name = $2,
            symbol = $3,
            minor_units = $4,
            is_active = $5,
            updated_at = now()
        WHERE id = $1 AND deleted_at IS NULL
        RETURNING
            id,
            alphabetic_code,
            numeric_code,
            currency_name,
            symbol,
            minor_units,
            is_active
        "#,
        id,
        currency_name,
        symbol,
        minor_units,
        is_active,
    )
    .fetch_optional(pool)
    .await?;

    record.ok_or(CurrencyError::NotFound)
}

/// Soft-delete a currency by setting `deleted_at`.
///
/// Returns [`CurrencyError::NotFound`] if the id does not match a
/// non-deleted currency. Existing rows in other tables that reference this
/// currency are left untouched.
pub async fn soft_delete(pool: &PgPool, id: Uuid) -> Result<(), CurrencyError> {
    let result = sqlx::query!(
        r#"
        UPDATE currencies
        SET deleted_at = now()
        WHERE id = $1 AND deleted_at IS NULL
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
