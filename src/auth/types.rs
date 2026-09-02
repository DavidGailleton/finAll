//! DTOs exchanged between the browser and the auth server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type.

use serde::{Deserialize, Serialize};

/// The authenticated user, in the shape the browser is allowed to see.
///
/// Deliberately excludes the password hash and any other sensitive column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUser {
    /// User id, rendered as a string so this type needs no `uuid` dependency on
    /// the client.
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
}
