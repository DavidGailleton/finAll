//! Input validation for the auth server functions.
//!
//! All values arriving from a server function are untrusted. These checks run on
//! the server and are the authoritative ones.
//!
//! NOTE: the specific limits below (password length bounds, email length, the
//! minimal email shape check) are placeholder policy. They are not derived from
//! any documented business rule and should be confirmed with the developer.

use crate::server::error::AuthError;

/// Minimum password length, in bytes. Placeholder policy.
const PASSWORD_MIN_LEN: usize = 8;

/// Maximum password length, in bytes. Bounds the work Argon2 does on a single
/// request so a huge body cannot be used to burn CPU.
const PASSWORD_MAX_LEN: usize = 1024;

/// Maximum stored email length. 254 is the practical RFC 5321 limit.
const EMAIL_MAX_LEN: usize = 254;

/// Trim surrounding whitespace and check the email has a minimally plausible
/// shape. The case is left unchanged: the `users.email` unique constraint is
/// case-sensitive, and whether to fold case is a decision for the developer.
pub fn email(input: &str) -> Result<String, AuthError> {
    let email = input.trim();

    if email.is_empty() {
        return Err(AuthError::InvalidInput("email is required"));
    }
    if email.len() > EMAIL_MAX_LEN {
        return Err(AuthError::InvalidInput("email is too long"));
    }

    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() && domain.contains('.') => {}
        _ => return Err(AuthError::InvalidInput("email is not valid")),
    }

    Ok(email.to_owned())
}

/// Check the password length bounds. Content rules (character classes, breach
/// lists, ...) are intentionally not imposed here.
pub fn password(input: &str) -> Result<(), AuthError> {
    if input.len() < PASSWORD_MIN_LEN {
        return Err(AuthError::InvalidInput("password is too short"));
    }
    if input.len() > PASSWORD_MAX_LEN {
        return Err(AuthError::InvalidInput("password is too long"));
    }
    Ok(())
}

/// Normalise an optional display name: trim, and treat blank as absent.
pub fn display_name(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
