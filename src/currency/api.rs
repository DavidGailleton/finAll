//! Server functions the browser calls to list and manage currencies.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::currency::types::CurrencyDto;

/// List active, non-deleted currencies, ordered by alphabetic code.
#[server]
pub async fn list_currencies() -> Result<Vec<CurrencyDto>, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::currency::{self, CurrencyError};

    let pool = expect_context::<sqlx::PgPool>();

    if extract::current_user(&pool).await?.is_none() {
        return Err(CurrencyError::Unauthorized.into());
    }

    let records = currency::list_active(&pool).await?;

    Ok(records
        .into_iter()
        .map(|record| CurrencyDto {
            id: record.id.to_string(),
            alphabetic_code: record.alphabetic_code,
            numeric_code: record.numeric_code,
            currency_name: record.currency_name,
            symbol: record.symbol,
            minor_units: record.minor_units,
            is_active: record.is_active,
        })
        .collect())
}

/// Create a new currency.
#[server]
pub async fn create_currency(
    alphabetic_code: String,
    numeric_code: Option<String>,
    currency_name: String,
    symbol: Option<String>,
    minor_units: i16,
) -> Result<CurrencyDto, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::currency::{self, CurrencyError};

    let pool = expect_context::<sqlx::PgPool>();

    if extract::current_user(&pool).await?.is_none() {
        return Err(CurrencyError::Unauthorized.into());
    }

    let alphabetic_code = currency::validate_alphabetic_code(&alphabetic_code)?;
    let numeric_code = currency::validate_numeric_code(numeric_code.as_deref())?;
    let currency_name = currency::validate_currency_name(&currency_name)?;
    let symbol = currency::validate_symbol(symbol.as_deref());
    let minor_units = currency::validate_minor_units(minor_units)?;

    let record = currency::create(
        &pool,
        &alphabetic_code,
        numeric_code.as_deref(),
        &currency_name,
        symbol.as_deref(),
        minor_units,
    )
    .await?;

    Ok(CurrencyDto {
        id: record.id.to_string(),
        alphabetic_code: record.alphabetic_code,
        numeric_code: record.numeric_code,
        currency_name: record.currency_name,
        symbol: record.symbol,
        minor_units: record.minor_units,
        is_active: record.is_active,
    })
}

/// Update a currency's name, symbol, minor units, and active flag. The
/// alphabetic and numeric codes are immutable after creation.
#[server]
pub async fn update_currency(
    id: String,
    currency_name: String,
    symbol: Option<String>,
    minor_units: i16,
    is_active: bool,
) -> Result<CurrencyDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::currency::{self, CurrencyError};

    let pool = expect_context::<sqlx::PgPool>();

    if extract::current_user(&pool).await?.is_none() {
        return Err(CurrencyError::Unauthorized.into());
    }

    let id =
        Uuid::parse_str(&id).map_err(|_| CurrencyError::InvalidInput("invalid currency id"))?;
    let currency_name = currency::validate_currency_name(&currency_name)?;
    let symbol = currency::validate_symbol(symbol.as_deref());
    let minor_units = currency::validate_minor_units(minor_units)?;

    let record = currency::update(
        &pool,
        id,
        &currency_name,
        symbol.as_deref(),
        minor_units,
        is_active,
    )
    .await?;

    Ok(CurrencyDto {
        id: record.id.to_string(),
        alphabetic_code: record.alphabetic_code,
        numeric_code: record.numeric_code,
        currency_name: record.currency_name,
        symbol: record.symbol,
        minor_units: record.minor_units,
        is_active: record.is_active,
    })
}

/// Soft-delete a currency. Existing rows in other tables that reference it
/// are left untouched.
#[server]
pub async fn delete_currency(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::currency::{self, CurrencyError};

    let pool = expect_context::<sqlx::PgPool>();

    if extract::current_user(&pool).await?.is_none() {
        return Err(CurrencyError::Unauthorized.into());
    }

    let id =
        Uuid::parse_str(&id).map_err(|_| CurrencyError::InvalidInput("invalid currency id"))?;

    currency::soft_delete(&pool, id).await?;

    Ok(())
}
