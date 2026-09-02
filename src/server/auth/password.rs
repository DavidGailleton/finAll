//! Password hashing with Argon2id (RustCrypto defaults, random per-password
//! salt). Hashes are stored as PHC strings in `users.password_hash`.

use argon2::{Argon2, PasswordHasher, PasswordVerifier};

use crate::server::error::AuthError;

/// Hash a plaintext password into a PHC string suitable for storage.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let hash = Argon2::default().hash_password(password.as_bytes())?;
    Ok(hash.to_string())
}

/// Check a plaintext password against a stored PHC string.
///
/// Returns `Ok(false)` for a genuine mismatch and `Err(Internal)` if the stored
/// hash cannot be parsed.
pub fn verify_password(password: &str, phc: &str) -> Result<bool, AuthError> {
    match Argon2::default().verify_password(password.as_bytes(), phc) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
        Err(err) => Err(err.into()),
    }
}
