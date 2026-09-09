//! Queries for the signed-in user's categories (`categories` table), the domain
//! error, and the input validation for the category server functions.
//!
//! Every value arriving from a server function is untrusted. `validate_name` is
//! the authoritative check for `category_name`; the `kind` value is constrained
//! to the [`crate::categories::types::CategoryKind`] variants before it reaches
//! this module. Each query is scoped by `user_id` so one user can never read or
//! change another's categories.

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum CategoryError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("category not found")]
    NotFound,

    #[error("you already have a category with that name")]
    DuplicateName,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for CategoryError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("categories: database error: {err}");
        CategoryError::Internal
    }
}

/// The columns needed to render a category to the browser. `kind` is the raw
/// database string; the server function maps it back to a `CategoryKind`.
pub struct CategoryRecord {
    pub id: Uuid,
    pub category_name: String,
    pub kind: String,
}

/// Trim the category name and require it non-blank (mirrors
/// `categories_name_not_empty`).
pub fn validate_name(input: &str) -> Result<String, CategoryError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(CategoryError::InvalidInput("category name is required"));
    }
    Ok(name.to_owned())
}

/// List the user's active (non-deleted) categories, ordered by kind then name.
pub async fn list_active_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<CategoryRecord>, CategoryError> {
    let records = sqlx::query_as!(
        CategoryRecord,
        r#"
        SELECT id, category_name, kind
        FROM categories
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY kind, category_name
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Insert a new category for the user and return the created record.
///
/// A name the user already uses (`categories_user_id_name_unique`) is mapped to
/// [`CategoryError::DuplicateName`].
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    category_name: &str,
    kind: &str,
) -> Result<CategoryRecord, CategoryError> {
    let row = match sqlx::query!(
        r#"
        INSERT INTO categories (user_id, category_name, kind)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
        user_id,
        category_name,
        kind,
    )
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(CategoryError::DuplicateName);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(CategoryRecord {
        id: row.id,
        category_name: category_name.to_owned(),
        kind: kind.to_owned(),
    })
}

/// Rename the user's category and return the updated record. The `kind` is
/// fixed at creation and is not touched here.
///
/// Returns [`CategoryError::NotFound`] if the id does not match one of this
/// user's non-deleted categories, and [`CategoryError::DuplicateName`] if the
/// new name is already used by another of the user's categories.
pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    category_name: &str,
) -> Result<CategoryRecord, CategoryError> {
    let row = match sqlx::query!(
        r#"
        UPDATE categories
        SET category_name = $3,
            updated_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        RETURNING id, kind
        "#,
        id,
        user_id,
        category_name,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(CategoryError::DuplicateName);
        }
        Err(err) => return Err(err.into()),
    };

    let Some(row) = row else {
        return Err(CategoryError::NotFound);
    };

    Ok(CategoryRecord {
        id: row.id,
        category_name: category_name.to_owned(),
        kind: row.kind,
    })
}

/// Soft-delete the user's category by setting `deleted_at`.
///
/// Returns [`CategoryError::NotFound`] if the id does not match one of this
/// user's non-deleted categories.
pub async fn soft_delete(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), CategoryError> {
    let result = sqlx::query!(
        r#"
        UPDATE categories
        SET deleted_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(CategoryError::NotFound);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_trims() {
        assert_eq!(validate_name("  Groceries  ").unwrap(), "Groceries");
    }

    #[test]
    fn validate_name_rejects_blank() {
        assert!(validate_name("   ").is_err());
        assert!(validate_name("").is_err());
    }
}

/// Authorization: every query here is scoped by `user_id`, so one user must
/// never read or change another's categories. A wrong owner is reported as
/// [`CategoryError::NotFound`], indistinguishable from a missing row.
///
/// Each `#[sqlx::test]` runs against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::test_support::create_user;

    #[sqlx::test]
    async fn list_active_for_user_returns_only_the_owners_categories_ordered(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        create(&pool, alice, "Salary", "income")
            .await
            .expect("alice income");
        create(&pool, alice, "Rent", "expense")
            .await
            .expect("alice expense");
        create(&pool, alice, "Groceries", "expense")
            .await
            .expect("alice expense 2");
        create(&pool, bob, "Salary", "income")
            .await
            .expect("bob income");

        let names: Vec<String> = list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .into_iter()
            .map(|record| record.category_name)
            .collect();
        // Ordered by kind ('expense' < 'income') then name.
        assert_eq!(names, vec!["Groceries", "Rent", "Salary"]);

        let bobs = list_active_for_user(&pool, bob).await.expect("lists");
        assert_eq!(bobs.len(), 1);
        assert_eq!(bobs[0].category_name, "Salary");
    }

    #[sqlx::test]
    async fn create_assigns_the_calling_users_id(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let category = create(&pool, alice, "Salary", "income")
            .await
            .expect("alice's category");

        assert!(list_active_for_user(&pool, bob)
            .await
            .expect("lists")
            .is_empty());
        assert!(matches!(
            update(&pool, bob, category.id, "Stolen").await,
            Err(CategoryError::NotFound)
        ));
    }

    #[sqlx::test]
    async fn create_rejects_a_duplicate_name_but_allows_it_for_another_user(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        create(&pool, alice, "Groceries", "expense")
            .await
            .expect("first");

        // Same user, same name -- even with a different kind.
        assert!(matches!(
            create(&pool, alice, "Groceries", "income").await,
            Err(CategoryError::DuplicateName)
        ));

        // A different user may use the same name.
        create(&pool, bob, "Groceries", "expense")
            .await
            .expect("bob's own");
    }

    #[sqlx::test]
    async fn update_denies_another_users_category_and_rejects_a_colliding_rename(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let groceries = create(&pool, alice, "Groceries", "expense")
            .await
            .expect("groceries");
        create(&pool, alice, "Rent", "expense").await.expect("rent");

        // Not the owner: NotFound, and nothing written.
        assert!(matches!(
            update(&pool, bob, groceries.id, "Hijacked").await,
            Err(CategoryError::NotFound)
        ));

        // Renaming onto an existing name is a duplicate.
        assert!(matches!(
            update(&pool, alice, groceries.id, "Rent").await,
            Err(CategoryError::DuplicateName)
        ));

        // A real rename keeps the kind and takes effect. (Trimming is the
        // caller's job -- `validate_name` -- so this passes an already-clean
        // name, as the server function does.)
        let renamed = update(&pool, alice, groceries.id, "Food")
            .await
            .expect("rename");
        assert_eq!(renamed.category_name, "Food");
        assert_eq!(renamed.kind, "expense");
    }

    #[sqlx::test]
    async fn soft_delete_is_scoped_and_not_repeatable(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let category = create(&pool, alice, "Salary", "income")
            .await
            .expect("category");

        assert!(matches!(
            soft_delete(&pool, bob, category.id).await,
            Err(CategoryError::NotFound)
        ));

        soft_delete(&pool, alice, category.id)
            .await
            .expect("owner deletes it");
        assert!(list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .is_empty());
        assert!(matches!(
            soft_delete(&pool, alice, category.id).await,
            Err(CategoryError::NotFound)
        ));
    }
}
