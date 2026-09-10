//! The server function the browser calls to read one account's balance.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::balances::types::AccountBalanceDto;

/// The signed-in user's balance for one account, valued in that account's
/// default currency.
///
/// Fails with a user-safe message when a currency the account holds cannot be
/// valued for lack of an exchange rate.
#[server]
pub async fn account_balance(account_id: String) -> Result<AccountBalanceDto, ServerFnError> {
    use std::sync::Arc;

    use sqlx::types::Uuid;

    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::balances::{self, BalanceError};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(BalanceError::Unauthorized)?;

    let account_id = Uuid::parse_str(&account_id)
        .map_err(|_| BalanceError::InvalidInput("invalid account id"))?;

    let total = balances::account_total(&pool, &cache, user.user_id, account_id).await?;

    Ok(AccountBalanceDto {
        amount: total.amount.to_string(),
        currency_code: total.alphabetic_code,
    })
}
