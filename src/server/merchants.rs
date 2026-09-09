//! Queries for the signed-in user's merchants (`merchants` table), the domain
//! error, and the input validation for the merchant server functions.
//!
//! Every value arriving from a server function is untrusted. `validate_name` is
//! the authoritative check for `merchant_name`. `default_category_id` is
//! optional; the `merchants_default_category_fk` foreign key on
//! `(user_id, default_category_id)` guarantees a chosen category belongs to the
//! same user, so a mismatched or missing category is reported as
//! [`MerchantError::UnknownCategory`]. Each query is scoped by `user_id` so one
//! user can never read or change another's merchants.

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum MerchantError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("merchant not found")]
    NotFound,

    #[error("you already have a merchant with that name")]
    DuplicateName,

    #[error("the selected category does not exist")]
    UnknownCategory,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for MerchantError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("merchants: database error: {err}");
        MerchantError::Internal
    }
}

/// The columns needed to render a merchant to the browser.
pub struct MerchantRecord {
    pub id: Uuid,
    pub merchant_name: String,
    pub default_category_id: Option<Uuid>,
}

/// Trim the merchant name and require it non-blank (mirrors
/// `merchants_name_not_empty`).
pub fn validate_name(input: &str) -> Result<String, MerchantError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(MerchantError::InvalidInput("merchant name is required"));
    }
    Ok(name.to_owned())
}

/// List the user's active (non-deleted) merchants, ordered by name.
pub async fn list_active_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<MerchantRecord>, MerchantError> {
    let records = sqlx::query_as!(
        MerchantRecord,
        r#"
        SELECT id, merchant_name, default_category_id
        FROM merchants
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY merchant_name
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Insert a new merchant for the user and return the created record.
///
/// A name the user already uses (`merchants_user_id_name_unique`) is mapped to
/// [`MerchantError::DuplicateName`]; a `default_category_id` that is not one of
/// this user's categories (`merchants_default_category_fk`) is mapped to
/// [`MerchantError::UnknownCategory`].
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    merchant_name: &str,
    default_category_id: Option<Uuid>,
) -> Result<MerchantRecord, MerchantError> {
    let row = match sqlx::query!(
        r#"
        INSERT INTO merchants (user_id, merchant_name, default_category_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
        user_id,
        merchant_name,
        default_category_id,
    )
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(MerchantError::DuplicateName);
        }
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(MerchantError::UnknownCategory);
        }
        Err(err) => return Err(err.into()),
    };

    Ok(MerchantRecord {
        id: row.id,
        merchant_name: merchant_name.to_owned(),
        default_category_id,
    })
}

/// Update the user's merchant (name and default category) and return the
/// updated record.
///
/// Returns [`MerchantError::NotFound`] if the id does not match one of this
/// user's non-deleted merchants, [`MerchantError::DuplicateName`] if the new
/// name is already used by another of the user's merchants, and
/// [`MerchantError::UnknownCategory`] if `default_category_id` is not one of
/// this user's categories.
pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    merchant_name: &str,
    default_category_id: Option<Uuid>,
) -> Result<MerchantRecord, MerchantError> {
    let row = match sqlx::query!(
        r#"
        UPDATE merchants
        SET merchant_name = $3,
            default_category_id = $4,
            updated_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        RETURNING id
        "#,
        id,
        user_id,
        merchant_name,
        default_category_id,
    )
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
            return Err(MerchantError::DuplicateName);
        }
        Err(sqlx::Error::Database(err)) if err.is_foreign_key_violation() => {
            return Err(MerchantError::UnknownCategory);
        }
        Err(err) => return Err(err.into()),
    };

    let Some(row) = row else {
        return Err(MerchantError::NotFound);
    };

    Ok(MerchantRecord {
        id: row.id,
        merchant_name: merchant_name.to_owned(),
        default_category_id,
    })
}

/// Soft-delete the user's merchant by setting `deleted_at`.
///
/// Returns [`MerchantError::NotFound`] if the id does not match one of this
/// user's non-deleted merchants.
pub async fn soft_delete(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<(), MerchantError> {
    let result = sqlx::query!(
        r#"
        UPDATE merchants
        SET deleted_at = now()
        WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(MerchantError::NotFound);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_trims() {
        assert_eq!(validate_name("  Acme Corp  ").unwrap(), "Acme Corp");
    }

    #[test]
    fn validate_name_rejects_blank() {
        assert!(validate_name("   ").is_err());
        assert!(validate_name("").is_err());
    }
}

/// Authorization: every query here is scoped by `user_id`, so one user must
/// never read or change another's merchants. A wrong owner is reported as
/// [`MerchantError::NotFound`], indistinguishable from a missing row.
///
/// Each `#[sqlx::test]` runs against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::categories;
    use crate::server::test_support::create_user;

    /// A category id belonging to `user_id`, for the `default_category_id` slot.
    async fn a_category(pool: &PgPool, user_id: Uuid, name: &str) -> Uuid {
        categories::create(pool, user_id, name, "expense")
            .await
            .expect("category")
            .id
    }

    #[sqlx::test]
    async fn list_active_for_user_returns_only_the_owners_merchants_ordered(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        create(&pool, alice, "Zebra", None).await.expect("alice 1");
        create(&pool, alice, "Acme", None).await.expect("alice 2");
        create(&pool, bob, "Acme", None).await.expect("bob 1");

        let names: Vec<String> = list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .into_iter()
            .map(|record| record.merchant_name)
            .collect();
        assert_eq!(names, vec!["Acme", "Zebra"]);

        let bobs = list_active_for_user(&pool, bob).await.expect("lists");
        assert_eq!(bobs.len(), 1);
        assert_eq!(bobs[0].merchant_name, "Acme");
    }

    #[sqlx::test]
    async fn create_assigns_the_calling_users_id(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let merchant = create(&pool, alice, "Acme", None)
            .await
            .expect("alice's merchant");

        assert!(list_active_for_user(&pool, bob)
            .await
            .expect("lists")
            .is_empty());
        assert!(matches!(
            update(&pool, bob, merchant.id, "Stolen", None).await,
            Err(MerchantError::NotFound)
        ));
    }

    #[sqlx::test]
    async fn create_rejects_a_duplicate_name_but_allows_it_for_another_user(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        create(&pool, alice, "Acme", None).await.expect("first");

        assert!(matches!(
            create(&pool, alice, "Acme", None).await,
            Err(MerchantError::DuplicateName)
        ));

        create(&pool, bob, "Acme", None).await.expect("bob's own");
    }

    #[sqlx::test]
    async fn create_and_update_manage_the_default_category(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let groceries = a_category(&pool, alice, "Groceries").await;
        let bobs_category = a_category(&pool, bob, "Bob's").await;

        // Created with the caller's own category.
        let merchant = create(&pool, alice, "Acme", Some(groceries))
            .await
            .expect("with category");
        assert_eq!(merchant.default_category_id, Some(groceries));

        // Cleared on update.
        let cleared = update(&pool, alice, merchant.id, "Acme", None)
            .await
            .expect("clear category");
        assert_eq!(cleared.default_category_id, None);

        // Another user's category is rejected on both paths.
        assert!(matches!(
            create(&pool, alice, "Other", Some(bobs_category)).await,
            Err(MerchantError::UnknownCategory)
        ));
        assert!(matches!(
            update(&pool, alice, merchant.id, "Acme", Some(bobs_category)).await,
            Err(MerchantError::UnknownCategory)
        ));
    }

    #[sqlx::test]
    async fn update_denies_another_users_merchant_and_rejects_a_colliding_rename(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let groceries = a_category(&pool, alice, "Groceries").await;
        let acme = create(&pool, alice, "Acme", Some(groceries))
            .await
            .expect("acme");
        create(&pool, alice, "Globex", None).await.expect("globex");

        // Not the owner: NotFound, and nothing written.
        assert!(matches!(
            update(&pool, bob, acme.id, "Hijacked", None).await,
            Err(MerchantError::NotFound)
        ));

        // Renaming onto an existing name is a duplicate.
        assert!(matches!(
            update(&pool, alice, acme.id, "Globex", None).await,
            Err(MerchantError::DuplicateName)
        ));

        // The rejected writes left the row untouched.
        let after = list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .into_iter()
            .find(|record| record.id == acme.id)
            .expect("still there");
        assert_eq!(after.merchant_name, "Acme");
        assert_eq!(after.default_category_id, Some(groceries));
    }

    #[sqlx::test]
    async fn soft_delete_is_scoped_and_not_repeatable(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;

        let merchant = create(&pool, alice, "Acme", None).await.expect("merchant");

        assert!(matches!(
            soft_delete(&pool, bob, merchant.id).await,
            Err(MerchantError::NotFound)
        ));

        soft_delete(&pool, alice, merchant.id)
            .await
            .expect("owner deletes it");
        assert!(list_active_for_user(&pool, alice)
            .await
            .expect("lists")
            .is_empty());
        assert!(matches!(
            soft_delete(&pool, alice, merchant.id).await,
            Err(MerchantError::NotFound)
        ));
    }
}
