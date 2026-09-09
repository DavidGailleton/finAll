//! Fixtures shared by the database-backed tests in this module tree.
//!
//! Compiled only under `cfg(test)`, so nothing here reaches a build. The
//! queries are the runtime-checked `sqlx::query` form rather than the `query!`
//! macros on purpose: `cargo sqlx prepare` runs `cargo check`, which does not
//! compile test code, so a macro query here would have no entry in `.sqlx/` and
//! would fail to build under `SQLX_OFFLINE=true`.
//!
//! Fixtures never read the system clock; callers pass explicit dates.

use std::str::FromStr;

use sqlx::types::chrono::NaiveDate;
use sqlx::types::{BigDecimal, Uuid};
use sqlx::PgPool;

/// Stand-in for the `users.password_hash` column, which only has to be
/// non-blank. These tests never authenticate, so hashing a password with
/// Argon2id (deliberately slow) would buy nothing.
const PLACEHOLDER_PASSWORD_HASH: &str = "not-a-real-password-hash";

/// Parse an exact decimal literal. Mirrors the helper the pure tests use.
pub fn dec(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).expect("valid decimal literal")
}

/// A fixed date, so no test depends on the system clock.
pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("valid calendar date")
}

/// Insert a user and return its id.
pub async fn create_user(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
        .bind(email)
        .bind(PLACEHOLDER_PASSWORD_HASH)
        .fetch_one(pool)
        .await
        .expect("insert test user")
}

/// The id of a seeded fiat currency, by ISO-4217 alphabetic code. The currency
/// rows come from the `seed_iso4217_currencies` migration, which
/// `#[sqlx::test]` applies to every per-test database.
pub async fn currency_id(pool: &PgPool, code: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM assets WHERE asset_class = 'fiat' AND code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("seeded currency")
}

/// Insert a transaction on an account. `amount` is an exact decimal literal;
/// it never passes through a float.
pub async fn insert_transaction(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    asset_id: Uuid,
    amount: &str,
    booking_date: NaiveDate,
) -> Uuid {
    sqlx::query_scalar(
        r#"
        INSERT INTO transactions (user_id, account_id, asset_id, amount, booking_date)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(account_id)
    .bind(asset_id)
    .bind(dec(amount))
    .bind(booking_date)
    .fetch_one(pool)
    .await
    .expect("insert test transaction")
}

/// Link two existing transactions as the two legs of a transfer and return the
/// transfer id.
pub async fn insert_transfer(
    pool: &PgPool,
    user_id: Uuid,
    source_transaction_id: Uuid,
    destination_transaction_id: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        r#"
        INSERT INTO transfers (user_id, source_transaction_id, destination_transaction_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(source_transaction_id)
    .bind(destination_transaction_id)
    .fetch_one(pool)
    .await
    .expect("insert test transfer")
}
