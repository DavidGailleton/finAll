//! DTOs exchanged between the browser and the category server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type.

use serde::{Deserialize, Serialize};

/// The kind of a category. The variants are exactly the values allowed by the
/// `categories_kind_valid` check constraint; they serialize as those lowercase
/// strings both across the server-function boundary and in the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryKind {
    Income,
    Expense,
}

impl CategoryKind {
    /// Every variant, in declaration order.
    pub const ALL: [CategoryKind; 2] = [CategoryKind::Income, CategoryKind::Expense];

    /// The string stored in `categories.kind`.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            CategoryKind::Income => "income",
            CategoryKind::Expense => "expense",
        }
    }

    /// Parse the string stored in `categories.kind`. Returns `None` for any
    /// value the check constraint would not allow.
    pub fn from_db_str(value: &str) -> Option<CategoryKind> {
        CategoryKind::ALL
            .into_iter()
            .find(|variant| variant.as_db_str() == value)
    }

    /// A capitalized label for display in the UI.
    pub fn label(&self) -> &'static str {
        match self {
            CategoryKind::Income => "Income",
            CategoryKind::Expense => "Expense",
        }
    }
}

/// A category, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CategoryDto {
    /// Category id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    pub category_name: String,
    /// Fixed when the category is created; the edit form does not change it.
    pub kind: CategoryKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_str_round_trips_for_every_variant() {
        for variant in CategoryKind::ALL {
            assert_eq!(
                CategoryKind::from_db_str(variant.as_db_str()),
                Some(variant)
            );
            assert!(!variant.label().is_empty());
        }
    }

    #[test]
    fn from_db_str_rejects_unknown() {
        assert_eq!(CategoryKind::from_db_str("transfer"), None);
        assert_eq!(CategoryKind::from_db_str(""), None);
        assert_eq!(CategoryKind::from_db_str("Income"), None);
    }
}
