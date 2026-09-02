//! Queries against the `users` table for authentication.

use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::error::AuthError;

/// The columns needed to authenticate a user.
pub struct UserRecord {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub password_hash: String,
}

/// Look up an active (not soft-deleted) user by exact email match.
pub async fn find_active_by_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<UserRecord>, AuthError> {
    let record = sqlx::query_as!(
        UserRecord,
        r#"
        SELECT id, email, display_name, password_hash
        FROM users
        WHERE email = $1 AND deleted_at IS NULL
        "#,
        email,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record)
}

/// Insert a new user and return its id.
///
/// A unique-constraint violation on the email is mapped to
/// [`AuthError::EmailTaken`]. Note that the `users.email` unique constraint is
/// not partial, so an email belonging to a soft-deleted user still counts as
/// taken.
pub async fn create(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    display_name: Option<&str>,
) -> Result<Uuid, AuthError> {
    let result = sqlx::query!(
        r#"
        INSERT INTO users (email, password_hash, display_name)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
        email,
        password_hash,
        display_name,
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(row) => Ok(row.id),
        Err(sqlx::Error::Database(err)) if err.is_unique_violation() => Err(AuthError::EmailTaken),
        Err(err) => Err(err.into()),
    }
}
