//! Queries for a signed-in user's transactions (`transactions` joined to
//! `assets` for the currency code, `accounts` for the account name, and
//! `categories` / `merchants` for display names), the create / edit /
//! soft-delete writes, the input validation for the transaction server
//! functions, and the domain error.
//!
//! Every value arriving from a server function is untrusted. `validate_amount`,
//! `validate_booking_date`, `validate_value_date` and `validate_date_range` are
//! the authoritative checks, mirroring the `transactions` table's constraints.
//! Every query is scoped by `user_id` so one user can never read or change
//! another's transactions.

use std::str::FromStr;

use bigdecimal::Zero;
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

/// The largest number of fractional digits `transactions.amount` can hold
/// (`NUMERIC(38, 18)`).
const AMOUNT_SCALE: i64 = 18;

/// The largest number of integer digits `transactions.amount` can hold
/// (`NUMERIC(38, 18)` leaves 38 - 18 for the integer part).
const AMOUNT_INTEGER_DIGITS: i64 = 20;

/// How many transactions one page of [`list`] holds.
pub const PAGE_SIZE: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("transaction not found")]
    NotFound,

    #[error("the selected currency does not exist")]
    UnknownAsset,

    #[error("this transaction is part of a transfer; edit or delete the transfer instead")]
    PartOfTransfer,

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
    pub asset_id: Uuid,
    pub asset_code: String,
    pub account_id: Uuid,
    pub account_name: String,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub category_name: Option<String>,
    pub merchant_name: Option<String>,
}

/// A keyset cursor: the `(booking_date, id)` of the last row already seen.
/// [`list`] returns rows strictly ordered after it.
pub type Cursor = (NaiveDate, Uuid);

/// The filter for [`list`]. Every field is optional; `from` / `to` bound
/// `booking_date` inclusively.
#[derive(Default)]
pub struct TransactionFilter {
    pub account_id: Option<Uuid>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub after: Option<Cursor>,
}

/// One page of [`list`] results, newest first, plus the cursor for the next
/// page (present only when more rows remain).
pub struct TransactionListPage {
    pub records: Vec<TransactionRecord>,
    pub next: Option<Cursor>,
}

/// One page of the user's non-deleted transactions matching `filter`, newest
/// first (`booking_date` then `id`, both descending). `id` is uuidv7 so it
/// orders by creation time and gives every row a stable total order.
///
/// Transactions on a soft-deleted account are excluded, matching the
/// `account_balances` view.
pub async fn list(
    pool: &PgPool,
    user_id: Uuid,
    filter: TransactionFilter,
) -> Result<TransactionListPage, TransactionError> {
    let (after_date, after_id) = match filter.after {
        Some((date, id)) => (Some(date), Some(id)),
        None => (None, None),
    };
    let limit = PAGE_SIZE as i64 + 1;

    let mut records = sqlx::query_as!(
        TransactionRecord,
        r#"
        SELECT
            t.id,
            t.amount,
            t.asset_id,
            a.code AS "asset_code!",
            t.account_id,
            acc.account_name AS "account_name!",
            t.booking_date,
            t.value_date,
            c.category_name AS "category_name?",
            m.merchant_name AS "merchant_name?"
        FROM transactions AS t
        INNER JOIN assets AS a ON a.id = t.asset_id
        INNER JOIN accounts AS acc
            ON acc.user_id = t.user_id AND acc.id = t.account_id
        LEFT JOIN categories AS c
            ON c.user_id = t.user_id AND c.id = t.category_id AND c.deleted_at IS NULL
        LEFT JOIN merchants AS m
            ON m.user_id = t.user_id AND m.id = t.merchant_id AND m.deleted_at IS NULL
        WHERE t.user_id = $1
          AND t.deleted_at IS NULL
          AND acc.deleted_at IS NULL
          AND ($2::uuid IS NULL OR t.account_id = $2)
          AND ($3::date IS NULL OR t.booking_date >= $3)
          AND ($4::date IS NULL OR t.booking_date <= $4)
          AND ($5::date IS NULL OR (t.booking_date, t.id) < ($5::date, $6::uuid))
        ORDER BY t.booking_date DESC, t.id DESC
        LIMIT $7
        "#,
        user_id,
        filter.account_id,
        filter.from,
        filter.to,
        after_date,
        after_id,
        limit,
    )
    .fetch_all(pool)
    .await?;

    let next = if records.len() > PAGE_SIZE {
        records.truncate(PAGE_SIZE);
        records
            .last()
            .map(|record| (record.booking_date, record.id))
    } else {
        None
    };

    Ok(TransactionListPage { records, next })
}

/// Encode a keyset [`Cursor`] as an opaque `"YYYY-MM-DD_<uuid>"` string for the
/// browser to hand back verbatim.
pub fn encode_cursor((booking_date, id): Cursor) -> String {
    format!("{booking_date}_{id}")
}

/// Decode a [`Cursor`] produced by [`encode_cursor`]. Any malformed value is
/// rejected rather than trusted.
pub fn parse_cursor(input: &str) -> Result<Cursor, TransactionError> {
    let invalid = || TransactionError::InvalidInput("invalid page cursor");
    let (date, id) = input.split_once('_').ok_or_else(invalid)?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| invalid())?;
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok((date, id))
}

/// Parse and validate a user-supplied `amount` string into an exact decimal.
///
/// Mirrors the `transactions_amount_nonzero` CHECK and the `NUMERIC(38, 18)`
/// column type: the value must be a number, must not be zero, and must fit the
/// column's precision. The sign is preserved exactly as written; nothing is
/// rounded.
pub fn validate_amount(input: &str) -> Result<BigDecimal, TransactionError> {
    let amount = BigDecimal::from_str(input.trim())
        .map_err(|_| TransactionError::InvalidInput("amount must be a number"))?;

    if amount.is_zero() {
        return Err(TransactionError::InvalidInput("amount must not be zero"));
    }

    // `scale` is the count of fractional digits (negative when the value is a
    // multiple of a power of ten); `digits` is the count of significant digits.
    let normalized = amount.normalized();
    let (_, scale) = normalized.as_bigint_and_exponent();
    if scale > AMOUNT_SCALE {
        return Err(TransactionError::InvalidInput(
            "amount has too many decimal places",
        ));
    }
    let integer_digits = i128::from(normalized.digits()) - i128::from(scale);
    if integer_digits > i128::from(AMOUNT_INTEGER_DIGITS) {
        return Err(TransactionError::InvalidInput("amount is too large"));
    }

    Ok(amount)
}

/// Parse a required ISO `YYYY-MM-DD` date (the shape `<input type="date">`
/// submits).
pub fn validate_booking_date(input: &str) -> Result<NaiveDate, TransactionError> {
    NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d")
        .map_err(|_| TransactionError::InvalidInput("booking date is invalid"))
}

/// Parse an optional ISO `YYYY-MM-DD` value date, treating a blank or absent
/// field as "no value date" (mirrors the nullable `value_date` column).
pub fn validate_value_date(input: Option<&str>) -> Result<Option<NaiveDate>, TransactionError> {
    match input.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| TransactionError::InvalidInput("value date is invalid")),
    }
}

/// Parse the `from` / `to` bounds of a date-range filter. A blank or absent
/// bound is "unbounded on that side"; both bounds are inclusive. Rejects a
/// range whose start is after its end.
pub fn validate_date_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>), TransactionError> {
    fn parse(value: &str) -> Result<NaiveDate, TransactionError> {
        NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
            .map_err(|_| TransactionError::InvalidInput("a filter date is invalid"))
    }

    let from = from
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse)
        .transpose()?;
    let to = to
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse)
        .transpose()?;

    if let (Some(start), Some(end)) = (from, to) {
        if start > end {
            return Err(TransactionError::InvalidInput(
                "the start date is after the end date",
            ));
        }
    }

    Ok((from, to))
}

/// Confirm `asset_id` is an active, non-deleted fiat currency (one that
/// `list_currencies` would return), not merely any row the foreign key allows.
/// Mirrors `crate::server::accounts::assert_currency_exists`.
async fn assert_currency_exists(pool: &PgPool, asset_id: Uuid) -> Result<(), TransactionError> {
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
        return Err(TransactionError::UnknownAsset);
    }

    Ok(())
}

/// Reject a transaction that is one leg of a (non-deleted) transfer: a plain
/// edit or delete of a single leg would leave the transfer half-broken, so the
/// caller is told to act on the transfer instead.
async fn assert_not_transfer_leg(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), TransactionError> {
    let leg = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM transfers
        WHERE user_id = $1
          AND (source_transaction_id = $2 OR destination_transaction_id = $2)
          AND deleted_at IS NULL
        "#,
        user_id,
        id,
    )
    .fetch_optional(pool)
    .await?;

    if leg.is_some() {
        return Err(TransactionError::PartOfTransfer);
    }

    Ok(())
}

/// Insert a transaction on one of the user's accounts. `category_id` and
/// `merchant_id` are left `NULL`.
///
/// Returns [`TransactionError::NotFound`] if `account_id` is not one of this
/// user's non-deleted accounts, and [`TransactionError::UnknownAsset`] if
/// `asset_id` is not a valid currency.
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    asset_id: Uuid,
    amount: &BigDecimal,
    booking_date: NaiveDate,
    value_date: Option<NaiveDate>,
) -> Result<Uuid, TransactionError> {
    let account = sqlx::query_scalar!(
        r#"
        SELECT id
        FROM accounts
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        account_id,
        user_id,
    )
    .fetch_optional(pool)
    .await?;

    if account.is_none() {
        return Err(TransactionError::NotFound);
    }

    assert_currency_exists(pool, asset_id).await?;

    let row = match sqlx::query!(
        r#"
        INSERT INTO transactions (user_id, account_id, asset_id, amount, booking_date, value_date)
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
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(TransactionError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(row.id)
}

/// Update the user's transaction: its amount, currency, and dates. The account
/// and any category/merchant are left unchanged.
///
/// Returns [`TransactionError::NotFound`] if the id is not one of this user's
/// non-deleted transactions, [`TransactionError::PartOfTransfer`] if it is a
/// transfer leg, and [`TransactionError::UnknownAsset`] if `asset_id` is not a
/// valid currency.
pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    asset_id: Uuid,
    amount: &BigDecimal,
    booking_date: NaiveDate,
    value_date: Option<NaiveDate>,
) -> Result<(), TransactionError> {
    assert_not_transfer_leg(pool, user_id, id).await?;
    assert_currency_exists(pool, asset_id).await?;

    let row = match sqlx::query!(
        r#"
        UPDATE transactions
        SET asset_id = $3,
            amount = $4,
            booking_date = $5,
            value_date = $6,
            updated_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        RETURNING id
        "#,
        id,
        user_id,
        asset_id,
        amount,
        booking_date,
        value_date,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(TransactionError::UnknownAsset);
        }
        Err(err) => return Err(err.into()),
    };

    if row.is_none() {
        return Err(TransactionError::NotFound);
    }

    Ok(())
}

/// Soft-delete the user's transaction by setting `deleted_at`. It then drops
/// out of the `account_balances` view like any other non-existent row.
///
/// Returns [`TransactionError::NotFound`] if the id is not one of this user's
/// non-deleted transactions, and [`TransactionError::PartOfTransfer`] if it is
/// a transfer leg.
pub async fn soft_delete(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), TransactionError> {
    assert_not_transfer_leg(pool, user_id, id).await?;

    let result = sqlx::query!(
        r#"
        UPDATE transactions
        SET deleted_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(TransactionError::NotFound);
    }

    Ok(())
}

/// Pure-input validation tests.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_amount_accepts_a_negative_value() {
        assert_eq!(
            validate_amount(" -25.50 ").expect("valid"),
            BigDecimal::from_str("-25.50").unwrap()
        );
    }

    #[test]
    fn validate_amount_rejects_zero() {
        assert!(validate_amount("0").is_err());
        assert!(validate_amount("0.00").is_err());
        assert!(validate_amount("-0.0").is_err());
    }

    #[test]
    fn validate_amount_rejects_non_numeric() {
        assert!(validate_amount("").is_err());
        assert!(validate_amount("abc").is_err());
        assert!(validate_amount("1,00").is_err());
    }

    #[test]
    fn validate_amount_rejects_too_many_decimal_places() {
        // 18 fractional digits is the limit.
        assert!(validate_amount("0.123456789012345678").is_ok());
        assert!(validate_amount("0.1234567890123456789").is_err());
    }

    #[test]
    fn validate_amount_rejects_a_value_wider_than_the_column() {
        // 20 integer digits is the limit.
        assert!(validate_amount("99999999999999999999").is_ok());
        assert!(validate_amount("100000000000000000000").is_err());
    }

    #[test]
    fn validate_value_date_treats_blank_as_absent() {
        assert_eq!(validate_value_date(None).expect("valid"), None);
        assert_eq!(validate_value_date(Some("   ")).expect("valid"), None);
        assert_eq!(
            validate_value_date(Some("2026-01-15")).expect("valid"),
            Some(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
        );
        assert!(validate_value_date(Some("15/01/2026")).is_err());
    }

    #[test]
    fn validate_date_range_accepts_open_and_closed_ranges() {
        assert_eq!(
            validate_date_range(None, None).expect("valid"),
            (None, None)
        );
        assert_eq!(
            validate_date_range(Some("  "), Some("2026-01-31")).expect("valid"),
            (None, Some(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()))
        );
        let (from, to) =
            validate_date_range(Some("2026-01-01"), Some("2026-01-01")).expect("valid");
        assert_eq!(from, to);
    }

    #[test]
    fn validate_date_range_rejects_a_backwards_range_and_bad_dates() {
        assert!(validate_date_range(Some("2026-02-01"), Some("2026-01-01")).is_err());
        assert!(validate_date_range(Some("nonsense"), None).is_err());
    }

    #[test]
    fn cursor_round_trips_and_rejects_garbage() {
        let id = Uuid::parse_str("0193c0f0-1234-7abc-8def-0123456789ab").unwrap();
        let cursor = (NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(), id);
        assert_eq!(parse_cursor(&encode_cursor(cursor)).expect("valid"), cursor);
        assert!(parse_cursor("not-a-cursor").is_err());
        assert!(parse_cursor("2026-01-15_not-a-uuid").is_err());
        assert!(parse_cursor("15-01-2026_00000000-0000-0000-0000-000000000000").is_err());
    }
}

/// Authorization: every query here is scoped by `user_id`. `list` returns only
/// the caller's rows (an `account_id` filter that names someone else's account
/// just matches nothing); `create` / `update` / `soft_delete` report `NotFound`
/// for a row or account that is not the caller's, indistinguishable from a
/// missing one. Each `#[sqlx::test]` runs against its own freshly migrated
/// database.
///
/// Cross-user category or merchant names cannot leak through the two
/// `LEFT JOIN`s: `transactions` carries composite foreign keys to
/// `categories (user_id, id)` and `merchants (user_id, id)`, so a row pointing
/// at another user's category cannot be inserted in the first place.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::accounts;
    use crate::server::test_support::{
        create_user, currency_id, date, dec, insert_transaction, insert_transfer,
    };

    /// The caller's transactions on one account, newest first (first page).
    async fn account_rows(
        pool: &PgPool,
        user_id: Uuid,
        account_id: Uuid,
    ) -> Vec<TransactionRecord> {
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

    #[sqlx::test]
    async fn list_returns_nothing_for_another_users_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, account.id, eur, "-25.50", date(2026, 1, 16)).await;

        let denied = account_rows(&pool, bob, account.id).await;
        assert!(denied.is_empty());

        // Control: the owner sees both rows, so the account really has data.
        let owned = account_rows(&pool, alice, account.id).await;
        assert_eq!(owned.len(), 2);
    }

    #[sqlx::test]
    async fn list_does_not_leak_the_callers_own_rows_for_a_foreign_account(pool: PgPool) {
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

        // Bob filtering by Alice's account must get nothing at all -- in
        // particular not his own rows, which is what a dropped `user_id`
        // predicate would return.
        assert!(account_rows(&pool, bob, alices.id).await.is_empty());

        // Control: Bob's own account still returns his row.
        let owned = account_rows(&pool, bob, bobs.id).await;
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, bobs_transaction);
    }

    #[sqlx::test]
    async fn list_is_scoped_to_the_caller_across_all_accounts(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let alices = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("bob's account");

        insert_transaction(&pool, alice, alices.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, alices.id, eur, "20.00", date(2026, 1, 16)).await;
        insert_transaction(&pool, bob, bobs.id, eur, "99.00", date(2026, 1, 17)).await;

        let page = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(page.records.len(), 2);
        assert!(page
            .records
            .iter()
            .all(|record| record.account_id == alices.id));
        // Newest first.
        assert_eq!(page.records[0].booking_date, date(2026, 1, 16));
        assert_eq!(page.next, None);
    }

    #[sqlx::test]
    async fn list_filters_by_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let cash = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("cash account");
        let bank = accounts::create(&pool, alice, "Bank", "bank", eur)
            .await
            .expect("bank account");

        insert_transaction(&pool, alice, cash.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transaction(&pool, alice, bank.id, eur, "20.00", date(2026, 1, 15)).await;

        let page = list(
            &pool,
            alice,
            TransactionFilter {
                account_id: Some(bank.id),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].account_id, bank.id);
        assert_eq!(page.records[0].account_name, "Bank");
    }

    #[sqlx::test]
    async fn list_date_range_is_inclusive_on_both_ends(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("account");

        for day in [10, 15, 20, 25] {
            insert_transaction(&pool, alice, account.id, eur, "1.00", date(2026, 1, day)).await;
        }

        let page = list(
            &pool,
            alice,
            TransactionFilter {
                from: Some(date(2026, 1, 15)),
                to: Some(date(2026, 1, 20)),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");

        let days: Vec<_> = page
            .records
            .iter()
            .map(|record| record.booking_date)
            .collect();
        assert_eq!(days, vec![date(2026, 1, 20), date(2026, 1, 15)]);
    }

    #[sqlx::test]
    async fn list_paginates_without_gaps_or_repeats(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Cash", "cash", eur)
            .await
            .expect("account");

        // One more than a page; rows share booking dates so the `id` tiebreak
        // is what keeps the page boundary clean.
        let total = PAGE_SIZE + 1;
        for i in 0..total {
            let day = 1 + (i % 3) as u32;
            insert_transaction(&pool, alice, account.id, eur, "1.00", date(2026, 1, day)).await;
        }

        let first = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(first.records.len(), PAGE_SIZE);
        let cursor = first.next.expect("a second page");

        let second = list(
            &pool,
            alice,
            TransactionFilter {
                after: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .expect("list runs");
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.next, None);

        let seam: Vec<_> = first
            .records
            .iter()
            .chain(&second.records)
            .map(|record| (record.booking_date, record.id))
            .collect();

        // Strictly descending by (booking_date, id) across the page boundary…
        let mut ordered = seam.clone();
        ordered.sort_by(|a, b| b.cmp(a));
        assert_eq!(seam, ordered);

        // …and every row exactly once.
        let mut ids: Vec<_> = seam.iter().map(|(_, id)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total);
    }

    #[sqlx::test]
    async fn list_excludes_soft_deleted_transactions_and_accounts(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let live = accounts::create(&pool, alice, "Live", "cash", eur)
            .await
            .expect("live account");
        let gone = accounts::create(&pool, alice, "Gone", "cash", eur)
            .await
            .expect("gone account");

        let kept = insert_transaction(&pool, alice, live.id, eur, "1.00", date(2026, 1, 15)).await;
        let deleted_txn =
            insert_transaction(&pool, alice, live.id, eur, "2.00", date(2026, 1, 16)).await;
        insert_transaction(&pool, alice, gone.id, eur, "3.00", date(2026, 1, 17)).await;

        soft_delete(&pool, alice, deleted_txn)
            .await
            .expect("delete transaction");
        accounts::soft_delete(&pool, alice, gone.id)
            .await
            .expect("delete account");

        let page = list(&pool, alice, TransactionFilter::default())
            .await
            .expect("list runs");
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, kept);
    }

    #[sqlx::test]
    async fn create_assigns_the_calling_users_id_and_rejects_a_foreign_account(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        create(
            &pool,
            alice,
            account.id,
            eur,
            &dec("10.00"),
            date(2026, 1, 15),
            None,
        )
        .await
        .expect("alice records a transaction");

        // Bob cannot record a transaction on Alice's account.
        let denied = create(
            &pool,
            bob,
            account.id,
            eur,
            &dec("10.00"),
            date(2026, 1, 15),
            None,
        )
        .await;
        assert!(matches!(denied, Err(TransactionError::NotFound)));

        // Only Alice's row exists.
        assert!(account_rows(&pool, bob, account.id).await.is_empty());
        assert_eq!(account_rows(&pool, alice, account.id).await.len(), 1);
    }

    #[sqlx::test]
    async fn create_stores_the_value_date_only_when_given(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        create(
            &pool,
            alice,
            account.id,
            eur,
            &dec("10.00"),
            date(2026, 1, 15),
            None,
        )
        .await
        .expect("no value date");
        create(
            &pool,
            alice,
            account.id,
            eur,
            &dec("20.00"),
            date(2026, 1, 16),
            Some(date(2026, 1, 18)),
        )
        .await
        .expect("with value date");

        let rows = account_rows(&pool, alice, account.id).await;
        // Newest first: the 16th (with a value date), then the 15th (without).
        assert_eq!(rows[0].value_date, Some(date(2026, 1, 18)));
        assert_eq!(rows[1].value_date, None);
    }

    #[sqlx::test]
    async fn update_denies_another_users_transaction_and_leaves_it_unchanged(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let usd = currency_id(&pool, "USD").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        let denied = update(
            &pool,
            bob,
            transaction,
            usd,
            &dec("-999.00"),
            date(2026, 2, 1),
            None,
        )
        .await;
        assert!(matches!(denied, Err(TransactionError::NotFound)));

        let after = account_rows(&pool, alice, account.id).await;
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].amount, dec("100.00"));
        assert_eq!(after[0].asset_id, eur);
        assert_eq!(after[0].booking_date, date(2026, 1, 15));
    }

    #[sqlx::test]
    async fn soft_delete_denies_another_users_transaction(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;

        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        assert!(matches!(
            soft_delete(&pool, bob, transaction).await,
            Err(TransactionError::NotFound)
        ));
        assert_eq!(account_rows(&pool, alice, account.id).await.len(), 1);

        // The owner deletes it; a second delete is not a fresh success.
        soft_delete(&pool, alice, transaction)
            .await
            .expect("owner deletes it");
        assert!(matches!(
            soft_delete(&pool, alice, transaction).await,
            Err(TransactionError::NotFound)
        ));
    }

    #[sqlx::test]
    async fn soft_deleted_transaction_drops_out_of_account_balances(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");
        let transaction =
            insert_transaction(&pool, alice, account.id, eur, "100.00", date(2026, 1, 15)).await;

        let balance: Option<BigDecimal> =
            sqlx::query_scalar("SELECT balance FROM account_balances WHERE account_id = $1")
                .bind(account.id)
                .fetch_optional(&pool)
                .await
                .expect("query runs");
        assert_eq!(balance, Some(dec("100.00")));

        soft_delete(&pool, alice, transaction)
            .await
            .expect("owner deletes it");

        let balance: Option<BigDecimal> =
            sqlx::query_scalar("SELECT balance FROM account_balances WHERE account_id = $1")
                .bind(account.id)
                .fetch_optional(&pool)
                .await
                .expect("query runs");
        assert_eq!(balance, None);
    }

    #[sqlx::test]
    async fn update_and_soft_delete_refuse_a_transfer_leg(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("alice's account");

        let source =
            insert_transaction(&pool, alice, account.id, eur, "-10.00", date(2026, 1, 15)).await;
        let destination =
            insert_transaction(&pool, alice, account.id, eur, "10.00", date(2026, 1, 15)).await;
        insert_transfer(&pool, alice, source, destination).await;

        assert!(matches!(
            update(
                &pool,
                alice,
                source,
                eur,
                &dec("-5.00"),
                date(2026, 1, 15),
                None
            )
            .await,
            Err(TransactionError::PartOfTransfer)
        ));
        assert!(matches!(
            soft_delete(&pool, alice, destination).await,
            Err(TransactionError::PartOfTransfer)
        ));

        // The legs are untouched.
        assert_eq!(account_rows(&pool, alice, account.id).await.len(), 2);
    }
}
