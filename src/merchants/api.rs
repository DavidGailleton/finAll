//! Server functions the browser calls to list and manage the signed-in user's
//! merchants.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::merchants::types::MerchantDto;

/// List the current user's active, non-deleted merchants, ordered by name.
#[server]
pub async fn list_merchants() -> Result<Vec<MerchantDto>, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::merchants::{self, MerchantError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(MerchantError::Unauthorized)?;

    let records = merchants::list_active_for_user(&pool, user.user_id).await?;

    let dtos = records
        .into_iter()
        .map(|record| MerchantDto {
            id: record.id.to_string(),
            merchant_name: record.merchant_name,
            default_category_id: record.default_category_id.map(|id| id.to_string()),
        })
        .collect();

    Ok(dtos)
}

/// Create a new merchant for the current user. A blank `default_category_id`
/// means "no default category".
#[server]
pub async fn create_merchant(
    merchant_name: String,
    default_category_id: String,
) -> Result<MerchantDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::merchants::{self, MerchantError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(MerchantError::Unauthorized)?;

    let merchant_name = merchants::validate_name(&merchant_name)?;
    let default_category_id = match default_category_id.trim() {
        "" => None,
        id => Some(
            Uuid::parse_str(id).map_err(|_| MerchantError::InvalidInput("invalid category id"))?,
        ),
    };

    let record =
        merchants::create(&pool, user.user_id, &merchant_name, default_category_id).await?;

    Ok(MerchantDto {
        id: record.id.to_string(),
        merchant_name: record.merchant_name,
        default_category_id: record.default_category_id.map(|id| id.to_string()),
    })
}

/// Update the current user's merchant: its name and default category. A blank
/// `default_category_id` clears the default category.
#[server]
pub async fn update_merchant(
    id: String,
    merchant_name: String,
    default_category_id: String,
) -> Result<MerchantDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::merchants::{self, MerchantError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(MerchantError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| MerchantError::InvalidInput("invalid merchant id"))?;
    let merchant_name = merchants::validate_name(&merchant_name)?;
    let default_category_id = match default_category_id.trim() {
        "" => None,
        id => Some(
            Uuid::parse_str(id).map_err(|_| MerchantError::InvalidInput("invalid category id"))?,
        ),
    };

    let record =
        merchants::update(&pool, user.user_id, id, &merchant_name, default_category_id).await?;

    Ok(MerchantDto {
        id: record.id.to_string(),
        merchant_name: record.merchant_name,
        default_category_id: record.default_category_id.map(|id| id.to_string()),
    })
}

/// Soft-delete the current user's merchant. Rows in other tables that reference
/// it are left untouched.
#[server]
pub async fn delete_merchant(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::merchants::{self, MerchantError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(MerchantError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| MerchantError::InvalidInput("invalid merchant id"))?;

    merchants::soft_delete(&pool, user.user_id, id).await?;

    Ok(())
}
