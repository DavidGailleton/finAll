//! The error type for the authentication layer.
//!
//! Variants carry only messages that are safe to show a user. Internal failures
//! (database, hashing, RNG) are logged server-side and collapsed into
//! [`AuthError::Internal`] so that SQL text, stack details or secrets never reach
//! the client. `ServerFnError` picks up the `Display` string through its blanket
//! `From<E: std::error::Error>` impl.

use leptos::logging;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid email or password")]
    InvalidCredentials,

    #[error("an account with this email already exists")]
    EmailTaken,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for AuthError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("auth: database error: {err}");
        AuthError::Internal
    }
}

impl From<argon2::password_hash::Error> for AuthError {
    fn from(err: argon2::password_hash::Error) -> Self {
        logging::error!("auth: password hashing error: {err}");
        AuthError::Internal
    }
}

impl From<getrandom::Error> for AuthError {
    fn from(err: getrandom::Error) -> Self {
        logging::error!("auth: RNG failure: {err}");
        AuthError::Internal
    }
}
