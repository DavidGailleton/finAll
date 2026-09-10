//! Income vs expense over an explicit period, grouped by category.
//!
//! For an inclusive `[from, to]` booking-date window, the signed sum of every
//! non-transfer transaction is aggregated per `(category, currency)` and valued
//! into a chosen display currency **at the exchange rate in force on `to`**
//! (via [`crate::server::assets::rates::resolve_rate_as_of`]). The per-currency
//! parts of one category are rolled into a single net, and each group is placed
//! on the income or expense side by the **sign of that net** — the category's
//! own `kind` is only a label. Netting is uniform: an income category that also
//! carries a negative transaction shows the signed result, and if that result
//! is negative the group lands under expenses.
//!
//! A `(category, currency)` subtotal with no as-of rate is kept visible without
//! a converted value; the report is then marked incomplete and that part is
//! left out of the totals — nothing is silently combined or dropped.
//!
//! Every query is scoped by `user_id`.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, RoundingMode, Signed, Zero};
use leptos::logging;
use sqlx::types::chrono::NaiveDate;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::categories::types::CategoryKind;
use crate::server::assets::conversion::{self, ConversionError};
use crate::server::assets::currency::{self, CurrencyError};
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::assets::rates::{self, RateResolutionError, ResolvedRate};

/// Rounding applied when a `(category, currency)` subtotal is converted into the
/// display currency, to that currency's minor-unit precision (inside
/// [`conversion::convert`]). Deliberately the same policy as
/// `crate::server::balances` and `crate::server::net_worth` — banker's rounding.
const INCOME_EXPENSE_ROUNDING: RoundingMode = RoundingMode::HalfEven;

#[derive(Debug, thiserror::Error)]
pub enum IncomeExpenseError {
    #[error("you must be signed in to do this")]
    Unauthorized,

    #[error("{0}")]
    InvalidInput(&'static str),

    #[error("that display currency does not exist")]
    CurrencyNotFound,

    #[error("something went wrong")]
    Internal,
}

impl From<sqlx::Error> for IncomeExpenseError {
    fn from(err: sqlx::Error) -> Self {
        logging::error!("income vs expense: database error: {err}");
        IncomeExpenseError::Internal
    }
}

impl From<CurrencyError> for IncomeExpenseError {
    fn from(err: CurrencyError) -> Self {
        match err {
            CurrencyError::InvalidInput(message) => IncomeExpenseError::InvalidInput(message),
            CurrencyError::NotFound => IncomeExpenseError::CurrencyNotFound,
            _ => IncomeExpenseError::Internal,
        }
    }
}

impl From<ConversionError> for IncomeExpenseError {
    fn from(err: ConversionError) -> Self {
        // `convert` only rejects caller-supplied inputs; here every one of them
        // is server-controlled, so a rejection is a bug, not user error.
        logging::error!("income vs expense: conversion rejected a server-built input: {err}");
        IncomeExpenseError::Internal
    }
}

// `RateResolutionError` is intentionally not given a `From` impl: the
// orchestrator matches on it so that `Unavailable` degrades to an unvalued part
// while `Internal` fails the request.

/// The currency the report is expressed in.
pub struct DisplayCurrency {
    pub asset_id: Uuid,
    pub alphabetic_code: String,
    pub minor_units: i16,
}

/// One row of the aggregation: a category's signed net in one currency, exact
/// and unconverted. `kind == None` iff `category_id == None` (uncategorised).
pub struct CategorySubtotal {
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub category_deleted: bool,
    pub kind: Option<CategoryKind>,
    pub asset_id: Uuid,
    pub currency_code: String,
    pub subtotal: BigDecimal,
}

/// One currency's contribution to a group, valued into the display currency.
pub struct CurrencyPart {
    pub currency_code: String,
    pub amount: BigDecimal,
    pub converted_amount: Option<BigDecimal>,
    pub rate: Option<BigDecimal>,
}

/// One group (a category across all its currencies, or the uncategorised
/// bucket), before it is rendered to strings for the DTO.
pub struct GroupLine {
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub category_kind: Option<CategoryKind>,
    pub category_deleted: bool,
    pub converted_net: Option<BigDecimal>,
    pub complete: bool,
    pub currencies: Vec<CurrencyPart>,
}

/// The outcome of folding and placing every group.
pub struct Summary {
    pub income_lines: Vec<GroupLine>,
    pub expense_lines: Vec<GroupLine>,
    pub unvalued_lines: Vec<GroupLine>,
    pub total_income: BigDecimal,
    pub total_expense: BigDecimal,
    pub net: BigDecimal,
    pub complete: bool,
}

/// The whole report: the period, the display currency, and the placed groups.
pub struct Report {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub display: DisplayCurrency,
    pub income_lines: Vec<GroupLine>,
    pub expense_lines: Vec<GroupLine>,
    pub unvalued_lines: Vec<GroupLine>,
    pub total_income: BigDecimal,
    pub total_expense: BigDecimal,
    pub net: BigDecimal,
    pub complete: bool,
}

/// Parse the required, inclusive `[from, to]` period. Both bounds must be
/// present, `YYYY-MM-DD`, and `from <= to`.
///
/// This is deliberately not [`crate::server::transactions::validate_date_range`]
/// (that returns `TransactionError` and treats a blank bound as "unbounded on
/// that side"; here both bounds are mandatory).
pub fn parse_period(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), IncomeExpenseError> {
    fn one(value: &str, missing: &'static str) -> Result<NaiveDate, IncomeExpenseError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(IncomeExpenseError::InvalidInput(missing));
        }
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| IncomeExpenseError::InvalidInput("a period date is invalid"))
    }

    let from = one(from, "a start date is required")?;
    let to = one(to, "an end date is required")?;
    if from > to {
        return Err(IncomeExpenseError::InvalidInput(
            "the start date is after the end date",
        ));
    }
    Ok((from, to))
}

/// Which side of the report a group's net lands on.
enum Side {
    Income,
    Expense,
    /// Net is exactly zero with nothing left unvalued — nothing to show.
    Omit,
    /// No sign could be determined (every currency part unvalued, more than one
    /// currency).
    Unvalued,
}

/// Place a group by the sign of its net, falling back to the single native
/// currency's sign when nothing could be valued.
fn side_of(converted_net: Option<&BigDecimal>, currencies: &[CurrencyPart]) -> Side {
    if let Some(net) = converted_net {
        if net.is_positive() {
            return Side::Income;
        }
        if net.is_negative() {
            return Side::Expense;
        }
        // net is exactly zero.
        let fully_valued = currencies
            .iter()
            .all(|part| part.converted_amount.is_some());
        if fully_valued {
            return Side::Omit;
        }
    }

    // No usable net: fall back to the native sign only when the group holds a
    // single currency (mixing currencies to get a sign is not allowed).
    match currencies {
        [only] if only.amount.is_positive() => Side::Income,
        [only] if only.amount.is_negative() => Side::Expense,
        [_] => Side::Omit,
        _ => Side::Unvalued,
    }
}

/// Fold the per-`(category, currency)` `subtotals` into per-group lines, value
/// each currency part into `display` with the rate already resolved in
/// `resolved`, roll the parts into a group net, and place each group by the
/// sign of that net.
///
/// A part already in the display currency contributes its exact value,
/// unrounded. Any other part is converted with the rate for its `asset_id`,
/// rounded to `display.minor_units` with [`INCOME_EXPENSE_ROUNDING`]. A part
/// with no entry in `resolved` is left unvalued (`converted_amount = None`),
/// flips the group's and the report's `complete` to `false`, and contributes
/// nothing to any net or total. Groups keep the order in which `subtotals`
/// first mentions them.
pub fn summarise(
    subtotals: &[CategorySubtotal],
    display: &DisplayCurrency,
    resolved: &HashMap<Uuid, ResolvedRate>,
) -> Result<Summary, IncomeExpenseError> {
    struct Accum {
        category_id: Option<Uuid>,
        category_name: Option<String>,
        category_kind: Option<CategoryKind>,
        category_deleted: bool,
        currencies: Vec<CurrencyPart>,
    }

    let mut groups: Vec<Accum> = Vec::new();
    let mut index: HashMap<Option<Uuid>, usize> = HashMap::new();
    let mut complete = true;

    for subtotal in subtotals {
        let (converted_amount, rate) = if subtotal.asset_id == display.asset_id {
            (Some(subtotal.subtotal.clone()), None)
        } else if let Some(resolved_rate) = resolved.get(&subtotal.asset_id) {
            let conversion = conversion::convert(
                &subtotal.subtotal,
                resolved_rate.rate.clone(),
                subtotal.asset_id,
                display.asset_id,
                resolved_rate.valuation_timestamp,
                display.minor_units,
                INCOME_EXPENSE_ROUNDING,
            )?;
            (
                Some(conversion.converted_amount),
                Some(resolved_rate.rate.clone()),
            )
        } else {
            complete = false;
            (None, None)
        };

        let part = CurrencyPart {
            currency_code: subtotal.currency_code.clone(),
            amount: subtotal.subtotal.clone(),
            converted_amount,
            rate,
        };

        let idx = *index.entry(subtotal.category_id).or_insert_with(|| {
            groups.push(Accum {
                category_id: subtotal.category_id,
                category_name: subtotal.category_name.clone(),
                category_kind: subtotal.kind,
                category_deleted: subtotal.category_deleted,
                currencies: Vec::new(),
            });
            groups.len() - 1
        });
        groups[idx].currencies.push(part);
    }

    let mut summary = Summary {
        income_lines: Vec::new(),
        expense_lines: Vec::new(),
        unvalued_lines: Vec::new(),
        total_income: BigDecimal::zero(),
        total_expense: BigDecimal::zero(),
        net: BigDecimal::zero(),
        complete,
    };

    for group in groups {
        let group_complete = group
            .currencies
            .iter()
            .all(|part| part.converted_amount.is_some());

        let converted_net = {
            let mut valued = group
                .currencies
                .iter()
                .filter_map(|part| part.converted_amount.as_ref())
                .peekable();
            if valued.peek().is_none() {
                None
            } else {
                let mut acc = BigDecimal::zero();
                for amount in valued {
                    acc += amount;
                }
                Some(acc)
            }
        };

        let line = GroupLine {
            category_id: group.category_id,
            category_name: group.category_name,
            category_kind: group.category_kind,
            category_deleted: group.category_deleted,
            converted_net: converted_net.clone(),
            complete: group_complete,
            currencies: group.currencies,
        };

        match side_of(converted_net.as_ref(), &line.currencies) {
            Side::Income => {
                if let Some(net) = &line.converted_net {
                    summary.total_income += net;
                }
                summary.income_lines.push(line);
            }
            Side::Expense => {
                if let Some(net) = &line.converted_net {
                    summary.total_expense += net;
                }
                summary.expense_lines.push(line);
            }
            Side::Unvalued => summary.unvalued_lines.push(line),
            Side::Omit => {}
        }
    }

    summary.net = &summary.total_income + &summary.total_expense;
    Ok(summary)
}

/// The signed-in user's income vs expense over `[from, to]`, in
/// `display_currency_code`, valued at the rate in force on `to`.
pub async fn report(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    display_currency_code: &str,
    from: &str,
    to: &str,
) -> Result<Report, IncomeExpenseError> {
    let code = currency::validate_alphabetic_code(display_currency_code)?;
    let (from, to) = parse_period(from, to)?;
    let display = display_currency(pool, &code).await?;
    let subtotals = subtotals(pool, user_id, from, to).await?;

    // Every conversion is anchored to the period-end date: the rate in force on
    // `to` (Frankfurter carries a non-trading day forward to the last one).
    let mut resolved: HashMap<Uuid, ResolvedRate> = HashMap::new();
    for subtotal in &subtotals {
        if subtotal.asset_id == display.asset_id || resolved.contains_key(&subtotal.asset_id) {
            continue;
        }
        match rates::resolve_rate_as_of(
            cache,
            &subtotal.currency_code,
            &display.alphabetic_code,
            to,
        )
        .await
        {
            Ok(rate) => {
                resolved.insert(subtotal.asset_id, rate);
            }
            // Left out on purpose: `summarise` renders an unvalued part for this
            // currency and marks the report incomplete.
            Err(RateResolutionError::Unavailable) => {}
            Err(RateResolutionError::Internal) => return Err(IncomeExpenseError::Internal),
        }
    }

    let summary = summarise(&subtotals, &display, &resolved)?;

    Ok(Report {
        from,
        to,
        display,
        income_lines: summary.income_lines,
        expense_lines: summary.expense_lines,
        unvalued_lines: summary.unvalued_lines,
        total_income: summary.total_income,
        total_expense: summary.total_expense,
        net: summary.net,
        complete: summary.complete,
    })
}

/// The `assets` / `fiat_assets` detail for one active currency, by code.
///
/// Same query as `crate::server::net_worth`'s helper of the same name; a shared
/// helper in `crate::server::assets::currency` would remove the duplication but
/// is a refactor beyond this change.
async fn display_currency(
    pool: &PgPool,
    code: &str,
) -> Result<DisplayCurrency, IncomeExpenseError> {
    let record = sqlx::query!(
        r#"
        SELECT
            a.id,
            a.code AS alphabetic_code,
            f.minor_units AS "minor_units!"
        FROM assets AS a
        INNER JOIN fiat_assets AS f ON f.asset_id = a.id
        WHERE
            a.asset_class = 'fiat'
            AND a.is_active = TRUE
            AND a.deleted_at IS NULL
            AND a.code = $1
        "#,
        code,
    )
    .fetch_optional(pool)
    .await?;

    let record = record.ok_or(IncomeExpenseError::CurrencyNotFound)?;

    Ok(DisplayCurrency {
        asset_id: record.id,
        alphabetic_code: record.alphabetic_code,
        minor_units: record.minor_units,
    })
}

/// The user's in-period, non-transfer transactions summed per
/// `(category, currency)`. A `NULL` `category_id` is the uncategorised bucket.
///
/// Transfer legs are excluded with a `NOT EXISTS` anti-join (this report needs
/// none of the transfer columns the ledger's `LEFT JOIN` fetches). `categories`
/// is joined without a `deleted_at` filter so a soft-deleted category's
/// immutable `kind` still classifies its history; the name is blanked here to
/// match the ledger.
async fn subtotals(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CategorySubtotal>, IncomeExpenseError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            t.category_id,
            c.category_name AS "category_name?",
            c.kind AS "kind?",
            (c.id IS NOT NULL AND c.deleted_at IS NOT NULL) AS "category_deleted!",
            t.asset_id,
            a.code AS "currency_code!",
            sum(t.amount) AS "subtotal!"
        FROM transactions AS t
        INNER JOIN accounts AS acc ON acc.user_id = t.user_id AND acc.id = t.account_id
        INNER JOIN assets AS a ON a.id = t.asset_id
        LEFT JOIN categories AS c ON c.user_id = t.user_id AND c.id = t.category_id
        WHERE
            t.user_id = $1
            AND t.deleted_at IS NULL
            AND acc.deleted_at IS NULL
            AND t.booking_date >= $2
            AND t.booking_date <= $3
            AND NOT EXISTS (
                SELECT 1
                FROM transfers AS tr
                WHERE
                    tr.user_id = t.user_id
                    AND (
                        tr.source_transaction_id = t.id
                        OR tr.destination_transaction_id = t.id
                    )
                    AND tr.deleted_at IS NULL
            )
        GROUP BY
            t.category_id, c.id, c.category_name, c.kind, c.deleted_at, t.asset_id, a.code
        ORDER BY c.category_name NULLS LAST, a.code
        "#,
        user_id,
        from,
        to,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let kind = match row.kind.as_deref() {
                None => None,
                Some(value) => Some(CategoryKind::from_db_str(value).ok_or_else(|| {
                    logging::error!(
                        "income vs expense: unknown category kind {value:?} in the database"
                    );
                    IncomeExpenseError::Internal
                })?),
            };

            Ok(CategorySubtotal {
                category_id: row.category_id,
                // Match the ledger: a soft-deleted category shows no name.
                category_name: if row.category_deleted {
                    None
                } else {
                    row.category_name
                },
                category_deleted: row.category_deleted,
                kind,
                asset_id: row.asset_id,
                currency_code: row.currency_code,
                subtotal: row.subtotal,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sqlx::types::chrono::{DateTime, Utc};

    use super::*;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn asset(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid fixed timestamp")
    }

    fn eur() -> DisplayCurrency {
        DisplayCurrency {
            asset_id: asset(1),
            alphabetic_code: "EUR".to_owned(),
            minor_units: 2,
        }
    }

    fn rate(value: &str) -> ResolvedRate {
        ResolvedRate {
            rate: dec(value),
            valuation_timestamp: at(1_700_000_000),
        }
    }

    fn sub(
        category: Option<u128>,
        kind: Option<CategoryKind>,
        asset_n: u128,
        code: &str,
        amount: &str,
    ) -> CategorySubtotal {
        CategorySubtotal {
            category_id: category.map(asset),
            category_name: category.map(|n| format!("Category {n}")),
            category_deleted: false,
            kind,
            asset_id: asset(asset_n),
            currency_code: code.to_owned(),
            subtotal: dec(amount),
        }
    }

    #[test]
    fn parse_period_accepts_a_valid_inclusive_range() {
        let (from, to) = parse_period("2026-01-01", "2026-01-31").expect("parses");
        assert_eq!(from, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(to, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert!(parse_period("2026-02-15", "2026-02-15").is_ok());
    }

    #[test]
    fn parse_period_requires_both_bounds() {
        assert!(matches!(
            parse_period("  ", "2026-01-31"),
            Err(IncomeExpenseError::InvalidInput("a start date is required"))
        ));
        assert!(matches!(
            parse_period("2026-01-01", ""),
            Err(IncomeExpenseError::InvalidInput("an end date is required"))
        ));
    }

    #[test]
    fn parse_period_rejects_bad_format_and_reversed_range() {
        assert!(matches!(
            parse_period("01/01/2026", "2026-01-31"),
            Err(IncomeExpenseError::InvalidInput("a period date is invalid"))
        ));
        assert!(matches!(
            parse_period("2026-02-01", "2026-01-01"),
            Err(IncomeExpenseError::InvalidInput(
                "the start date is after the end date"
            ))
        ));
    }

    #[test]
    fn no_subtotals_is_a_zero_complete_report() {
        let summary = summarise(&[], &eur(), &HashMap::new()).expect("summarises");
        assert!(summary.income_lines.is_empty());
        assert!(summary.expense_lines.is_empty());
        assert!(summary.unvalued_lines.is_empty());
        assert_eq!(summary.net, dec("0"));
        assert!(summary.complete);
    }

    #[test]
    fn a_positive_net_lands_on_the_income_side() {
        let rows = [
            sub(Some(10), Some(CategoryKind::Income), 1, "EUR", "10"),
            sub(Some(10), Some(CategoryKind::Income), 1, "EUR", "-2"),
        ];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");

        assert_eq!(summary.income_lines.len(), 1);
        assert_eq!(summary.income_lines[0].converted_net, Some(dec("8")));
        assert_eq!(summary.total_income, dec("8"));
        assert_eq!(summary.net, dec("8"));
        assert!(summary.expense_lines.is_empty());
    }

    #[test]
    fn an_income_category_that_nets_negative_lands_on_the_expense_side() {
        let rows = [sub(Some(7), Some(CategoryKind::Income), 1, "EUR", "-30")];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");

        assert_eq!(summary.expense_lines.len(), 1);
        assert_eq!(
            summary.expense_lines[0].category_kind,
            Some(CategoryKind::Income)
        );
        assert_eq!(summary.total_expense, dec("-30"));
        assert_eq!(summary.net, dec("-30"));
    }

    #[test]
    fn an_expense_category_that_nets_positive_lands_on_the_income_side() {
        let rows = [sub(Some(3), Some(CategoryKind::Expense), 1, "EUR", "50")];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");
        assert_eq!(summary.income_lines.len(), 1);
        assert_eq!(summary.total_income, dec("50"));
    }

    #[test]
    fn an_uncategorised_group_is_placed_by_its_sign() {
        let rows = [sub(None, None, 1, "EUR", "40")];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");
        assert_eq!(summary.income_lines.len(), 1);
        assert_eq!(summary.income_lines[0].category_id, None);
        assert_eq!(summary.income_lines[0].category_kind, None);
    }

    #[test]
    fn a_fully_valued_zero_net_group_is_omitted() {
        let rows = [
            sub(Some(5), Some(CategoryKind::Expense), 1, "EUR", "20"),
            sub(Some(5), Some(CategoryKind::Expense), 1, "EUR", "-20"),
        ];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");
        assert!(summary.income_lines.is_empty());
        assert!(summary.expense_lines.is_empty());
        assert!(summary.unvalued_lines.is_empty());
        assert_eq!(summary.net, dec("0"));
    }

    #[test]
    fn a_foreign_part_is_converted_and_rounded_to_minor_units() {
        let rows = [sub(Some(2), Some(CategoryKind::Income), 2, "USD", "100")];
        let resolved = HashMap::from([(asset(2), rate("1.234"))]);
        let summary = summarise(&rows, &eur(), &resolved).expect("summarises");

        // 100 * 1.234 = 123.400 -> 123.40 at 2 minor units.
        assert_eq!(summary.income_lines[0].converted_net, Some(dec("123.40")));
        assert_eq!(summary.total_income, dec("123.40"));
        assert_eq!(
            summary.income_lines[0].currencies[0].rate,
            Some(dec("1.234"))
        );
        assert!(summary.complete);
    }

    #[test]
    fn a_single_currency_group_with_no_rate_falls_back_to_its_native_sign() {
        let rows = [sub(Some(4), Some(CategoryKind::Expense), 2, "USD", "-100")];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");

        assert_eq!(summary.expense_lines.len(), 1);
        assert_eq!(summary.expense_lines[0].converted_net, None);
        assert!(!summary.expense_lines[0].complete);
        assert!(!summary.complete);
        assert_eq!(summary.total_expense, dec("0"));
    }

    #[test]
    fn a_multi_currency_group_can_be_partially_valued() {
        let rows = [
            sub(Some(6), Some(CategoryKind::Income), 1, "EUR", "120"),
            sub(Some(6), Some(CategoryKind::Income), 2, "USD", "50"),
        ];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");

        assert_eq!(summary.income_lines.len(), 1);
        assert_eq!(summary.income_lines[0].converted_net, Some(dec("120")));
        assert!(!summary.income_lines[0].complete);
        assert_eq!(summary.income_lines[0].currencies.len(), 2);
        assert_eq!(summary.total_income, dec("120"));
        assert!(!summary.complete);
    }

    #[test]
    fn a_multi_currency_group_with_nothing_valued_is_unvalued() {
        let rows = [
            sub(Some(8), Some(CategoryKind::Income), 2, "USD", "10"),
            sub(Some(8), Some(CategoryKind::Income), 3, "GBP", "-10"),
        ];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");
        assert_eq!(summary.unvalued_lines.len(), 1);
        assert!(summary.income_lines.is_empty());
        assert!(summary.expense_lines.is_empty());
        assert!(!summary.complete);
    }

    #[test]
    fn a_display_currency_part_contributes_exact_and_unrounded() {
        let rows = [sub(Some(9), Some(CategoryKind::Income), 1, "EUR", "10.005")];
        let summary = summarise(&rows, &eur(), &HashMap::new()).expect("summarises");
        assert_eq!(summary.total_income, dec("10.005"));
        assert_eq!(summary.net, dec("10.005"));
    }
}

#[cfg(test)]
mod db_tests {
    use std::sync::Arc;

    use super::*;
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::test_support::{
        create_user, currency_id, date, dec, insert_transaction, insert_transfer,
    };
    use crate::server::{accounts, categories, transactions};

    /// A rate cache with no HTTP reach: single-currency fixtures never resolve a
    /// rate, and the one cross-currency test seeds this directly.
    fn fx_cache() -> Arc<FxRateCache> {
        FxRateCache::new().expect("build fx cache")
    }

    async fn categorised_transaction(
        pool: &PgPool,
        user_id: Uuid,
        account_id: Uuid,
        asset_id: Uuid,
        category_id: Uuid,
        amount: &str,
        booking_date: NaiveDate,
    ) {
        transactions::create(
            pool,
            user_id,
            account_id,
            &transactions::TransactionWrite {
                asset_id,
                category_id: Some(category_id),
                merchant_id: None,
                amount: dec(amount),
                booking_date,
                value_date: None,
            },
        )
        .await
        .expect("create categorised transaction");
    }

    #[sqlx::test]
    async fn nets_within_one_category_and_places_it_by_sign(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let salary = categories::create(&pool, alice, "Salary", "income")
            .await
            .expect("category");

        categorised_transaction(
            &pool,
            alice,
            account.id,
            eur,
            salary.id,
            "1000",
            date(2026, 1, 5),
        )
        .await;
        categorised_transaction(
            &pool,
            alice,
            account.id,
            eur,
            salary.id,
            "-200",
            date(2026, 1, 6),
        )
        .await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");

        assert_eq!(report.income_lines.len(), 1);
        assert_eq!(report.income_lines[0].converted_net, Some(dec("800")));
        assert_eq!(report.total_income, dec("800"));
        assert_eq!(report.net, dec("800"));
        assert!(report.complete);
    }

    #[sqlx::test]
    async fn an_income_category_that_nets_negative_shows_under_expenses(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let refunds = categories::create(&pool, alice, "Refunds", "income")
            .await
            .expect("category");

        categorised_transaction(
            &pool,
            alice,
            account.id,
            eur,
            refunds.id,
            "10",
            date(2026, 3, 2),
        )
        .await;
        categorised_transaction(
            &pool,
            alice,
            account.id,
            eur,
            refunds.id,
            "-40",
            date(2026, 3, 3),
        )
        .await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-03-01", "2026-03-31")
            .await
            .expect("report");

        assert!(report.income_lines.is_empty());
        assert_eq!(report.expense_lines.len(), 1);
        assert_eq!(
            report.expense_lines[0].category_kind,
            Some(CategoryKind::Income)
        );
        assert_eq!(report.total_expense, dec("-30"));
    }

    #[sqlx::test]
    async fn the_period_bounds_are_inclusive(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");

        insert_transaction(&pool, alice, account.id, eur, "5", date(2025, 12, 31)).await;
        insert_transaction(&pool, alice, account.id, eur, "10", date(2026, 1, 1)).await;
        insert_transaction(&pool, alice, account.id, eur, "20", date(2026, 1, 31)).await;
        insert_transaction(&pool, alice, account.id, eur, "40", date(2026, 2, 1)).await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");

        // Only the two in-window rows (10 + 20) count, as one uncategorised line.
        assert_eq!(report.net, dec("30"));
        assert_eq!(report.income_lines.len(), 1);
        assert_eq!(report.income_lines[0].category_id, None);
    }

    #[sqlx::test]
    async fn transfer_legs_are_excluded(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let checking = accounts::create(&pool, alice, "Checking", "bank", eur)
            .await
            .expect("account");
        let savings = accounts::create(&pool, alice, "Savings", "bank", eur)
            .await
            .expect("account");

        insert_transaction(&pool, alice, checking.id, eur, "1000", date(2026, 1, 2)).await;

        let before = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");

        let source =
            insert_transaction(&pool, alice, checking.id, eur, "-300", date(2026, 1, 10)).await;
        let destination =
            insert_transaction(&pool, alice, savings.id, eur, "300", date(2026, 1, 10)).await;
        insert_transfer(&pool, alice, source, destination).await;

        let after = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");

        assert_eq!(before.net, dec("1000"));
        assert_eq!(after.net, dec("1000"));
    }

    #[sqlx::test]
    async fn another_users_transactions_are_never_included(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let bob = create_user(&pool, "bob@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let alices = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let bobs = accounts::create(&pool, bob, "Bob Cash", "cash", eur)
            .await
            .expect("account");

        insert_transaction(&pool, alice, alices.id, eur, "100", date(2026, 1, 15)).await;
        insert_transaction(&pool, bob, bobs.id, eur, "777", date(2026, 1, 15)).await;

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");
        assert_eq!(report.net, dec("100"));
    }

    #[sqlx::test]
    async fn transactions_on_a_soft_deleted_account_are_excluded(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let open = accounts::create(&pool, alice, "Open", "cash", eur)
            .await
            .expect("account");
        let closed = accounts::create(&pool, alice, "Closed", "cash", eur)
            .await
            .expect("account");

        insert_transaction(&pool, alice, open.id, eur, "60", date(2026, 1, 5)).await;
        insert_transaction(&pool, alice, closed.id, eur, "999", date(2026, 1, 5)).await;
        accounts::soft_delete(&pool, alice, closed.id)
            .await
            .expect("soft delete");

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");
        assert_eq!(report.net, dec("60"));
    }

    #[sqlx::test]
    async fn a_soft_deleted_category_still_contributes_with_its_kind(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let eur = currency_id(&pool, "EUR").await;
        let account = accounts::create(&pool, alice, "Alice Cash", "cash", eur)
            .await
            .expect("account");
        let groceries = categories::create(&pool, alice, "Groceries", "expense")
            .await
            .expect("category");

        categorised_transaction(
            &pool,
            alice,
            account.id,
            eur,
            groceries.id,
            "-75",
            date(2026, 1, 8),
        )
        .await;
        categories::soft_delete(&pool, alice, groceries.id)
            .await
            .expect("soft delete category");

        let report = report(&pool, &fx_cache(), alice, "EUR", "2026-01-01", "2026-01-31")
            .await
            .expect("report");

        assert_eq!(report.expense_lines.len(), 1);
        let line = &report.expense_lines[0];
        assert!(line.category_deleted);
        assert_eq!(line.category_name, None);
        assert_eq!(line.category_kind, Some(CategoryKind::Expense));
        assert_eq!(report.total_expense, dec("-75"));
    }

    #[sqlx::test]
    async fn an_unknown_display_currency_is_rejected(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let denied = report(&pool, &fx_cache(), alice, "ZZZ", "2026-01-01", "2026-01-31").await;
        assert!(matches!(denied, Err(IncomeExpenseError::CurrencyNotFound)));
    }

    #[sqlx::test]
    async fn a_reversed_period_is_rejected(pool: PgPool) {
        let alice = create_user(&pool, "alice@example.test").await;
        let denied = report(&pool, &fx_cache(), alice, "EUR", "2026-02-01", "2026-01-01").await;
        assert!(matches!(
            denied,
            Err(IncomeExpenseError::InvalidInput(
                "the start date is after the end date"
            ))
        ));
    }

    #[sqlx::test]
    async fn a_foreign_currency_needs_an_as_of_rate(pool: PgPool) {
        use sqlx::types::chrono::{DateTime, Utc};

        let alice = create_user(&pool, "alice@example.test").await;
        let usd = currency_id(&pool, "USD").await;
        let account = accounts::create(&pool, alice, "USD Account", "bank", usd)
            .await
            .expect("account");
        let travel = categories::create(&pool, alice, "Travel", "expense")
            .await
            .expect("category");

        categorised_transaction(
            &pool,
            alice,
            account.id,
            usd,
            travel.id,
            "-50",
            date(2026, 6, 10),
        )
        .await;

        // Nothing cached or fetchable for the period -> the line is unvalued and
        // the report is incomplete.
        let cache = FxRateCache::new().expect("cache");
        let incomplete = report(&pool, &cache, alice, "EUR", "2026-06-01", "2026-06-30")
            .await
            .expect("report");
        assert!(!incomplete.complete);
        assert_eq!(incomplete.expense_lines.len(), 1);
        assert_eq!(incomplete.expense_lines[0].converted_net, None);
        assert_eq!(incomplete.total_expense, dec("0"));

        // Seed the direct USD -> EUR rate for the period-end date.
        let period_end = date(2026, 6, 30);
        let as_of = DateTime::<Utc>::from_naive_utc_and_offset(
            date(2026, 6, 1).and_hms_opt(0, 0, 0).unwrap(),
            Utc,
        );
        cache.seed("USD", period_end, as_of, &[("EUR", "0.8")]);

        let complete = report(&pool, &cache, alice, "EUR", "2026-06-01", "2026-06-30")
            .await
            .expect("report");
        assert!(complete.complete);
        // -50 USD * 0.8 EUR/USD -> -40.00 EUR.
        assert_eq!(complete.expense_lines[0].converted_net, Some(dec("-40.00")));
        assert_eq!(complete.total_expense, dec("-40.00"));
    }
}
