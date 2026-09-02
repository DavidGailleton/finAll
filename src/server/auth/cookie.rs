//! The session cookie: `finall_session`, holding the raw opaque token.
//!
//! `HttpOnly` and `SameSite=Lax` always; `Secure` unless `LEPTOS_ENV=DEV`, so
//! local development over plain HTTP still works while every deployed
//! environment gets a secure cookie.

use axum::http::HeaderValue;

use crate::server::auth::session::SESSION_TTL_DAYS;
use crate::server::error::AuthError;

pub const COOKIE_NAME: &str = "finall_session";

fn is_secure() -> bool {
    std::env::var("LEPTOS_ENV")
        .map(|value| !value.eq_ignore_ascii_case("DEV"))
        .unwrap_or(true)
}

fn header_value(body: &str) -> Result<HeaderValue, AuthError> {
    let mut value = body.to_owned();
    if is_secure() {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value).map_err(|_| AuthError::Internal)
}

/// A `Set-Cookie` value that stores `token` for the full session lifetime.
pub fn build(token: &str) -> Result<HeaderValue, AuthError> {
    let max_age = SESSION_TTL_DAYS * 24 * 60 * 60;
    header_value(&format!(
        "{COOKIE_NAME}={token}; Max-Age={max_age}; Path=/; HttpOnly; SameSite=Lax"
    ))
}

/// A `Set-Cookie` value that immediately clears the session cookie.
pub fn clear() -> Result<HeaderValue, AuthError> {
    header_value(&format!(
        "{COOKIE_NAME}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax"
    ))
}

/// Extract the session token from a `Cookie` request header value.
pub fn read_token(cookie_header: &str) -> Option<&str> {
    cookie_header
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name.trim() == COOKIE_NAME).then_some(value.trim()))
}
