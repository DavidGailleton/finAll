//! Opaque session tokens.
//!
//! A token is 32 bytes from the OS CSPRNG, hex-encoded. The raw value is sent to
//! the browser in the session cookie; only its SHA-256 hash is stored in
//! `sessions.token_hash`, so a database leak does not expose usable tokens.
//! SHA-256 (not Argon2) is used here because the token already has full entropy
//! and the hash is on the hot path of every authenticated request.

use sha2::{Digest, Sha256};

use crate::server::error::AuthError;

/// Number of random bytes in a raw token, before hex encoding.
const TOKEN_BYTES: usize = 32;

/// A freshly generated token: `raw` goes in the cookie, `hash` goes in the
/// database.
pub struct GeneratedToken {
    pub raw: String,
    pub hash: String,
}

/// Generate a new random token and its storage hash.
pub fn generate() -> Result<GeneratedToken, AuthError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    let raw = hex_encode(&bytes);
    let hash = hash_token(&raw);
    Ok(GeneratedToken { raw, hash })
}

/// Hash a raw token for storage or lookup: SHA-256, hex-encoded.
pub fn hash_token(raw: &str) -> String {
    hex_encode(&Sha256::digest(raw.as_bytes()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
