//! Input validation for the auth server functions.
//!
//! All values arriving from a server function are untrusted. These checks run on
//! the server and are the authoritative ones.
//!
//! Password and email policy follow the OWASP Authentication Cheat Sheet: a
//! minimum password length with no forced composition rules and a generous
//! maximum, and a pragmatic (not full RFC 5322 grammar) email shape check.
//! Email is case-folded to lowercase before storage or comparison — see
//! [`normalize_email`] — rather than via a partial/expression unique index, so
//! two accounts can never differ only by email case.

use crate::server::error::AuthError;

/// Minimum password length, in bytes. OWASP Authentication Cheat Sheet /
/// NIST SP 800-63B baseline.
const PASSWORD_MIN_LEN: usize = 8;

/// Maximum password length, in bytes. OWASP requires accepting at least 64
/// characters with no forced truncation; this value also bounds the work
/// Argon2 does on a single request so a huge body cannot be used to burn CPU.
const PASSWORD_MAX_LEN: usize = 1024;

/// Maximum stored email length. 254 is the practical RFC 5321 limit.
const EMAIL_MAX_LEN: usize = 254;

/// Trim and case-fold an email to the canonical form used for both storage
/// and lookup. Every write and every query must go through this function so
/// the two stay in sync.
pub fn normalize_email(input: &str) -> String {
    input.trim().to_lowercase()
}

/// Normalize an email and check it has a minimally plausible shape. OWASP
/// discourages complex email regexes (readability and ReDoS risk), so this is
/// deliberately just a length bound plus a `local@domain.tld` shape check
/// rather than a full RFC 5322 grammar.
pub fn email(input: &str) -> Result<String, AuthError> {
    let email = normalize_email(input);

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

    Ok(email)
}

/// Check the password length bounds (OWASP: minimum 8, no forced composition
/// rules, no maximum below 64). Character-class rules and breached-password
/// checks are intentionally not imposed here.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(email("  User@Example.COM  ").unwrap(), "user@example.com");
    }

    #[test]
    fn email_rejects_empty() {
        assert!(matches!(email("   "), Err(AuthError::InvalidInput(_))));
    }

    #[test]
    fn email_rejects_missing_at() {
        assert!(matches!(
            email("user.example.com"),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn email_rejects_empty_local_part() {
        assert!(matches!(
            email("@example.com"),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn email_rejects_domain_without_dot() {
        assert!(matches!(
            email("user@localhost"),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn email_rejects_too_long() {
        let local = "a".repeat(EMAIL_MAX_LEN);
        let too_long = format!("{local}@example.com");
        assert!(matches!(email(&too_long), Err(AuthError::InvalidInput(_))));
    }

    #[test]
    fn email_accepts_max_length() {
        // Exactly EMAIL_MAX_LEN total, so it must be accepted.
        let domain = "@example.com";
        let local = "a".repeat(EMAIL_MAX_LEN - domain.len());
        let max_len_email = format!("{local}{domain}");
        assert_eq!(max_len_email.len(), EMAIL_MAX_LEN);
        assert!(email(&max_len_email).is_ok());
    }

    #[test]
    fn password_rejects_below_minimum() {
        assert!(matches!(
            password(&"a".repeat(PASSWORD_MIN_LEN - 1)),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn password_accepts_minimum() {
        assert!(password(&"a".repeat(PASSWORD_MIN_LEN)).is_ok());
    }

    #[test]
    fn password_accepts_maximum() {
        assert!(password(&"a".repeat(PASSWORD_MAX_LEN)).is_ok());
    }

    #[test]
    fn password_rejects_above_maximum() {
        assert!(matches!(
            password(&"a".repeat(PASSWORD_MAX_LEN + 1)),
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[test]
    fn display_name_treats_blank_as_absent() {
        assert_eq!(display_name(Some("   ")), None);
        assert_eq!(display_name(None), None);
        assert_eq!(display_name(Some("  Ada  ")), Some("Ada".to_owned()));
    }
}
