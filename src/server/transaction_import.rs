//! Parses a CSV upload into validated transaction rows and inserts each one
//! through [`transactions::create`], exactly like a manually entered
//! transaction. A structurally broken file (no usable header, missing a
//! required column) fails the whole import; a bad individual row is skipped
//! and reported rather than aborting the rest -- CSV content is untrusted,
//! external data (see `src/server/CLAUDE.md`), so no row-matching, dedup, or
//! fuzzy-name logic is invented here.

use std::collections::HashMap;

use leptos::logging;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::server::accounts::{self, AccountError};
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::categories;
use crate::server::merchants;
use crate::server::transactions::{self, TransactionWrite};

#[derive(Debug, thiserror::Error)]
pub enum TransactionImportError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("account not found")]
    NotFound,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for TransactionImportError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("transaction import: database error: {err}");
        TransactionImportError::Internal
    }
}

/// One CSV row that could not be imported, and why.
pub struct RowError {
    pub row_number: usize,
    pub reason: String,
}

/// The outcome of a CSV import: rows inserted, plus every row that was
/// rejected, in file order.
pub struct ImportOutcome {
    pub imported: usize,
    pub rejected: Vec<RowError>,
}

/// Build a case-insensitive `name -> id` lookup. On a case-only collision
/// (the schema's own uniqueness is case-sensitive) the first entry wins --
/// a documented, deterministic tie-break, not an invented merge rule.
fn name_index(names_and_ids: impl Iterator<Item = (String, Uuid)>) -> HashMap<String, Uuid> {
    let mut index = HashMap::new();
    for (name, id) in names_and_ids {
        index.entry(name.to_lowercase()).or_insert(id);
    }
    index
}

/// The CSV columns this import understands, resolved once from the header.
struct Columns {
    date: usize,
    amount: usize,
    category: Option<usize>,
    merchant: Option<usize>,
}

/// Resolve the required/optional column positions by header name
/// (case-insensitive). Fails only if a *required* column is entirely absent.
fn resolve_columns(headers: &csv::StringRecord) -> Result<Columns, TransactionImportError> {
    let mut index = HashMap::new();
    for (position, name) in headers.iter().enumerate() {
        index.insert(name.to_lowercase(), position);
    }
    let date = *index
        .get("date")
        .ok_or(TransactionImportError::InvalidInput(
            "the CSV file must have a \"date\" column",
        ))?;
    let amount = *index
        .get("amount")
        .ok_or(TransactionImportError::InvalidInput(
            "the CSV file must have an \"amount\" column",
        ))?;
    Ok(Columns {
        date,
        amount,
        category: index.get("category").copied(),
        merchant: index.get("merchant").copied(),
    })
}

/// A column's cell value for this row, or `None` for a missing column or a
/// blank cell (the reader already trims whitespace from every field).
fn cell(record: &csv::StringRecord, index: Option<usize>) -> Option<&str> {
    index
        .and_then(|position| record.get(position))
        .filter(|value| !value.is_empty())
}

/// Validate one CSV row into a [`TransactionWrite`], or the reason it cannot
/// be imported.
fn resolve_row(
    record: &csv::StringRecord,
    columns: &Columns,
    asset_id: Uuid,
    category_index: &HashMap<String, Uuid>,
    merchant_index: &HashMap<String, Uuid>,
) -> Result<TransactionWrite, String> {
    let amount = transactions::validate_amount(record.get(columns.amount).unwrap_or(""))
        .map_err(|err| err.to_string())?;
    let booking_date = transactions::validate_booking_date(record.get(columns.date).unwrap_or(""))
        .map_err(|err| err.to_string())?;

    let category_id = match cell(record, columns.category) {
        None => None,
        Some(name) => Some(
            *category_index
                .get(&name.to_lowercase())
                .ok_or_else(|| format!("category \"{name}\" does not exist"))?,
        ),
    };
    let merchant_id = match cell(record, columns.merchant) {
        None => None,
        Some(name) => Some(
            *merchant_index
                .get(&name.to_lowercase())
                .ok_or_else(|| format!("merchant \"{name}\" does not exist"))?,
        ),
    };

    Ok(TransactionWrite {
        asset_id,
        amount,
        booking_date,
        value_date: None,
        category_id,
        merchant_id,
    })
}

/// Parse `csv_content` and insert every valid row on `account_id` through
/// [`transactions::create`], exactly like a manually entered transaction.
///
/// Returns [`TransactionImportError::NotFound`] if the account is not one of
/// this user's non-deleted accounts, and
/// [`TransactionImportError::InvalidInput`] if the file has no usable header
/// row. A bad individual row never aborts the import -- it is collected in
/// [`ImportOutcome::rejected`] instead.
pub async fn import_csv(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    account_id: Uuid,
    csv_content: &str,
) -> Result<ImportOutcome, TransactionImportError> {
    let account = accounts::find_for_user(pool, user_id, account_id)
        .await
        .map_err(|err| match err {
            AccountError::NotFound => TransactionImportError::NotFound,
            _ => TransactionImportError::Internal,
        })?;

    let category_index = name_index(
        categories::list_active_for_user(pool, user_id)
            .await
            .map_err(|_| TransactionImportError::Internal)?
            .into_iter()
            .map(|category| (category.category_name, category.id)),
    );
    let merchant_index = name_index(
        merchants::list_active_for_user(pool, user_id)
            .await
            .map_err(|_| TransactionImportError::Internal)?
            .into_iter()
            .map(|merchant| (merchant.merchant_name, merchant.id)),
    );

    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_content.as_bytes());
    let headers = reader
        .headers()
        .map_err(|_| TransactionImportError::InvalidInput("the CSV file has no header row"))?
        .clone();
    let columns = resolve_columns(&headers)?;

    let mut outcome = ImportOutcome {
        imported: 0,
        rejected: Vec::new(),
    };

    for (offset, record) in reader.records().enumerate() {
        // The header is row 1, so the first data row is row 2 -- matching
        // what a spreadsheet program shows.
        let row_number = offset + 2;
        let record = match record {
            Ok(record) => record,
            Err(_) => {
                outcome.rejected.push(RowError {
                    row_number,
                    reason: "could not read this row".to_owned(),
                });
                continue;
            }
        };

        match resolve_row(
            &record,
            &columns,
            account.default_asset_id,
            &category_index,
            &merchant_index,
        ) {
            Ok(write) => {
                match transactions::create(pool, cache, user_id, account_id, &write).await {
                    Ok(_) => outcome.imported += 1,
                    Err(err) => outcome.rejected.push(RowError {
                        row_number,
                        reason: err.to_string(),
                    }),
                }
            }
            Err(reason) => outcome.rejected.push(RowError { row_number, reason }),
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(fields: &[&str]) -> csv::StringRecord {
        csv::StringRecord::from(fields.to_vec())
    }

    fn headers(names: &[&str]) -> csv::StringRecord {
        record(names)
    }

    #[test]
    fn resolve_columns_finds_required_and_optional_by_name_case_insensitively() {
        let columns = resolve_columns(&headers(&["Date", "AMOUNT", "Category"])).unwrap();
        assert_eq!(columns.date, 0);
        assert_eq!(columns.amount, 1);
        assert_eq!(columns.category, Some(2));
        assert_eq!(columns.merchant, None);
    }

    #[test]
    fn resolve_columns_rejects_a_missing_required_column() {
        assert!(resolve_columns(&headers(&["amount"])).is_err());
        assert!(resolve_columns(&headers(&["date"])).is_err());
    }

    #[test]
    fn resolve_row_rejects_a_bad_amount_or_date() {
        let columns = resolve_columns(&headers(&["date", "amount"])).unwrap();
        let asset_id = Uuid::nil();
        let empty = HashMap::new();

        assert!(resolve_row(
            &record(&["2026-01-15", "not-a-number"]),
            &columns,
            asset_id,
            &empty,
            &empty
        )
        .is_err());
        assert!(resolve_row(
            &record(&["not-a-date", "10.00"]),
            &columns,
            asset_id,
            &empty,
            &empty
        )
        .is_err());
    }

    #[test]
    fn resolve_row_treats_a_blank_optional_cell_as_none() {
        let columns = resolve_columns(&headers(&["date", "amount", "category"])).unwrap();
        let asset_id = Uuid::nil();
        let empty = HashMap::new();

        let write = resolve_row(
            &record(&["2026-01-15", "10.00", ""]),
            &columns,
            asset_id,
            &empty,
            &empty,
        )
        .expect("valid row");
        assert_eq!(write.category_id, None);
    }

    #[test]
    fn resolve_row_matches_a_category_name_case_insensitively_and_rejects_unknown_names() {
        let columns = resolve_columns(&headers(&["date", "amount", "category"])).unwrap();
        let asset_id = Uuid::nil();
        let category_id = Uuid::parse_str("0193c0f0-1234-7abc-8def-0123456789ab").unwrap();
        let mut categories = HashMap::new();
        categories.insert("groceries".to_owned(), category_id);
        let merchants = HashMap::new();

        let write = resolve_row(
            &record(&["2026-01-15", "10.00", "GROCERIES"]),
            &columns,
            asset_id,
            &categories,
            &merchants,
        )
        .expect("matches case-insensitively");
        assert_eq!(write.category_id, Some(category_id));

        assert!(resolve_row(
            &record(&["2026-01-15", "10.00", "Unknown"]),
            &columns,
            asset_id,
            &categories,
            &merchants
        )
        .is_err());
    }
}

/// Authorization and end-to-end row outcomes. Each `#[sqlx::test]` runs
/// against its own freshly migrated database.
#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::test_support::{create_user, currency_id};

    fn fx_cache() -> Arc<FxRateCache> {
        FxRateCache::new().expect("build fx cache")
    }

    #[sqlx::test]
    async fn import_csv_inserts_valid_rows_and_reports_rejected_ones(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        categories::create(&pool, alice, "Groceries", "expense")
            .await
            .expect("category");
        let cache = fx_cache();

        let csv_content = "date,amount,category\n\
             2026-01-15,10.00,Groceries\n\
             2026-01-16,not-a-number,Groceries\n\
             2026-01-17,-5.00,Unknown Category\n";

        let outcome = import_csv(&pool, &cache, alice, account.id, csv_content)
            .await
            .expect("import runs");

        assert_eq!(outcome.imported, 1);
        assert_eq!(outcome.rejected.len(), 2);
        assert_eq!(outcome.rejected[0].row_number, 3);
        assert_eq!(outcome.rejected[1].row_number, 4);
    }

    #[sqlx::test]
    async fn import_csv_rejects_the_whole_file_without_a_usable_header(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let cache = fx_cache();

        let result = import_csv(&pool, &cache, alice, account.id, "foo,bar\n1,2\n").await;

        assert!(matches!(
            result,
            Err(TransactionImportError::InvalidInput(_))
        ));
    }

    #[sqlx::test]
    async fn import_csv_denies_an_account_that_is_not_the_callers(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let bobs_account = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("account");
        let cache = fx_cache();

        let result = import_csv(
            &pool,
            &cache,
            alice,
            bobs_account.id,
            "date,amount\n2026-01-15,10.00\n",
        )
        .await;

        assert!(matches!(result, Err(TransactionImportError::NotFound)));
    }
}
