//! Seeds a demo user and illustrative example data (a few accounts,
//! categories, merchants, and transactions) so the dashboards, ledger, and
//! reports have something to show without any manual setup.
//!
//! [`seed_demo_account`] is called from `main.rs` **only** when
//! `leptos_options.env == Env::DEV` — this module does not check that itself,
//! so it must never be wired to a path reachable in production. It is
//! idempotent: a no-op once the demo user exists, so restarting the dev stack
//! never duplicates data.
//!
//! Every write goes through the same domain functions the app itself uses
//! (`accounts::create`, `categories::create`, …), so all of their existing
//! validation and uniqueness rules apply unchanged.

use std::str::FromStr;

use bigdecimal::BigDecimal;
use leptos::logging;
use sqlx::types::chrono::{NaiveDate, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::accounts::{self, AccountError};
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::auth::{password, user};
use crate::server::categories::{self, CategoryError};
use crate::server::error::AuthError;
use crate::server::merchants::{self, MerchantError};
use crate::server::transactions::{self, TransactionError, TransactionWrite};
use crate::server::transfers::{self, TransferError};

/// Fixed, dev-only login — documented in `.env.example`. Never used for a real
/// account; `seed_demo_account` refuses to run more than once (see below), and
/// it is only ever called when `leptos_options.env == Env::DEV`.
pub const DEMO_EMAIL: &str = "demo@example.test";
pub const DEMO_PASSWORD: &str = "demo-password-123";

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("something went wrong seeding the demo account")]
    Internal,
}

impl From<sqlx::Error> for SeedError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("seed: database error: {err}");
        SeedError::Internal
    }
}

impl From<AuthError> for SeedError {
    fn from(err: AuthError) -> Self {
        logging::error!("seed: auth error: {err}");
        SeedError::Internal
    }
}

impl From<AccountError> for SeedError {
    fn from(err: AccountError) -> Self {
        logging::error!("seed: account error: {err}");
        SeedError::Internal
    }
}

impl From<CategoryError> for SeedError {
    fn from(err: CategoryError) -> Self {
        logging::error!("seed: category error: {err}");
        SeedError::Internal
    }
}

impl From<MerchantError> for SeedError {
    fn from(err: MerchantError) -> Self {
        logging::error!("seed: merchant error: {err}");
        SeedError::Internal
    }
}

impl From<TransactionError> for SeedError {
    fn from(err: TransactionError) -> Self {
        logging::error!("seed: transaction error: {err}");
        SeedError::Internal
    }
}

impl From<TransferError> for SeedError {
    fn from(err: TransferError) -> Self {
        logging::error!("seed: transfer error: {err}");
        SeedError::Internal
    }
}

/// Create the demo user and its example data. A no-op (`Ok(())`, logged) if
/// the demo user already exists.
pub async fn seed_demo_account(pool: &PgPool, cache: &FxRateCache) -> Result<(), SeedError> {
    if user::find_active_by_email(pool, DEMO_EMAIL)
        .await?
        .is_some()
    {
        logging::log!("seed: demo account already exists, skipping");
        return Ok(());
    }

    let password_hash = password::hash_password(DEMO_PASSWORD).await?;
    let user_id = user::create(pool, DEMO_EMAIL, &password_hash, Some("Demo User")).await?;

    let eur = currency_id(pool, "EUR").await?;
    let usd = currency_id(pool, "USD").await?;

    let checking = accounts::create(pool, user_id, "Everyday Checking", "bank", eur).await?;
    let cash = accounts::create(pool, user_id, "Cash Wallet", "cash", eur).await?;
    // Exercises the on-demand FX conversion: a foreign-currency transaction on
    // this account is translated at its booking-date rate (or left "pending"
    // without network access — the write is never blocked either way).
    let travel = accounts::create(pool, user_id, "Travel Card", "bank", usd).await?;

    let salary = categories::create(pool, user_id, "Salary", "income").await?;
    let groceries = categories::create(pool, user_id, "Groceries", "expense").await?;
    let rent = categories::create(pool, user_id, "Rent", "expense").await?;
    let dining = categories::create(pool, user_id, "Dining out", "expense").await?;
    let transport = categories::create(pool, user_id, "Transport", "expense").await?;

    let employer = merchants::create(pool, user_id, "Northwind Employer", Some(salary.id)).await?;
    let market = merchants::create(pool, user_id, "Green Grocer", Some(groceries.id)).await?;
    let landlord = merchants::create(pool, user_id, "Riverside Landlord", Some(rent.id)).await?;
    let cafe = merchants::create(pool, user_id, "Corner Cafe", Some(dining.id)).await?;
    let cab = merchants::create(pool, user_id, "City Cabs", Some(transport.id)).await?;

    let today = Utc::now().date_naive();

    // A couple of months of ordinary, single-currency activity on the
    // checking account: (amount, days before today, category, merchant).
    let checking_rows = [
        ("2400.00", 58, salary.id, employer.id),
        ("2400.00", 28, salary.id, employer.id),
        ("-72.40", 50, groceries.id, market.id),
        ("-64.10", 36, groceries.id, market.id),
        ("-58.90", 22, groceries.id, market.id),
        ("-950.00", 55, rent.id, landlord.id),
        ("-950.00", 25, rent.id, landlord.id),
        ("-18.50", 12, dining.id, cafe.id),
    ];
    for (amount, days_ago, category_id, merchant_id) in checking_rows {
        transactions::create(
            pool,
            cache,
            user_id,
            checking.id,
            &TransactionWrite {
                asset_id: eur,
                amount: dec(amount)?,
                booking_date: days_before(today, days_ago),
                value_date: None,
                category_id: Some(category_id),
                merchant_id: Some(merchant_id),
            },
        )
        .await?;
    }

    // A little cash spending, and a EUR purchase on the (USD) travel card —
    // foreign on that account, so it exercises the on-demand FX conversion.
    transactions::create(
        pool,
        cache,
        user_id,
        cash.id,
        &TransactionWrite {
            asset_id: eur,
            amount: dec("-15.00")?,
            booking_date: days_before(today, 20),
            value_date: None,
            category_id: Some(transport.id),
            merchant_id: Some(cab.id),
        },
    )
    .await?;
    transactions::create(
        pool,
        cache,
        user_id,
        travel.id,
        &TransactionWrite {
            asset_id: eur,
            amount: dec("-42.00")?,
            booking_date: days_before(today, 8),
            value_date: None,
            category_id: Some(dining.id),
            merchant_id: Some(cafe.id),
        },
    )
    .await?;

    // A transfer from checking to the cash wallet.
    let source = transactions::create(
        pool,
        cache,
        user_id,
        checking.id,
        &TransactionWrite {
            asset_id: eur,
            amount: dec("-100.00")?,
            booking_date: days_before(today, 15),
            value_date: None,
            category_id: None,
            merchant_id: None,
        },
    )
    .await?;
    let destination = transactions::create(
        pool,
        cache,
        user_id,
        cash.id,
        &TransactionWrite {
            asset_id: eur,
            amount: dec("100.00")?,
            booking_date: days_before(today, 15),
            value_date: None,
            category_id: None,
            merchant_id: None,
        },
    )
    .await?;
    transfers::link(pool, user_id, source, destination).await?;

    logging::log!("seed: demo account created ({DEMO_EMAIL}; see .env.example for the password)");
    Ok(())
}

/// The id of an active, non-deleted fiat currency by its alphabetic code.
async fn currency_id(pool: &PgPool, code: &str) -> Result<Uuid, SeedError> {
    sqlx::query_scalar!(
        r#"
        SELECT id
        FROM assets
        WHERE asset_class = 'fiat' AND code = $1 AND deleted_at IS NULL
        "#,
        code,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        logging::error!("seed: currency {code} not found");
        SeedError::Internal
    })
}

/// Parse one of this module's own decimal literals.
fn dec(value: &str) -> Result<BigDecimal, SeedError> {
    BigDecimal::from_str(value).map_err(|_| {
        logging::error!("seed: invalid decimal literal {value:?} in seed data");
        SeedError::Internal
    })
}

/// `n` days before `date`. Built from `NaiveDate::pred_opt` alone: this crate
/// has no direct `chrono` dependency and `sqlx::types::chrono` does not
/// re-export `Datelike`/`Duration` (see `assets::schedule`). `n` is always
/// small here, so the loop is negligible.
fn days_before(date: NaiveDate, n: u32) -> NaiveDate {
    let mut day = date;
    for _ in 0..n {
        day = day.pred_opt().unwrap_or(day);
    }
    day
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::assets::fx_cache::FxRateCache;

    #[sqlx::test]
    async fn seeding_twice_only_creates_the_demo_account_once(pool: PgPool) {
        let cache = FxRateCache::new().expect("build fx cache");

        seed_demo_account(&pool, &cache)
            .await
            .expect("first seed run");
        let after_first = user::find_active_by_email(&pool, DEMO_EMAIL)
            .await
            .expect("lookup")
            .expect("demo user exists");

        seed_demo_account(&pool, &cache)
            .await
            .expect("second seed run is a no-op");
        let after_second = user::find_active_by_email(&pool, DEMO_EMAIL)
            .await
            .expect("lookup")
            .expect("demo user still exists");

        assert_eq!(after_first.id, after_second.id);

        let account_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM accounts WHERE user_id = $1 AND deleted_at IS NULL",
        )
        .bind(after_first.id)
        .fetch_one(&pool)
        .await
        .expect("count accounts");
        assert_eq!(account_count, 3);
    }
}
