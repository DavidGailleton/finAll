//! Password hashing with Argon2id (RustCrypto defaults, random per-password
//! salt). Hashes are stored as PHC strings in `users.password_hash`.
//!
//! Argon2id is deliberately CPU- and memory-heavy, so both operations run on
//! `tokio::task::spawn_blocking` rather than inline on an async worker thread.

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use leptos::logging;

use crate::server::error::AuthError;

/// Hash a plaintext password into a PHC string suitable for storage.
pub async fn hash_password(password: &str) -> Result<String, AuthError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(AuthError::from)
    })
    .await
    .map_err(|err| {
        logging::error!("auth: password hash task failed to join: {err}");
        AuthError::Internal
    })?
}

/// Check a plaintext password against a stored PHC string.
///
/// Returns `Ok(false)` for a genuine mismatch and `Err(Internal)` if the stored
/// hash cannot be parsed.
pub async fn verify_password(password: &str, phc: &str) -> Result<bool, AuthError> {
    let password = password.to_owned();
    let phc = phc.to_owned();
    tokio::task::spawn_blocking(move || {
        match Argon2::default().verify_password(password.as_bytes(), phc.as_str()) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
            Err(err) => Err(err.into()),
        }
    })
    .await
    .map_err(|err| {
        logging::error!("auth: password verify task failed to join: {err}");
        AuthError::Internal
    })?
}
