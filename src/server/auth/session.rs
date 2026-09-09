//! The `sessions` table: create a session, validate a token (sliding the
//! expiry), and revoke on logout.
//!
//! Expiry is computed on the database with `now() + make_interval(...)` so there
//! is a single clock and no Rust/Postgres skew.

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::error::AuthError;

/// Session lifetime, in days. A validation slides the expiry this far forward
/// from the database's current time.
pub const SESSION_TTL_DAYS: i32 = 30;

/// Slide the session expiry at most once per this many hours of use. A
/// validation within this window of the session's last recorded use skips the
/// write entirely, so ordinary navigation does not rewrite the row on every
/// request.
const SESSION_SLIDE_THROTTLE_HOURS: i32 = 1;

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

/// Look up the active user for a session token hash. Returns `Ok(None)` when
/// there is no unexpired session for a non-deleted user.
///
/// A used session's expiry still slides `SESSION_TTL_DAYS` forward, but the
/// write happens at most once per `SESSION_SLIDE_THROTTLE_HOURS`: the validation
/// itself is a read, and the slide `UPDATE` is skipped while the session was
/// last used inside that window.
pub async fn authenticate(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<AuthenticatedUser>, AuthError> {
    let row = sqlx::query!(
        r#"
        SELECT
            s.id AS session_id,
            u.id AS "user_id!",
            u.email AS "email!",
            u.display_name,
            (s.last_used_at < now() - make_interval(hours => $2)) AS "needs_slide!"
        FROM sessions AS s
        INNER JOIN users AS u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.expires_at > now()
          AND u.deleted_at IS NULL
        "#,
        token_hash,
        SESSION_SLIDE_THROTTLE_HOURS,
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    if row.needs_slide {
        // Authentication has already succeeded; a failed expiry bump must not
        // fail the request.
        if let Err(err) = sqlx::query!(
            r#"
            UPDATE sessions
            SET last_used_at = now(),
                expires_at = now() + make_interval(days => $2)
            WHERE id = $1
            "#,
            row.session_id,
            SESSION_TTL_DAYS,
        )
        .execute(pool)
        .await
        {
            logging::error!("auth: failed to slide session expiry: {err}");
        }
    }

    Ok(Some(AuthenticatedUser {
        user_id: row.user_id,
        email: row.email,
        display_name: row.display_name,
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

/// The throttled slide is the behaviour these cover: a validation still resolves
/// the user and still slides an aged session forward `SESSION_TTL_DAYS`, but a
/// validation inside `SESSION_SLIDE_THROTTLE_HOURS` of the last recorded use
/// leaves the row untouched. Each `#[sqlx::test]` runs against its own freshly
/// migrated database. Queries here use the runtime (`sqlx::query`) form on
/// purpose — `cargo sqlx prepare` does not compile test code, so a macro query
/// would have no `.sqlx/` entry.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::server::test_support::create_user;
    use sqlx::types::chrono::{DateTime, Utc};

    const TOKEN_HASH: &str = "test-session-token-hash";

    async fn expires_at(pool: &PgPool, token_hash: &str) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_one(pool)
            .await
            .expect("session row exists")
    }

    #[sqlx::test]
    async fn authenticate_resolves_a_valid_session_to_its_user(pool: PgPool) {
        let user_id = create_user(&pool, "alice@example.test").await;
        create(&pool, user_id, TOKEN_HASH)
            .await
            .expect("create session");

        let found = authenticate(&pool, TOKEN_HASH)
            .await
            .expect("query runs")
            .expect("a valid session resolves to its user");
        assert_eq!(found.user_id, user_id);
        assert_eq!(found.email, "alice@example.test");
    }

    #[sqlx::test]
    async fn authenticate_rejects_an_expired_session(pool: PgPool) {
        let user_id = create_user(&pool, "alice@example.test").await;
        create(&pool, user_id, TOKEN_HASH)
            .await
            .expect("create session");

        // `created_at` moves too, so `sessions_expires_after_created` still holds.
        sqlx::query(
            "UPDATE sessions
             SET created_at = now() - make_interval(days => 2),
                 expires_at = now() - make_interval(days => 1)
             WHERE token_hash = $1",
        )
        .bind(TOKEN_HASH)
        .execute(&pool)
        .await
        .expect("backdate expiry");

        assert!(authenticate(&pool, TOKEN_HASH)
            .await
            .expect("query runs")
            .is_none());
    }

    #[sqlx::test]
    async fn authenticate_does_not_slide_within_the_throttle_window(pool: PgPool) {
        let user_id = create_user(&pool, "alice@example.test").await;
        create(&pool, user_id, TOKEN_HASH)
            .await
            .expect("create session");

        // `create` sets `last_used_at` to now(), so the first validation is well
        // inside the throttle window and must not rewrite the row.
        let before = expires_at(&pool, TOKEN_HASH).await;
        authenticate(&pool, TOKEN_HASH)
            .await
            .expect("query runs")
            .expect("valid session");
        assert_eq!(before, expires_at(&pool, TOKEN_HASH).await);
    }

    #[sqlx::test]
    async fn authenticate_slides_once_the_throttle_window_has_passed(pool: PgPool) {
        let user_id = create_user(&pool, "alice@example.test").await;
        create(&pool, user_id, TOKEN_HASH)
            .await
            .expect("create session");

        // Age the last use past the throttle window and pull the expiry in, so a
        // slide back to `now() + SESSION_TTL_DAYS` is observable.
        sqlx::query(
            "UPDATE sessions
             SET last_used_at = now() - make_interval(hours => 2),
                 expires_at = now() + make_interval(days => 1)
             WHERE token_hash = $1",
        )
        .bind(TOKEN_HASH)
        .execute(&pool)
        .await
        .expect("age the session");

        let before = expires_at(&pool, TOKEN_HASH).await;
        authenticate(&pool, TOKEN_HASH)
            .await
            .expect("query runs")
            .expect("valid session");
        assert!(
            expires_at(&pool, TOKEN_HASH).await > before,
            "a stale session's expiry should slide forward"
        );
    }
}
