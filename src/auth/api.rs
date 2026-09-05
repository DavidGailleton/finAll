//! Server functions the browser calls to sign up, sign in, sign out, and check
//! the current session.
//!
//! Each body runs only on the server (`ssr`). Server-only imports live inside the
//! function bodies so this module still compiles for the browser target, where
//! these become network calls.

use leptos::prelude::*;

use crate::auth::types::SessionUser;

/// Register a new account (open registration) and start a session.
#[server]
pub async fn signup(
    email: String,
    password: String,
    display_name: Option<String>,
) -> Result<SessionUser, ServerFnError> {
    use axum::http::header::SET_COOKIE;
    use leptos_axum::ResponseOptions;

    use crate::server::auth::{cookie, password as pw, session, token, user, validate};

    let pool = expect_context::<sqlx::PgPool>();

    let email = validate::email(&email)?;
    validate::password(&password)?;
    let display_name = validate::display_name(display_name.as_deref());

    let password_hash = pw::hash_password(&password)?;
    let user_id = user::create(&pool, &email, &password_hash, display_name.as_deref()).await?;

    let fresh = token::generate()?;
    session::create(&pool, user_id, &fresh.hash).await?;

    expect_context::<ResponseOptions>().insert_header(SET_COOKIE, cookie::build(&fresh.raw)?);

    Ok(SessionUser {
        id: user_id.to_string(),
        email,
        display_name,
    })
}

/// Verify credentials and start a session.
#[server]
pub async fn login(email: String, password: String) -> Result<SessionUser, ServerFnError> {
    use axum::http::header::SET_COOKIE;
    use leptos_axum::ResponseOptions;

    use crate::server::auth::{cookie, password as pw, session, token, user, validate};
    use crate::server::error::AuthError;

    let pool = expect_context::<sqlx::PgPool>();
    let email = validate::normalize_email(&email);

    let record = user::find_active_by_email(&pool, &email).await?;

    let account = match record {
        Some(account) => {
            if !pw::verify_password(&password, &account.password_hash)? {
                return Err(AuthError::InvalidCredentials.into());
            }
            account
        }
        None => {
            // Spend comparable time hashing so the response does not reveal
            // whether the email is registered. The result is discarded.
            if let Err(err) = pw::hash_password(&password) {
                leptos::logging::error!("auth: timing-equaliser hash failed: {err}");
            }
            return Err(AuthError::InvalidCredentials.into());
        }
    };

    let fresh = token::generate()?;
    session::create(&pool, account.id, &fresh.hash).await?;

    expect_context::<ResponseOptions>().insert_header(SET_COOKIE, cookie::build(&fresh.raw)?);

    Ok(SessionUser {
        id: account.id.to_string(),
        email: account.email,
        display_name: account.display_name,
    })
}

/// Revoke the current session and clear the cookie.
#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use axum::http::header::{COOKIE, SET_COOKIE};
    use axum::http::HeaderMap;
    use leptos_axum::ResponseOptions;

    use crate::server::auth::{cookie, session, token};
    use crate::server::error::AuthError;

    let pool = expect_context::<sqlx::PgPool>();

    let headers: HeaderMap = leptos_axum::extract().await.map_err(|err| {
        leptos::logging::error!("auth: could not read request headers: {err:?}");
        AuthError::Internal
    })?;

    if let Some(raw) = headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(cookie::read_token)
    {
        session::revoke(&pool, &token::hash_token(raw)).await?;
    }

    expect_context::<ResponseOptions>().insert_header(SET_COOKIE, cookie::clear()?);

    Ok(())
}

/// The user for the current session, or `None` if not signed in. Validating the
/// session also slides its expiry forward.
#[server]
pub async fn current_user() -> Result<Option<SessionUser>, ServerFnError> {
    use crate::server::auth::extract;

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool).await?;

    Ok(user.map(|user| SessionUser {
        id: user.user_id.to_string(),
        email: user.email,
        display_name: user.display_name,
    }))
}
