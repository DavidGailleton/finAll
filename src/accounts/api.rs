//! Server functions the browser calls to list and manage the signed-in user's
//! accounts.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::accounts::types::{AccountDto, AccountType};

/// List the current user's active, non-deleted accounts, ordered by name.
#[server]
pub async fn list_accounts() -> Result<Vec<AccountDto>, ServerFnError> {
    use crate::server::accounts::{self, AccountError};
    use crate::server::auth::extract;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(AccountError::Unauthorized)?;

    let records = accounts::list_active_for_user(&pool, user.user_id).await?;

    let dtos = records
        .into_iter()
        .map(|record| {
            Ok(AccountDto {
                id: record.id.to_string(),
                account_name: record.account_name,
                account_type: AccountType::from_db_str(&record.account_type)
                    .ok_or(AccountError::Internal)?,
                default_asset_id: record.default_asset_id.to_string(),
            })
        })
        .collect::<Result<Vec<AccountDto>, AccountError>>()?;

    Ok(dtos)
}

/// Create a new account for the current user.
#[server]
pub async fn create_account(
    account_name: String,
    account_type: AccountType,
    default_asset_id: String,
) -> Result<AccountDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::accounts::{self, AccountError};
    use crate::server::auth::extract;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(AccountError::Unauthorized)?;

    let account_name = accounts::validate_name(&account_name)?;
    let default_asset_id = Uuid::parse_str(&default_asset_id)
        .map_err(|_| AccountError::InvalidInput("invalid currency id"))?;

    let record = accounts::create(
        &pool,
        user.user_id,
        &account_name,
        account_type.as_db_str(),
        default_asset_id,
    )
    .await?;

    Ok(AccountDto {
        id: record.id.to_string(),
        account_name: record.account_name,
        account_type,
        default_asset_id: record.default_asset_id.to_string(),
    })
}

/// Update the current user's account: its name, type, and currency.
#[server]
pub async fn update_account(
    id: String,
    account_name: String,
    account_type: AccountType,
    default_asset_id: String,
) -> Result<AccountDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::accounts::{self, AccountError};
    use crate::server::auth::extract;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(AccountError::Unauthorized)?;

    let id = Uuid::parse_str(&id).map_err(|_| AccountError::InvalidInput("invalid account id"))?;
    let account_name = accounts::validate_name(&account_name)?;
    let default_asset_id = Uuid::parse_str(&default_asset_id)
        .map_err(|_| AccountError::InvalidInput("invalid currency id"))?;

    let record = accounts::update(
        &pool,
        user.user_id,
        id,
        &account_name,
        account_type.as_db_str(),
        default_asset_id,
    )
    .await?;

    Ok(AccountDto {
        id: record.id.to_string(),
        account_name: record.account_name,
        account_type,
        default_asset_id: record.default_asset_id.to_string(),
    })
}

/// Soft-delete the current user's account. Rows in other tables that reference
/// it are left untouched.
#[server]
pub async fn delete_account(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::accounts::{self, AccountError};
    use crate::server::auth::extract;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(AccountError::Unauthorized)?;

    let id = Uuid::parse_str(&id).map_err(|_| AccountError::InvalidInput("invalid account id"))?;

    accounts::soft_delete(&pool, user.user_id, id).await?;

    Ok(())
}
