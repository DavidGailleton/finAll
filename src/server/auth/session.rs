//! The `sessions` table: create a session, validate a token (sliding the
//! expiry), and revoke on logout.
//!
//! Expiry is computed on the database with `now() + make_interval(...)` so there
//! is a single clock and no Rust/Postgres skew.

use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::error::AuthError;

/// Session lifetime, in days. Each successful validation slides the expiry this
/// far forward from the database's current time.
pub const SESSION_TTL_DAYS: i32 = 30;

/// The user behind a valid session, as needed by the auth server functions.
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
}

/// Create a session row for `user_id` holding the given token hash.
pub async fn create(pool: &PgPool, user_id: Uuid, token_hash: &str) -> Result<(), AuthError> {
    sqlx::query!(
        r#"
        INSERT INTO sessions (user_id, token_hash, expires_at)
        VALUES ($1, $2, now() + make_interval(days => $3))
        "#,
        user_id,
        token_hash,
        SESSION_TTL_DAYS,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Look up the active user for a session token hash, and slide the session
/// expiry forward. Returns `Ok(None)` when there is no unexpired session for a
/// non-deleted user.
pub async fn authenticate(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<AuthenticatedUser>, AuthError> {
    let row = sqlx::query!(
        r#"
        UPDATE sessions AS s
        SET last_used_at = now(),
            expires_at = now() + make_interval(days => $2)
        FROM users AS u
        WHERE s.token_hash = $1
          AND s.expires_at > now()
          AND u.id = s.user_id
          AND u.deleted_at IS NULL
        RETURNING
            u.id AS "user_id!",
            u.email AS "email!",
            u.display_name
        "#,
        token_hash,
        SESSION_TTL_DAYS,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AuthenticatedUser {
        user_id: r.user_id,
        email: r.email,
        display_name: r.display_name,
    }))
}

/// Delete the session identified by this token hash. A logout with no matching
/// row is not an error.
pub async fn revoke(pool: &PgPool, token_hash: &str) -> Result<(), AuthError> {
    sqlx::query!(
        r#"
        DELETE FROM sessions
        WHERE token_hash = $1
        "#,
        token_hash,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete every session whose expiry has already passed. `revoke` already
/// removes a session immediately on logout; this sweeps sessions that expired
/// without ever being revoked (e.g. a user who never logged out). Returns the
/// number of rows removed.
pub async fn prune_expired(pool: &PgPool) -> Result<u64, AuthError> {
    let result = sqlx::query!(
        r#"
        DELETE FROM sessions
        WHERE expires_at < now()
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
