//! Queries for the signed-in user's accounts (`accounts` table), the domain
//! error, and the input validation for the account server functions.
//!
//! Every value arriving from a server function is untrusted. `validate_name` is
//! the authoritative check for `account_name`; the `account_type` value is
//! constrained to the [`crate::accounts::types::AccountType`] variants before it
//! reaches this module, and `assert_currency_exists` confirms the chosen
//! `default_asset_id` is a currency the user could actually have picked. Each
//! query is scoped by `user_id` so one user can never read or change another's
//! accounts.

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("the selected currency does not exist")]
    UnknownAsset,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for AccountError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("accounts: database error: {err}");
        AccountError::Internal
    }
}

/// The columns needed to render an account to the browser. `account_type` is
/// the raw database string; the server function maps it back to an
/// `AccountType`.
pub struct AccountRecord {
    pub id: Uuid,
    pub account_name: String,
    pub account_type: String,
    pub default_asset_id: Uuid,
}

/// Trim the account name and require it non-blank (mirrors
/// `accounts_name_not_empty`).
pub fn validate_name(input: &str) -> Result<String, AccountError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(AccountError::InvalidInput("account name is required"));
    }
    Ok(name.to_owned())
}

/// Confirm `asset_id` is an active, non-deleted fiat currency (i.e. one that
/// `list_currencies` would return), not merely any row the foreign key allows.
async fn assert_currency_exists(pool: &PgPool, asset_id: Uuid) -> Result<(), AccountError> {
    let found = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM assets
        WHERE id = $1
          AND asset_class = 'fiat'
          AND is_active = TRUE
          AND deleted_at IS NULL
        "#,
        asset_id,
    )
    .fetch_optional(pool)
    .await?;

    if found.is_none() {
        return Err(AccountError::UnknownAsset);
    }

    Ok(())
}

/// List the user's active (non-deleted) accounts, ordered by name.
pub async fn list_active_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<AccountRecord>, AccountError> {
    let records = sqlx::query_as!(
        AccountRecord,
        r#"
        SELECT id, account_name, account_type, default_asset_id
        FROM accounts
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY account_name
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Insert a new account for the user and return the created record.
///
/// A foreign-key violation on `default_asset_id` (e.g. the currency was deleted
/// between the check and the insert) is mapped to [`AccountError::UnknownAsset`].
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    account_name: &str,
    account_type: &str,
    default_asset_id: Uuid,
) -> Result<AccountRecord, AccountError> {
    assert_currency_exists(pool, default_asset_id).await?;

    let row = match sqlx::query!(
        r#"
        INSERT INTO accounts (user_id, default_asset_id, account_name, account_type)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
        user_id,
        default_asset_id,
        account_name,
        account_type,
    )
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(AccountError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(AccountRecord {
        id: row.id,
        account_name: account_name.to_owned(),
        account_type: account_type.to_owned(),
        default_asset_id,
    })
}

/// Update the user's account (name, type, currency) and return the updated
/// record.
///
/// Returns [`AccountError::NotFound`] if the id does not match one of this
/// user's non-deleted accounts, and [`AccountError::UnknownAsset`] if
/// `default_asset_id` is not a valid currency.
pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    account_name: &str,
    account_type: &str,
    default_asset_id: Uuid,
) -> Result<AccountRecord, AccountError> {
    assert_currency_exists(pool, default_asset_id).await?;

    let row = match sqlx::query!(
        r#"
        UPDATE accounts
        SET account_name = $3,
            account_type = $4,
            default_asset_id = $5,
            updated_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        RETURNING id
        "#,
        id,
        user_id,
        account_name,
        account_type,
        default_asset_id,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(AccountError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    let Some(row) = row else {
        return Err(AccountError::NotFound);
    };

    Ok(AccountRecord {
        id: row.id,
        account_name: account_name.to_owned(),
        account_type: account_type.to_owned(),
        default_asset_id,
    })
}

/// Soft-delete the user's account by setting `deleted_at`.
///
/// Returns [`AccountError::NotFound`] if the id does not match one of this
/// user's non-deleted accounts.
pub async fn soft_delete(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), AccountError> {
    let result = sqlx::query!(
        r#"
        UPDATE accounts
        SET deleted_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AccountError::NotFound);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_trims() {
        assert_eq!(validate_name("  Checking  ").unwrap(), "Checking");
    }

    #[test]
    fn validate_name_rejects_blank() {
        assert!(validate_name("   ").is_err());
        assert!(validate_name("").is_err());
    }
}
