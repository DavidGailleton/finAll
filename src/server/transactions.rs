//! Query for a signed-in user's transactions on one account (`transactions`
//! joined to `assets` for the currency code and to `categories` / `merchants`
//! for display names) and the domain error.
//!
//! The query is scoped by `user_id` so one user can never read another's
//! transactions.

use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for TransactionError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("transactions: database error: {err}");
        TransactionError::Internal
    }
}

/// The columns needed to render a transaction row to the browser. `amount` is
/// the exact stored decimal; the server function stringifies it.
pub struct TransactionRecord {
    pub id: Uuid,
    pub amount: BigDecimal,
    pub asset_code: String,
    pub booking_date: NaiveDate,
    pub category_name: Option<String>,
    pub merchant_name: Option<String>,
}

/// The 50 most recent non-deleted transactions on one of the user's accounts,
/// newest first (`booking_date` then `created_at`).
///
/// The account is not checked for existence here; a `account_id` that is not
/// one of this user's accounts simply matches no rows.
pub async fn recent_for_account(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<Vec<TransactionRecord>, TransactionError> {
    let records = sqlx::query_as!(
        TransactionRecord,
        r#"
        SELECT
            t.id,
            t.amount,
            a.code AS "asset_code!",
            t.booking_date,
            c.category_name AS "category_name?",
            m.merchant_name AS "merchant_name?"
        FROM transactions AS t
        INNER JOIN assets AS a ON a.id = t.asset_id
        LEFT JOIN categories AS c
            ON c.user_id = t.user_id AND c.id = t.category_id AND c.deleted_at IS NULL
        LEFT JOIN merchants AS m
            ON m.user_id = t.user_id AND m.id = t.merchant_id AND m.deleted_at IS NULL
        WHERE t.user_id = $1 AND t.account_id = $2 AND t.deleted_at IS NULL
        ORDER BY t.booking_date DESC, t.created_at DESC
        LIMIT 50
        "#,
        user_id,
        account_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Authorization: `recent_for_account` is scoped by `user_id`, and an
/// `account_id` belonging to someone else simply matches no rows (an empty
/// list, not an error). Each `#[sqlx::test]` runs against its own freshly
/// migrated database.
///
/// Cross-user category or merchant names cannot leak through the two
/// `LEFT JOIN`s: `transactions` carries composite foreign keys to
/// `categories (user_id, id)` and `merchants (user_id, id)`, so a row pointing
/// at another user's category cannot be inserted in the first place.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{create_user, currency_id, date, insert_transaction};

    #[sqlx::test]
    async fn recent_for_account_returns_nothing_for_another_users_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, account.id, eur, "-25.50", date(2026, 1, 16)).await;

        let denied = recent_for_account(&pool, bob, account.id)
            .await
            .expect("query runs");
        assert!(denied.is_empty());

        // Control: the owner sees both rows, so the account really has data.
        let owned = recent_for_account(&pool, alice, account.id)
            .await
            .expect("query runs");
        assert_eq!(owned.len(), 2);
    }

    #[sqlx::test]
    async fn recent_for_account_does_not_leak_the_callers_own_rows_for_a_foreign_account(
        pool: PgPool,
    ) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let alices = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("bob's account");

        insert_transaction(&pool, alice, alices.id, eur, "100.00", date(2026, 1, 15)).await;
        let bobs_transaction =
            insert_transaction(&pool, bob, bobs.id, eur, "42.00", date(2026, 1, 15)).await;

        // Bob asking for Alice's account must get nothing at all -- in
        // particular not his own rows, which is what a dropped `account_id`
        // predicate would return.
        let records = recent_for_account(&pool, bob, alices.id)
            .await
            .expect("query runs");
        assert!(records.is_empty());

        // Control: Bob's own account still returns his row.
        let owned = recent_for_account(&pool, bob, bobs.id)
            .await
            .expect("query runs");
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, bobs_transaction);
    }
}
