//! Resolving the current user for a server function from the request cookie.

use axum::http::header::COOKIE;
use axum::http::HeaderMap;
use leptos::logging;

use crate::server::auth::{cookie, session, token};
use crate::server::error::AuthError;

/// Resolve the current authenticated user from the request's session cookie,
/// sliding the session expiry forward.
///
/// Returns `Ok(None)` when there is no cookie or no valid session; that is the
/// normal "not signed in" case, not an error.
pub async fn current_user(
    pool: &sqlx::PgPool,
) -> Result<Option<session::AuthenticatedUser>, AuthError> {
    let headers: HeaderMap = leptos_axum::extract().await.map_err(|err| {
        logging::error!("auth: could not read request headers: {err:?}");
        AuthError::Internal
    })?;

    let Some(raw) = headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(cookie::read_token)
    else {
        return Ok(None);
    };

    session::authenticate(pool, &token::hash_token(raw)).await
}
