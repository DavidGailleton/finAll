//! Transfers: linking two of the signed-in user's existing transactions as the
//! two legs of one movement of money between accounts.
//!
//! A transfer is a single [`transfers`] row pointing at two [`transactions`]
//! rows that already exist — an outgoing (negative) leg on one account and an
//! incoming (positive) leg on another. It stores no amount, currency, or date of
//! its own: everything lives on the two transactions, which are recorded,
//! edited, and deleted like any other transaction.
//!
//! [`link`] creates that row (rejecting a pair that is the same transaction, on
//! the same account, already linked, or not one negative and one positive).
//! [`unlink`] deletes it, leaving both transactions in place. Deleting either
//! leg (in [`crate::server::transactions::soft_delete`]) also removes the link.
//! The link row is hard-deleted, not soft-deleted: it carries no financial data,
//! and the `transfers` unique constraints would otherwise block re-linking.

use bigdecimal::Zero;
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("one of those transactions could not be found")]
    TransactionNotFound,

    #[error("a transfer links two different transactions")]
    SameTransaction,

    #[error("the two transactions are on the same account")]
    SameAccount,

    #[error("one of those transactions is already part of a transfer")]
    AlreadyLinked,

    #[error("a transfer needs one outgoing transaction and one incoming one")]
    NotOpposite,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for TransferError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("transfers: database error: {err}");
        TransferError::Internal
    }
}

/// Link two of the user's existing transactions as the two legs of a transfer,
/// returning the new transfer id.
///
/// The order of the arguments does not matter: the negative-amount transaction
/// becomes the source and the positive-amount one the destination. Both must be
/// the user's own non-deleted transactions, on different accounts, with opposite
/// signs, and neither already part of a transfer.
pub async fn link(
    pool: &PgPool,
    user_id: Uuid,
    first_transaction_id: Uuid,
    second_transaction_id: Uuid,
) -> Result<Uuid, TransferError> {
    if first_transaction_id == second_transaction_id {
        return Err(TransferError::SameTransaction);
    }

    let mut tx = pool.begin().await?;

    let rows = sqlx::query!(
        r#"
        SELECT
            t.id,
            t.account_id,
            t.amount,
            EXISTS (
                SELECT 1
                FROM transfers AS tr
                WHERE tr.user_id = t.user_id
                  AND (tr.source_transaction_id = t.id OR tr.destination_transaction_id = t.id)
                  AND tr.deleted_at IS NULL
            ) AS "already_linked!"
        FROM transactions AS t
        WHERE t.user_id = $1 AND t.id = ANY($2) AND t.deleted_at IS NULL
        "#,
        user_id,
        &[first_transaction_id, second_transaction_id][..],
    )
    .fetch_all(&mut *tx)
    .await?;

    if rows.len() != 2 {
        return Err(TransferError::TransactionNotFound);
    }
    if rows.iter().any(|row| row.already_linked) {
        return Err(TransferError::AlreadyLinked);
    }
    if rows[0].account_id == rows[1].account_id {
        return Err(TransferError::SameAccount);
    }

    // The `transactions_amount_nonzero` CHECK guarantees neither leg is zero, so
    // one strictly negative and one strictly positive is the only valid pair.
    let zero = BigDecimal::zero();
    let source = rows.iter().find(|row| row.amount < zero);
    let destination = rows.iter().find(|row| row.amount > zero);
    let (source, destination) = match (source, destination) {
        (Some(source), Some(destination)) => (source.id, destination.id),
        _ => return Err(TransferError::NotOpposite),
    };

    let transfer_id = match sqlx::query_scalar!(
        r#"
        INSERT INTO transfers (user_id, source_transaction_id, destination_transaction_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
        user_id,
        source,
        destination,
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        // A concurrent link of one of the same transactions raced us to it.
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(TransferError::AlreadyLinked);
        }
        Err(err) => return Err(err.into()),
    };

    tx.commit().await?;

    Ok(transfer_id)
}

/// Remove the transfer that `transaction_id` is a leg of, leaving both
/// transactions in place. Idempotent: removing a link that is not there (or was
/// already removed) is a success, so the edit form's picker can send a redundant
/// "unlink" without erroring.
pub async fn unlink(
    pool: &PgPool,
    user_id: Uuid,
    transaction_id: Uuid,
) -> Result<(), TransferError> {
    sqlx::query!(
        r#"
        DELETE FROM transfers
        WHERE user_id = $1
          AND (source_transaction_id = $2 OR destination_transaction_id = $2)
        "#,
        user_id,
        transaction_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// One transaction offered in the edit form's "Paired transaction" picker (a
/// candidate to link, or the one currently linked). `label` is a ready-to-show
/// summary built here so the browser needs no decimal or date formatting.
pub struct CandidateRow {
    pub transaction_id: Uuid,
    pub label: String,
}

/// The current pair (if this transaction is already a transfer leg) and the
/// transactions it could be linked to instead.
pub struct LinkOptions {
    pub current_pair: Option<CandidateRow>,
    pub candidates: Vec<CandidateRow>,
}

/// Build the display label for a candidate: `date · account · signed amount code`.
fn candidate_label(
    booking_date: NaiveDate,
    account_name: &str,
    amount: &BigDecimal,
    currency_code: &str,
) -> String {
    let sign = if amount > &BigDecimal::zero() {
        "+"
    } else {
        ""
    };
    format!("{booking_date} \u{b7} {account_name} \u{b7} {sign}{amount} {currency_code}")
}

/// The choices for the edit form's "Paired transaction" field for one of the
/// user's transactions: the transaction it is currently linked to, if any, and
/// the transactions it could be linked to (a different account, the opposite
/// amount sign, not already part of a transfer). Newest first, capped so the
/// `<select>` stays usable.
pub async fn link_options(
    pool: &PgPool,
    user_id: Uuid,
    transaction_id: Uuid,
) -> Result<LinkOptions, TransferError> {
    let anchor = sqlx::query!(
        r#"
        SELECT account_id, amount
        FROM transactions
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        transaction_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(TransferError::TransactionNotFound)?;

    let current_pair = sqlx::query!(
        r#"
        SELECT
            pair.id,
            pair.booking_date,
            pair_acc.account_name AS "account_name!",
            pair.amount,
            pair_asset.code AS "currency_code!"
        FROM transfers AS tr
        INNER JOIN transactions AS pair
            ON pair.id = CASE
                WHEN tr.source_transaction_id = $1 THEN tr.destination_transaction_id
                ELSE tr.source_transaction_id
            END
        INNER JOIN accounts AS pair_acc
            ON pair_acc.user_id = tr.user_id AND pair_acc.id = pair.account_id
        INNER JOIN assets AS pair_asset ON pair_asset.id = pair.asset_id
        WHERE tr.user_id = $2
          AND (tr.source_transaction_id = $1 OR tr.destination_transaction_id = $1)
          AND tr.deleted_at IS NULL
        "#,
        transaction_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?
    .map(|row| CandidateRow {
        transaction_id: row.id,
        label: candidate_label(
            row.booking_date,
            &row.account_name,
            &row.amount,
            &row.currency_code,
        ),
    });

    let want_positive = anchor.amount < BigDecimal::zero();

    let candidates = sqlx::query!(
        r#"
        SELECT
            t.id,
            t.booking_date,
            acc.account_name AS "account_name!",
            t.amount,
            a.code AS "currency_code!"
        FROM transactions AS t
        INNER JOIN accounts AS acc ON acc.user_id = t.user_id AND acc.id = t.account_id
        INNER JOIN assets AS a ON a.id = t.asset_id
        WHERE t.user_id = $1
          AND t.deleted_at IS NULL
          AND acc.deleted_at IS NULL
          AND t.id <> $2
          AND t.account_id <> $3
          AND (t.amount > 0) = $4
          AND NOT EXISTS (
              SELECT 1
              FROM transfers AS tr
              WHERE tr.user_id = t.user_id
                AND (tr.source_transaction_id = t.id OR tr.destination_transaction_id = t.id)
                AND tr.deleted_at IS NULL
          )
        ORDER BY t.booking_date DESC, t.id DESC
        LIMIT 100
        "#,
        user_id,
        transaction_id,
        anchor.account_id,
        want_positive,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| CandidateRow {
        transaction_id: row.id,
        label: candidate_label(
            row.booking_date,
            &row.account_name,
            &row.amount,
            &row.currency_code,
        ),
    })
    .collect();

    Ok(LinkOptions {
        current_pair,
        candidates,
    })
}

/// Authorization: every query is scoped by `user_id`, so a transaction that is
/// not the caller's is reported as [`TransferError::TransactionNotFound`] and a
/// transfer that is not the caller's is invisible to [`unlink`] and
/// [`link_options`]. Each `#[sqlx::test]` runs against its own freshly migrated
/// database.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{create_user, currency_id, date, dec, insert_transaction};
    use crate::server::transactions::{self, list, TransactionFilter};

    /// The caller's transactions on one account, newest first.
    async fn account_rows(
        pool: &PgPool,
        user_id: Uuid,
        account_id: Uuid,
    ) -> Vec<transactions::TransactionRecord> {
        list(
            pool,
            user_id,
            TransactionFilter {
                account_id: Some(account_id),
                ..TransactionFilter::default()
            },
        )
        .await
        .expect("list runs")
        .records
    }

    async fn balance(pool: &PgPool, account_id: Uuid) -> Option<BigDecimal> {
        sqlx::query_scalar("SELECT balance FROM account_balances WHERE account_id = $1")
            .bind(account_id)
            .fetch_optional(pool)
            .await
            .expect("query runs")
    }

    /// Alice with a checking and a savings account, both EUR.
    async fn alice_with_two_accounts(pool: &PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
        let alice = create_user(pool, "alice@example.test").await;
        let eur = currency_id(pool, "EUR").await;
        let checking = accounts::create(pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking")
            .id;
        let savings = accounts::create(pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings")
            .id;
        (alice, eur, checking, savings)
    }

    #[sqlx::test]
    async fn link_connects_two_existing_transactions_without_touching_them(pool: PgPool) {
        let (alice, eur, checking, savings) = alice_with_two_accounts(&pool).await;
        let out = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let into = insert_transaction(&pool, alice, savings, eur, "30", date(2026, 1, 15)).await;

        let transfer_id = link(&pool, alice, into, out).await.expect("link");

        // No new transactions were written and the balances are unchanged.
        assert_eq!(account_rows(&pool, alice, checking).await.len(), 1);
        assert_eq!(account_rows(&pool, alice, savings).await.len(), 1);
        assert_eq!(balance(&pool, checking).await, Some(dec("-30")));
        assert_eq!(balance(&pool, savings).await, Some(dec("30")));

        // The negative leg is the source regardless of argument order.
        let (source, destination): (Uuid, Uuid) = sqlx::query_as(
            "SELECT source_transaction_id, destination_transaction_id FROM transfers WHERE id = $1",
        )
        .bind(transfer_id)
        .fetch_one(&pool)
        .await
        .expect("transfer row");
        assert_eq!(source, out);
        assert_eq!(destination, into);
    }

    #[sqlx::test]
    async fn link_rejects_the_same_transaction(pool: PgPool) {
        let (alice, eur, checking, _) = alice_with_two_accounts(&pool).await;
        let txn = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;

        assert!(matches!(
            link(&pool, alice, txn, txn).await,
            Err(TransferError::SameTransaction)
        ));
    }

    #[sqlx::test]
    async fn link_rejects_the_same_account(pool: PgPool) {
        let (alice, eur, checking, _) = alice_with_two_accounts(&pool).await;
        let out = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let into = insert_transaction(&pool, alice, checking, eur, "30", date(2026, 1, 15)).await;

        assert!(matches!(
            link(&pool, alice, out, into).await,
            Err(TransferError::SameAccount)
        ));
    }

    #[sqlx::test]
    async fn link_rejects_two_transactions_with_the_same_sign(pool: PgPool) {
        let (alice, eur, checking, savings) = alice_with_two_accounts(&pool).await;
        let one = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let two = insert_transaction(&pool, alice, savings, eur, "-30", date(2026, 1, 15)).await;

        assert!(matches!(
            link(&pool, alice, one, two).await,
            Err(TransferError::NotOpposite)
        ));
    }

    #[sqlx::test]
    async fn link_rejects_an_already_linked_transaction(pool: PgPool) {
        let (alice, eur, checking, savings) = alice_with_two_accounts(&pool).await;
        let out = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let into = insert_transaction(&pool, alice, savings, eur, "30", date(2026, 1, 15)).await;
        link(&pool, alice, out, into).await.expect("first link");

        let other = insert_transaction(&pool, alice, savings, eur, "30", date(2026, 1, 16)).await;
        assert!(matches!(
            link(&pool, alice, out, other).await,
            Err(TransferError::AlreadyLinked)
        ));
    }

    #[sqlx::test]
    async fn link_denies_another_users_transaction(pool: PgPool) {
        let (alice, eur, checking, _) = alice_with_two_accounts(&pool).await;
        let bob = create_user(&pool, "bob@example.test").await;
        let bobs = accounts::create(&pool, bob, "Bob", "bank", eur)
            .await
            .expect("bob's account")
            .id;
        let alices =
            insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let bobs_txn = insert_transaction(&pool, bob, bobs, eur, "30", date(2026, 1, 15)).await;

        assert!(matches!(
            link(&pool, alice, alices, bobs_txn).await,
            Err(TransferError::TransactionNotFound)
        ));
    }

    #[sqlx::test]
    async fn unlink_removes_the_link_and_keeps_both_transactions(pool: PgPool) {
        let (alice, eur, checking, savings) = alice_with_two_accounts(&pool).await;
        let out = insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let into = insert_transaction(&pool, alice, savings, eur, "30", date(2026, 1, 15)).await;
        link(&pool, alice, out, into).await.expect("link");

        unlink(&pool, alice, out).await.expect("unlink");

        // Both transactions and both balances survive; the row is no longer a leg.
        assert_eq!(balance(&pool, checking).await, Some(dec("-30")));
        assert_eq!(balance(&pool, savings).await, Some(dec("30")));
        assert!(account_rows(&pool, alice, checking).await[0]
            .transfer_id
            .is_none());

        // A second unlink is still a success.
        unlink(&pool, alice, out).await.expect("idempotent unlink");
    }

    #[sqlx::test]
    async fn link_options_lists_opposite_sign_transactions_in_other_accounts(pool: PgPool) {
        let (alice, eur, checking, savings) = alice_with_two_accounts(&pool).await;
        let anchor =
            insert_transaction(&pool, alice, checking, eur, "-30", date(2026, 1, 15)).await;
        let match_1 = insert_transaction(&pool, alice, savings, eur, "30", date(2026, 1, 15)).await;
        // Same sign as the anchor — not a candidate.
        insert_transaction(&pool, alice, savings, eur, "-30", date(2026, 1, 15)).await;
        // Same account as the anchor — not a candidate.
        insert_transaction(&pool, alice, checking, eur, "30", date(2026, 1, 15)).await;

        let options = link_options(&pool, alice, anchor).await.expect("options");
        assert!(options.current_pair.is_none());
        assert_eq!(options.candidates.len(), 1);
        assert_eq!(options.candidates[0].transaction_id, match_1);

        // Once linked, the pair is reported and drops out of the candidate list.
        link(&pool, alice, anchor, match_1).await.expect("link");
        let options = link_options(&pool, alice, anchor).await.expect("options");
        assert_eq!(
            options.current_pair.map(|pair| pair.transaction_id),
            Some(match_1)
        );
        assert!(options.candidates.is_empty());
    }
}
