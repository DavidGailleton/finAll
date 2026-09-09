//! Creating a transfer: moving money between two of the signed-in user's own
//! accounts.
//!
//! A transfer is three rows written in one database transaction — a negative
//! leg on the source account, a positive leg on the destination account (both
//! ordinary [`transactions`] rows), and the [`transfers`] row that links them.
//! Either all three land or none do; a half-written transfer can never exist.
//!
//! The two leg amounts, and the currency each is in, are supplied independently
//! by the caller. Nothing is converted and no exchange rate is applied, looked
//! up, or stored — the two legs may be in different currencies. Each leg's
//! `asset_id` must be an active fiat currency but need not be its account's
//! `default_asset_id`, exactly like a plain transaction.
//!
//! Transfer legs carry no category or merchant: a transfer is neither income nor
//! an expense. Editing or deleting a transfer is not built;
//! [`crate::server::transactions::update`] / `soft_delete` already refuse to
//! touch a single leg directly.

use bigdecimal::Zero;
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::{PgConnection, PgPool};

use crate::server::transactions::{self, TransactionError};

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("you cannot transfer money to the same account")]
    SameAccount,

    #[error("the selected currency does not exist")]
    UnknownAsset,

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

/// Parse and validate one leg's amount: the magnitude of money moved, as an
/// exact decimal. Reuses [`transactions::validate_amount`] (non-zero, fits
/// `NUMERIC(38, 18)`) and then requires a strictly positive value — direction is
/// implied by which account is the source, so a signed amount would be
/// ambiguous.
pub fn validate_amount(input: &str) -> Result<BigDecimal, TransferError> {
    let amount = transactions::validate_amount(input).map_err(|err| match err {
        TransactionError::InvalidInput(message) => TransferError::InvalidInput(message),
        _ => TransferError::Internal,
    })?;

    if amount <= BigDecimal::zero() {
        return Err(TransferError::InvalidInput(
            "transfer amounts must be positive",
        ));
    }

    Ok(amount)
}

/// The inputs for [`create`]. `source_amount` / `destination_amount` are the
/// positive magnitudes leaving the source and arriving at the destination;
/// `source_asset_id` / `destination_asset_id` name the currency each is in
/// (each must be an active fiat currency, but may differ from that account's
/// default). The booking date is required; the value date is optional. Both
/// dates apply to both legs.
pub struct TransferWrite {
    pub source_account_id: Uuid,
    pub destination_account_id: Uuid,
    pub source_asset_id: Uuid,
    pub destination_asset_id: Uuid,
    pub source_amount: BigDecimal,
    pub destination_amount: BigDecimal,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
}

/// Confirm `account_id` is one of the user's non-deleted accounts.
///
/// Scoping by `user_id` is the authorization check: another user's account is
/// reported as [`TransferError::NotFound`], indistinguishable from a missing
/// one.
async fn assert_account_exists(
    conn: &mut PgConnection,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<(), TransferError> {
    let found = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM accounts
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        account_id,
        user_id,
    )
    .fetch_optional(&mut *conn)
    .await?;

    if found.is_none() {
        return Err(TransferError::NotFound);
    }

    Ok(())
}

/// Confirm `asset_id` is an active, non-deleted fiat currency — one that
/// `list_currencies` would return — not merely any row the foreign key allows.
/// Mirrors the private `assert_currency_exists` in
/// [`crate::server::transactions`] / `crate::server::accounts`, but runs on the
/// transfer's transaction connection.
async fn assert_currency_exists(
    conn: &mut PgConnection,
    asset_id: Uuid,
) -> Result<(), TransferError> {
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
    .fetch_optional(&mut *conn)
    .await?;

    if found.is_none() {
        return Err(TransferError::UnknownAsset);
    }

    Ok(())
}

/// Insert one leg of a transfer and return its transaction id. `amount` is
/// stored verbatim (already signed by the caller).
async fn insert_leg(
    conn: &mut PgConnection,
    user_id: Uuid,
    account_id: Uuid,
    asset_id: Uuid,
    amount: &BigDecimal,
    booking_date: NaiveDate,
    value_date: Option<NaiveDate>,
) -> Result<Uuid, TransferError> {
    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO transactions
            (user_id, account_id, asset_id, amount, booking_date, value_date)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
        user_id,
        account_id,
        asset_id,
        amount,
        booking_date,
        value_date,
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(id)
}

/// Move money between two of the user's own accounts, writing both transaction
/// legs and the linking `transfers` row in a single database transaction.
///
/// Returns [`TransferError::SameAccount`] if the two accounts are the same,
/// [`TransferError::NotFound`] if either account is not one of this user's
/// non-deleted accounts, and [`TransferError::UnknownAsset`] if either leg's
/// currency is not an active fiat currency. On any error the transaction is
/// rolled back, so a partial transfer is never persisted.
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    write: &TransferWrite,
) -> Result<Uuid, TransferError> {
    if write.source_account_id == write.destination_account_id {
        return Err(TransferError::SameAccount);
    }

    let mut tx = pool.begin().await?;

    // Every check runs before either leg is inserted, so an invalid account or
    // currency writes nothing at all.
    assert_account_exists(&mut tx, user_id, write.source_account_id).await?;
    assert_account_exists(&mut tx, user_id, write.destination_account_id).await?;
    assert_currency_exists(&mut tx, write.source_asset_id).await?;
    assert_currency_exists(&mut tx, write.destination_asset_id).await?;

    let source_leg = insert_leg(
        &mut tx,
        user_id,
        write.source_account_id,
        write.source_asset_id,
        &(-write.source_amount.clone()),
        write.booking_date,
        write.value_date,
    )
    .await?;

    let destination_leg = insert_leg(
        &mut tx,
        user_id,
        write.destination_account_id,
        write.destination_asset_id,
        &write.destination_amount,
        write.booking_date,
        write.value_date,
    )
    .await?;

    let transfer_id = sqlx::query_scalar!(
        r#"
        INSERT INTO transfers (user_id, source_transaction_id, destination_transaction_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
        user_id,
        source_leg,
        destination_leg,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(transfer_id)
}

/// Pure-input validation tests.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_amount_requires_a_positive_value() {
        assert_eq!(
            validate_amount(" 30.00 ").expect("valid"),
            BigDecimal::from(30)
        );
        assert!(validate_amount("0").is_err());
        assert!(validate_amount("-30.00").is_err());
        assert!(validate_amount("not a number").is_err());
    }
}

/// Authorization: every query is scoped by `user_id`. `assert_account_exists`
/// reports an account that is not the caller's as `NotFound`, so `create` cannot
/// move money into or out of another user's account. Each `#[sqlx::test]` runs
/// against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{create_user, currency_id, date, dec};
    use crate::server::transactions::{list, TransactionFilter};

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

    fn write(source: Uuid, destination: Uuid, asset: Uuid, amount: &str) -> TransferWrite {
        TransferWrite {
            source_account_id: source,
            destination_account_id: destination,
            source_asset_id: asset,
            destination_asset_id: asset,
            source_amount: dec(amount),
            destination_amount: dec(amount),
            booking_date: date(2026, 1, 15),
            value_date: None,
        }
    }

    #[sqlx::test]
    async fn create_writes_both_legs_and_links_them(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings");

        let transfer_id = create(&pool, alice, &write(checking.id, savings.id, eur, "30"))
            .await
            .expect("transfer");

        assert_eq!(balance(&pool, checking.id).await, Some(dec("-30")));
        assert_eq!(balance(&pool, savings.id).await, Some(dec("30")));

        // The two legs are linked by the transfer row…
        let (source_txn, destination_txn): (Uuid, Uuid) = sqlx::query_as(
            "SELECT source_transaction_id, destination_transaction_id FROM transfers WHERE id = $1",
        )
        .bind(transfer_id)
        .fetch_one(&pool)
        .await
        .expect("transfer row");

        let source_row = account_rows(&pool, alice, checking.id).await;
        let destination_row = account_rows(&pool, alice, savings.id).await;
        assert_eq!(source_row[0].id, source_txn);
        assert_eq!(destination_row[0].id, destination_txn);

        // …and neither leg is categorised.
        assert!(source_row[0].category_id.is_none());
        assert!(source_row[0].merchant_id.is_none());
        assert!(destination_row[0].category_id.is_none());
        assert!(destination_row[0].merchant_id.is_none());
    }

    #[sqlx::test]
    async fn create_supports_different_currencies_without_converting(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;
        let euro_account = accounts::create(&pool, alice, "Euro", "bank", eur)
            .await
            .expect("euro account");
        let dollar_account = accounts::create(&pool, alice, "Dollar", "bank", usd)
            .await
            .expect("dollar account");

        create(
            &pool,
            alice,
            &TransferWrite {
                destination_asset_id: usd,
                destination_amount: dec("33"),
                ..write(euro_account.id, dollar_account.id, eur, "30")
            },
        )
        .await
        .expect("transfer");

        // Each leg records the currency named in the write, not its account's
        // default (here they happen to line up, but the write is the source of
        // truth).
        let source_row = account_rows(&pool, alice, euro_account.id).await;
        assert_eq!(source_row[0].amount, dec("-30"));
        assert_eq!(source_row[0].asset_id, eur);

        let destination_row = account_rows(&pool, alice, dollar_account.id).await;
        assert_eq!(destination_row[0].amount, dec("33"));
        assert_eq!(destination_row[0].asset_id, usd);
    }

    #[sqlx::test]
    async fn create_rejects_the_same_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Only", "bank", eur)
            .await
            .expect("account");

        assert!(matches!(
            create(&pool, alice, &write(account.id, account.id, eur, "10")).await,
            Err(TransferError::SameAccount)
        ));
        assert!(account_rows(&pool, alice, account.id).await.is_empty());
    }

    #[sqlx::test]
    async fn create_denies_another_users_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let alices = accounts::create(&pool, alice, "Alice", "bank", eur)
            .await
            .expect("alice's account");
        let bobs = accounts::create(&pool, bob, "Bob", "bank", eur)
            .await
            .expect("bob's account");

        assert!(matches!(
            create(&pool, alice, &write(alices.id, bobs.id, eur, "10")).await,
            Err(TransferError::NotFound)
        ));

        // The source leg was never written: an invalid destination rolls the
        // whole transfer back.
        assert!(account_rows(&pool, alice, alices.id).await.is_empty());
    }

    #[sqlx::test]
    async fn create_rejects_an_unknown_currency(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("checking");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("savings");

        assert!(matches!(
            create(
                &pool,
                alice,
                &TransferWrite {
                    source_asset_id: Uuid::nil(),
                    ..write(checking.id, savings.id, eur, "10")
                },
            )
            .await,
            Err(TransferError::UnknownAsset)
        ));

        // Nothing was written: the currency is checked before either insert.
        assert!(account_rows(&pool, alice, checking.id).await.is_empty());
        assert!(account_rows(&pool, alice, savings.id).await.is_empty());
    }
}
