//! Input validation for the columns on the `assets` spine, shared by every
//! asset class.
//!
//! Values arriving from a server function are untrusted; these server-side
//! checks are the authoritative ones and mirror the `assets` table's CHECK
//! constraints. Class-specific validation (a fiat code shape, minor units, …)
//! lives with that class, e.g. [`crate::server::assets::currency`].
//!
//! `name` returns a `&'static str` message rather than a domain error so it
//! does not depend on any one class's error type; the caller maps it into its
//! own error.

/// Trim the asset name and require it non-blank (mirrors `assets_name_not_empty`).
pub fn name(input: &str) -> Result<String, &'static str> {
    let name = input.trim();
    if name.is_empty() {
        return Err("name is required");
    }
    Ok(name.to_owned())
}

/// Trim an optional symbol, treating blank as absent (mirrors
/// `assets_symbol_not_empty`, which only constrains a non-null value).
pub fn symbol(input: Option<&str>) -> Option<String> {
    input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_trimmed() {
        assert_eq!(name("  Euro  ").unwrap(), "Euro");
    }

    #[test]
    fn name_rejects_blank() {
        assert!(name("   ").is_err());
    }

    #[test]
    fn symbol_treats_blank_as_absent() {
        assert_eq!(symbol(Some("   ")), None);
        assert_eq!(symbol(None), None);
        assert_eq!(symbol(Some("  €  ")), Some("€".to_owned()));
    }
}
