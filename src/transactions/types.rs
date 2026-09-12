//! DTOs exchanged between the browser and the transaction server functions.
//!
//! These are compiled for both targets, so they must not reference any
//! server-only type. Following the rest of the app, ids are strings; `amount`
//! is the exact stored decimal rendered as a string (never a float), and the
//! dates are ISO `YYYY-MM-DD` strings.

use serde::{Deserialize, Serialize};

use crate::accounts::types::AccountType;

/// One of the user's transactions, in the shape the browser is allowed to see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionDto {
    /// Transaction id, rendered as a string so this type needs no `uuid`
    /// dependency on the client.
    pub id: String,
    /// Id of the account this transaction is on, rendered as a string.
    pub account_id: String,
    /// Name of that account. Shown on the global transaction list; the
    /// per-account list ignores it.
    pub account_name: String,
    /// The account's kind — the add/edit form shows a single date field for a
    /// `Cash` account.
    pub account_type: AccountType,
    /// Id of the account's default currency, rendered as a string. Used to tell
    /// whether this transaction is in a foreign currency.
    pub account_default_asset_id: String,
    /// Alphabetic code of the account's default currency, for rendering the
    /// converted amount.
    pub account_currency_code: String,
    /// The signed transaction amount, exactly as stored (`NUMERIC(38, 18)`),
    /// rendered as a decimal string. Never rounded, truncated, or parsed
    /// through a floating-point value.
    pub amount: String,
    /// The amount in the account's currency, translated at the booking-date
    /// rate. `None` when the transaction is already in the account currency or
    /// its conversion is still pending.
    pub account_amount: Option<String>,
    /// The rate used for that conversion, as a decimal string; `None` in the
    /// same cases as `account_amount`.
    pub fx_rate: Option<String>,
    /// Id of the transaction's own currency (a fiat `assets` row), rendered as
    /// a string for the same reason as `id`. May differ from the account's
    /// default currency; used to pre-select the edit form's currency field.
    pub asset_id: String,
    /// Alphabetic code of the transaction's own currency, which may differ
    /// from the account's default currency.
    pub asset_code: String,
    /// Booking date as an ISO `YYYY-MM-DD` string.
    pub booking_date: String,
    /// Value date as an ISO `YYYY-MM-DD` string, if one is recorded.
    pub value_date: Option<String>,
    /// Id of the transaction's category, rendered as a string, if it is
    /// categorised. Used to pre-select the edit form's category field.
    pub category_id: Option<String>,
    /// The category name, if the transaction is categorised and the category is
    /// still active.
    pub category_name: Option<String>,
    /// Id of the transaction's merchant, rendered as a string, if it has one.
    /// Used to pre-select the edit form's merchant field.
    pub merchant_id: Option<String>,
    /// The merchant name, if the transaction has one and the merchant is still
    /// active.
    pub merchant_name: Option<String>,
    /// Id of the transfer this transaction is a leg of, if any. When set, the
    /// row is shown as a transfer (the transaction is still edited and deleted
    /// on its own).
    pub transfer_id: Option<String>,
    /// Name of the account on the other leg of that transfer, for display.
    pub transfer_counterparty: Option<String>,
}

/// One page of a transaction list, plus the opaque cursor to fetch the next
/// page (absent when the last row has been reached).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionPage {
    pub transactions: Vec<TransactionDto>,
    pub next_cursor: Option<String>,
}

/// One row a transaction import (CSV or PDF statement) could not insert, and
/// why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportRowError {
    /// 1-based. For the CSV import this counts the header as row 1 (so the
    /// first data row is 2), matching what a spreadsheet program shows; for
    /// a PDF statement import it numbers the extracted rows sequentially.
    pub row_number: usize,
    pub reason: String,
}

/// The outcome of a transaction import (CSV or PDF statement): rows
/// inserted, plus every row that was rejected, in file order. A structurally
/// broken file (no usable header/layout) is a hard error instead -- this
/// only reports per-row outcomes for a file that was readable at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportSummary {
    pub imported: usize,
    pub rejected: Vec<ImportRowError>,
}

/// One row extracted from an uploaded bank-statement PDF, awaiting review.
/// `description` is the bank's raw text for that row, shown only as a
/// read-only hint in the review UI -- it is never sent back to
/// `confirm_statement_import` and is never persisted (transactions have no
/// free-text description field). `suggested_merchant_id`/`_name` are only a
/// pre-fill; the user can change or clear them before confirming.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedRowDto {
    pub row_number: usize,
    pub booking_date: String,
    pub value_date: Option<String>,
    pub amount: String,
    pub description: String,
    pub suggested_merchant_id: Option<String>,
    pub suggested_merchant_name: Option<String>,
}

/// The outcome of extracting a statement PDF: rows ready for review, plus
/// any row the parser could not understand. A structurally unrecognized file
/// is a hard error instead (see `extract_statement_pdf`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatementExtractionDto {
    pub rows: Vec<ExtractedRowDto>,
    pub rejected: Vec<ImportRowError>,
}

/// One reviewed row sent back for insertion, shaped like
/// [`crate::transactions::api::create_transaction`]'s own fields (a blank id
/// means none).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfirmedRowDto {
    pub booking_date: String,
    pub value_date: Option<String>,
    pub amount: String,
    pub category_id: String,
    pub merchant_id: String,
}
