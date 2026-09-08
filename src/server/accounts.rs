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

/// Fetch one of the user's active (non-deleted) accounts by id.
///
/// Returns [`AccountError::NotFound`] if the id does not match one of this
/// user's non-deleted accounts (a mismatched owner is indistinguishable from a
/// missing row).
pub async fn find_for_user(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<AccountRecord, AccountError> {
    let record = sqlx::query_as!(
        AccountRecord,
        r#"
        SELECT id, account_name, account_type, default_asset_id
        FROM accounts
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    record.ok_or(AccountError::NotFound)
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

/// Authorization: every query here is scoped by `user_id`, so one user must
/// never read or change another's accounts. A wrong owner is deliberately
/// reported as [`AccountError::NotFound`], indistinguishable from a missing
/// row, so these tests assert `NotFound` rather than a distinct "forbidden"
/// signal.
///
/// Each `#[sqlx::test]` runs against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::test_support::{create_user, currency_id};

    /// Two users and the currency their accounts are denominated in.
    async fn two_users(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let alice = create_user(pool, "alice@example.test").await;
        let bob = create_user(pool, "bob@example.test").await;
        let eur = currency_id(pool, "EUR").await;
        (alice, bob, eur)
    }

    #[sqlx::test]
    async fn list_active_for_user_returns_only_the_owners_accounts(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;

        let a1 = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's first account");
        let a2 = create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("alice's second account");
        let b1 = create(&pool, bob, "Bob Bank", "bank", eur)
            .await
            .expect("bob's account");

        let alices: Vec<Uuid> = list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(alices.len(), 2);
        assert!(alices.contains(&a1.id));
        assert!(alices.contains(&a2.id));
        assert!(!alices.contains(&b1.id));

        let bobs: Vec<Uuid> = list_active_for_user(&pool, bob)
            .await
            .expect("lists")
            .iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(bobs, vec![b1.id]);
    }

    #[sqlx::test]
    async fn list_active_for_user_is_empty_for_a_user_with_no_accounts(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;

        create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let bobs = list_active_for_user(&pool, bob).await.expect("lists");
        assert!(bobs.is_empty());
    }

    #[sqlx::test]
    async fn find_for_user_denies_another_users_account(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;

        let account = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let denied = find_for_user(&pool, bob, account.id).await;
        assert!(matches!(denied, Err(AccountError::NotFound)));

        // Control: the owner still reaches it, so the id itself is valid.
        let found = find_for_user(&pool, alice, account.id)
            .await
            .expect("owner finds it");
        assert_eq!(found.id, account.id);
    }

    #[sqlx::test]
    async fn create_assigns_the_calling_users_id(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;

        let account = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        assert!(matches!(
            find_for_user(&pool, bob, account.id).await,
            Err(AccountError::NotFound)
        ));
        assert!(find_for_user(&pool, alice, account.id).await.is_ok());
    }

    #[sqlx::test]
    async fn update_denies_another_users_account_and_leaves_it_unchanged(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;
        let usd = currency_id(&pool, "USD").await;

        let account = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let denied = update(&pool, bob, account.id, "Stolen", "bank", usd).await;
        assert!(matches!(denied, Err(AccountError::NotFound)));

        // The rejection must also mean nothing was written.
        let after = find_for_user(&pool, alice, account.id)
            .await
            .expect("owner finds it");
        assert_eq!(after.account_name, "Alice Cash");
        assert_eq!(after.account_type, "cash");
        assert_eq!(after.default_asset_id, eur);
    }

    #[sqlx::test]
    async fn soft_delete_denies_another_users_account_and_leaves_it_visible(pool: PgPool) {
        let (alice, bob, eur) = two_users(&pool).await;

        let account = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let denied = soft_delete(&pool, bob, account.id).await;
        assert!(matches!(denied, Err(AccountError::NotFound)));

        assert!(find_for_user(&pool, alice, account.id).await.is_ok());
    }

    #[sqlx::test]
    async fn find_for_user_rejects_the_owners_own_soft_deleted_account(pool: PgPool) {
        let (alice, _bob, eur) = two_users(&pool).await;

        let account = create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        soft_delete(&pool, alice, account.id)
            .await
            .expect("owner deletes it");

        assert!(matches!(
            find_for_user(&pool, alice, account.id).await,
            Err(AccountError::NotFound)
        ));
        assert!(list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .is_empty());

        // Deleting again is not a second success.
        assert!(matches!(
            soft_delete(&pool, alice, account.id).await,
            Err(AccountError::NotFound)
        ));
    }
}
