//! DTOs exchanged between the browser and the merchant server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type.

use serde::{Deserialize, Serialize};

/// A merchant, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MerchantDto {
    /// Merchant id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    pub merchant_name: String,
    /// The user's default category for this merchant, as a string id, or
    /// `None` when unset. The category name for display is resolved on the
    /// page from the separately loaded category list.
    pub default_category_id: Option<String>,
}
