//! The single-number balance of one account.
//!
//! Every transaction is recorded in its account's currency at its booking-date
//! FX rate (see [`crate::server::transactions`]), so an account's balance is now
//! just the signed sum of those account-currency amounts — the reworked
//! `account_balances` view already does that per account. This module reads it
//! and adds the ownership check.
//!
//! Every query is scoped by `user_id` so one user can never read another's
//! balance.

use bigdecimal::BigDecimal;
use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::accounts::{self, AccountError};

#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("no exchange rate available to value this account")]
    RateUnavailable,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for BalanceError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("balances: database error: {err}");
        BalanceError::Internal
    }
}

impl From<AccountError> for BalanceError {
    fn from(err: AccountError) -> Self {
        match err {
            AccountError::Unauthorized => BalanceError::Unauthorized,
            AccountError::NotFound => BalanceError::NotFound,
            _ => BalanceError::Internal,
        }
    }
}

/// An account's balance as one number in its default currency.
pub struct TotalBalance {
    pub amount: BigDecimal,
    pub alphabetic_code: String,
}

/// The current user's balance for one account, as one number in the account's
/// default currency.
///
/// Fails with [`BalanceError::RateUnavailable`] when the account holds a
/// foreign-currency transaction whose conversion is still pending (its
/// booking-date rate has not been fetched yet — the background backfill fills
/// these shortly after startup).
pub async fn account_total(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<TotalBalance, BalanceError> {
    let account = accounts::find_for_user(pool, user_id, account_id).await?;
    let alphabetic_code = default_currency_code(pool, account.default_asset_id).await?;

    if has_pending_conversion(pool, user_id, account_id).await? {
        return Err(BalanceError::RateUnavailable);
    }

    let amount = sqlx::query_scalar!(
        r#"
        SELECT balance AS "balance!"
        FROM account_balances
        WHERE user_id = $1 AND account_id = $2
        "#,
        user_id,
        account_id,
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| BigDecimal::from(0));

    Ok(TotalBalance {
        amount,
        alphabetic_code,
    })
}

/// The alphabetic code of an account's default currency.
async fn default_currency_code(pool: &PgPool, asset_id: Uuid) -> Result<String, BalanceError> {
    sqlx::query_scalar!("SELECT code FROM assets WHERE id = $1", asset_id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// Whether the account has a non-deleted transaction whose account-currency
/// amount has not been computed yet.
async fn has_pending_conversion(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<bool, BalanceError> {
    let pending = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM transactions
            WHERE user_id = $1
              AND account_id = $2
              AND deleted_at IS NULL
              AND account_amount IS NULL
        ) AS "pending!"
        "#,
        user_id,
        account_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(pending)
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::test_support::{
        create_user, currency_id, date, dec, insert_foreign_transaction, insert_transaction,
    };

    #[sqlx::test]
    async fn account_total_denies_another_users_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        let denied = account_total(&pool, bob, account.id).await;
        assert!(matches!(denied, Err(BalanceError::NotFound)));

        let total = account_total(&pool, alice, account.id)
            .await
            .expect("owner reads it");
        assert_eq!(total.amount, dec("100.00"));
        assert_eq!(total.alphabetic_code, "EUR");
    }

    #[sqlx::test]
    async fn account_total_sums_only_the_requested_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let first = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's first account");
        let second = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("alice's second account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("bob's account");

        insert_transaction(&pool, alice, first.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, first.id, eur, "25.50", date(2026, 1, 16)).await;
        insert_transaction(&pool, alice, second.id, eur, "999.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, bob, bobs.id, eur, "777.00", date(2026, 1, 15)).await;

        let total = account_total(&pool, alice, first.id)
            .await
            .expect("owner reads it");
        assert_eq!(total.amount, dec("125.50"));
    }

    #[sqlx::test]
    async fn account_total_is_zero_for_an_account_with_no_transactions(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let total = account_total(&pool, alice, account.id)
            .await
            .expect("owner reads it");
        assert_eq!(total.amount, dec("0"));
    }

    #[sqlx::test]
    async fn a_foreign_transaction_folds_in_at_its_recorded_account_amount(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;

        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");
        insert_transaction(&pool, alice, account.id, eur, "200.00", date(2026, 1, 10)).await;
        // A $100 transaction recorded as EUR 92.00.
        insert_foreign_transaction(
            &pool,
            alice,
            account.id,
            usd,
            "100.00",
            Some("92.00"),
            date(2026, 1, 12),
        )
        .await;

        let total = account_total(&pool, alice, account.id)
            .await
            .expect("owner reads it");
        assert_eq!(total.amount, dec("292.00"));
    }

    #[sqlx::test]
    async fn a_pending_conversion_makes_the_balance_unavailable(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;

        let account = accounts::create(&pool, alice, "Alice Bank", "bank", eur)
            .await
            .expect("account");
        insert_transaction(&pool, alice, account.id, eur, "200.00", date(2026, 1, 10)).await;
        insert_foreign_transaction(
            &pool,
            alice,
            account.id,
            usd,
            "100.00",
            None,
            date(2026, 1, 12),
        )
        .await;

        let result = account_total(&pool, alice, account.id).await;
        assert!(matches!(result, Err(BalanceError::RateUnavailable)));
    }
}
