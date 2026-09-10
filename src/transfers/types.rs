//! DTOs for the transfer server functions. Compiled for both targets, so no
//! server-only types. Ids cross the wire as strings, like the rest of the app.

use serde::{Deserialize, Serialize};

/// One transaction the edit form's "Paired transaction" picker can offer: a
/// candidate to link this transaction to, or the one it is already linked to.
/// `label` is a ready-to-display summary built on the server (date, account,
/// signed amount, currency).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkCandidate {
    pub transaction_id: String,
    pub label: String,
}

/// The choices for the edit form's "Paired transaction" field for one
/// transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferLinkOptions {
    /// The transaction currently linked to this one, if it is already part of a
    /// transfer. Shown selected, and not repeated in `candidates`.
    pub current_pair: Option<LinkCandidate>,
    /// The transactions this one could be paired with: a different account, the
    /// opposite amount sign, not already part of a transfer.
    pub candidates: Vec<LinkCandidate>,
}
