//! DTOs exchanged between the browser and the account server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type.

use serde::{Deserialize, Serialize};

/// The kind of an account. The variants are exactly the values allowed by the
/// `accounts_type_valid` check constraint; they serialize as those lowercase
/// strings both across the server-function boundary and in the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountType {
    Cash,
    Bank,
    Credit,
    Investment,
    Crypto,
    Loan,
    Other,
}

impl AccountType {
    /// Every variant, in declaration order.
    const ALL: [AccountType; 7] = [
        AccountType::Cash,
        AccountType::Bank,
        AccountType::Credit,
        AccountType::Investment,
        AccountType::Crypto,
        AccountType::Loan,
        AccountType::Other,
    ];

    /// The string stored in `accounts.account_type`.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            AccountType::Cash => "cash",
            AccountType::Bank => "bank",
            AccountType::Credit => "credit",
            AccountType::Investment => "investment",
            AccountType::Crypto => "crypto",
            AccountType::Loan => "loan",
            AccountType::Other => "other",
        }
    }

    /// Parse the string stored in `accounts.account_type`. Returns `None` for
    /// any value the check constraint would not allow.
    pub fn from_db_str(value: &str) -> Option<AccountType> {
        AccountType::ALL
            .into_iter()
            .find(|variant| variant.as_db_str() == value)
    }
}

/// An account, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountDto {
    /// Account id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    pub account_name: String,
    pub account_type: AccountType,
    /// The id of the currency (a fiat `assets` row) this account is denominated
    /// in, rendered as a string for the same reason as `id`.
    pub default_asset_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_str_round_trips_for_every_variant() {
        for variant in AccountType::ALL {
            assert_eq!(AccountType::from_db_str(variant.as_db_str()), Some(variant));
        }
    }

    #[test]
    fn from_db_str_rejects_unknown() {
        assert_eq!(AccountType::from_db_str("savings"), None);
        assert_eq!(AccountType::from_db_str(""), None);
        assert_eq!(AccountType::from_db_str("Cash"), None);
    }
}
