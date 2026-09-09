//! Server functions the browser calls to list and manage the signed-in user's
//! categories.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside
//! the function bodies so this module still compiles for the browser target,
//! where these become network calls.

use leptos::prelude::*;

use crate::categories::types::{CategoryDto, CategoryKind};

/// List the current user's active, non-deleted categories, ordered by kind then
/// name.
#[server]
pub async fn list_categories() -> Result<Vec<CategoryDto>, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::categories::{self, CategoryError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(CategoryError::Unauthorized)?;

    let records = categories::list_active_for_user(&pool, user.user_id).await?;

    let dtos = records
        .into_iter()
        .map(|record| {
            Ok(CategoryDto {
                id: record.id.to_string(),
                category_name: record.category_name,
                kind: CategoryKind::from_db_str(&record.kind).ok_or(CategoryError::Internal)?,
            })
        })
        .collect::<Result<Vec<CategoryDto>, CategoryError>>()?;

    Ok(dtos)
}

/// Create a new category for the current user.
#[server]
pub async fn create_category(
    category_name: String,
    kind: CategoryKind,
) -> Result<CategoryDto, ServerFnError> {
    use crate::server::auth::extract;
    use crate::server::categories::{self, CategoryError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(CategoryError::Unauthorized)?;

    let category_name = categories::validate_name(&category_name)?;

    let record = categories::create(&pool, user.user_id, &category_name, kind.as_db_str()).await?;

    Ok(CategoryDto {
        id: record.id.to_string(),
        category_name: record.category_name,
        kind,
    })
}

/// Rename the current user's category. Its kind is fixed at creation and is not
/// changed here.
#[server]
pub async fn update_category(
    id: String,
    category_name: String,
) -> Result<CategoryDto, ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::categories::{self, CategoryError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(CategoryError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| CategoryError::InvalidInput("invalid category id"))?;
    let category_name = categories::validate_name(&category_name)?;

    let record = categories::update(&pool, user.user_id, id, &category_name).await?;

    Ok(CategoryDto {
        id: record.id.to_string(),
        category_name: record.category_name,
        kind: CategoryKind::from_db_str(&record.kind).ok_or(CategoryError::Internal)?,
    })
}

/// Soft-delete the current user's category. Rows in other tables that reference
/// it are left untouched.
#[server]
pub async fn delete_category(id: String) -> Result<(), ServerFnError> {
    use sqlx::types::Uuid;

    use crate::server::auth::extract;
    use crate::server::categories::{self, CategoryError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(CategoryError::Unauthorized)?;

    let id =
        Uuid::parse_str(&id).map_err(|_| CategoryError::InvalidInput("invalid category id"))?;

    categories::soft_delete(&pool, user.user_id, id).await?;

    Ok(())
}
