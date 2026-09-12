//! Deblock (French neobank) statement PDF parser, built on the bank-agnostic
//! primitives in `extraction.rs`. A second bank's parser would live
//! alongside this file, reusing the same primitives with its own header
//! labels, column layout, and date/amount formatting -- nothing here is
//! shared beyond `extraction.rs` on purpose.
//!
//! Deblock statements come in at least two languages, with different header
//! labels, date formats, and number formats -- detected from which header
//! row actually matches, not assumed:
//! - French: dates spelled out with the year (`"29 avril 2024"`), amounts
//!   with a comma decimal separator (`"9,99"`).
//! - English: dates have **no year printed anywhere in the statement**
//!   (`"June 01"`) -- confirmed by inspecting the real sample's raw content
//!   stream and metadata, not assumed -- so parsing an English statement
//!   requires the caller to supply `statement_year` explicitly; amounts use a
//!   period decimal separator and an apostrophe (ASCII or typographic) as a
//!   thousands separator on larger amounts (`"1'124.61"`).
//!
//! Transaction direction (debit vs. credit) is decided only by which column
//! (by x-position) the amount was drawn in -- never by the wording of
//! `description` (e.g. "Frais"/"Virement"/"Card Payment"/"Transfer"), per
//! `src/server/CLAUDE.md`'s rule against inferring direction from a name.

use std::collections::HashMap;

use sqlx::types::chrono::NaiveDate;
use sqlx::types::BigDecimal;

use super::extraction::{self, ColumnSpec, Word};
use super::StatementImportError;
use crate::server::transaction_import::RowError;
use crate::server::transactions;

/// (printed header label, canonical column key). The canonical key is what
/// `parse_row` looks cells up by, regardless of which language matched.
const HEADER_LABELS_FR: [(&str, &str); 5] = [
    ("Date", "date"),
    ("Valeur", "value_date"),
    ("Opération", "description"),
    ("Débit", "debit"),
    ("Crédit", "credit"),
];
const HEADER_LABELS_EN: [(&str, &str); 5] = [
    ("Date", "date"),
    ("Value date", "value_date"),
    ("Transaction", "description"),
    ("Debit", "debit"),
    ("Credit", "credit"),
];

/// A word's x-position is compared to a column's header x within this
/// distance to decide which column it belongs to.
const COLUMN_X_TOLERANCE: f64 = 1.0;

const FRENCH_MONTHS: [(&str, u32); 12] = [
    ("janvier", 1),
    ("février", 2),
    ("mars", 3),
    ("avril", 4),
    ("mai", 5),
    ("juin", 6),
    ("juillet", 7),
    ("août", 8),
    ("septembre", 9),
    ("octobre", 10),
    ("novembre", 11),
    ("décembre", 12),
];

const ENGLISH_MONTHS: [(&str, u32); 12] = [
    ("January", 1),
    ("February", 2),
    ("March", 3),
    ("April", 4),
    ("May", 5),
    ("June", 6),
    ("July", 7),
    ("August", 8),
    ("September", 9),
    ("October", 10),
    ("November", 11),
    ("December", 12),
];

/// Which of the two header/format variants a statement's header row matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Locale {
    French,
    English,
}

/// One row extracted from a Deblock statement, before merchant-suggestion
/// resolution (that needs the user's merchant list, done by `mod.rs`).
pub struct DeblockRow {
    /// 1-based, sequential across the whole document (all pages).
    pub row_number: usize,
    pub booking_date: NaiveDate,
    pub value_date: NaiveDate,
    /// Signed: negative for a debit cell, positive for a credit cell.
    pub amount: BigDecimal,
    /// The raw transaction-description cell -- never persisted, shown only
    /// as a review hint.
    pub description: String,
    /// The quoted substring within `description`, or the full text if none.
    pub merchant_candidate: String,
}

pub struct DeblockParse {
    pub rows: Vec<DeblockRow>,
    pub rejected: Vec<RowError>,
}

/// Parse a French `"29 avril 2024"`-style date (day, month name, year --
/// self-contained, no external year needed). `None` on any unrecognized
/// shape or month name (no French chrono locale is configured).
fn parse_french_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month_name = parts.next()?;
    let year: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let month = FRENCH_MONTHS
        .iter()
        .find(|(name, _)| *name == month_name)?
        .1;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Parse an English `"June 01"`-style date (month name, day) using the
/// caller-supplied `year` -- this statement format never prints a year
/// anywhere (confirmed by inspecting a real sample's raw content and
/// metadata), so it cannot be recovered from the file itself.
fn parse_english_date(text: &str, year: i32) -> Option<NaiveDate> {
    let mut parts = text.split_whitespace();
    let month_name = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let month = ENGLISH_MONTHS
        .iter()
        .find(|(name, _)| *name == month_name)?
        .1;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// `"9,99"` -> `"9.99"` (French comma decimal separator -> period).
fn normalize_amount_fr(text: &str) -> String {
    text.trim().replace(',', ".")
}

/// `"1'124.61"` -> `"1124.61"`. English statements already use a period
/// decimal separator; larger amounts group thousands with an apostrophe --
/// either the ASCII `'` or the typographic `’` observed in a real
/// statement -- which is simply removed.
fn normalize_amount_en(text: &str) -> String {
    text.trim().replace(['\'', '\u{2019}'], "")
}

/// Decodes the handful of standard XML/HTML entities Deblock's own PDF
/// generator sometimes leaves un-decoded in transaction descriptions (e.g.
/// `"McDonald&apos;s"`, confirmed in a real statement) -- not a general
/// entity decoder, just these five fixed, unambiguous replacements. `&amp;`
/// is decoded last so it can't turn a literal `&lt;` into a second-stage
/// entity.
fn decode_entities(text: &str) -> String {
    text.replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The substring between the first pair of `"..."` quotes in `description`,
/// or the trimmed full text if it has none.
fn merchant_candidate(description: &str) -> String {
    if let Some(start) = description.find('"') {
        if let Some(end_offset) = description[start + 1..].find('"') {
            let end = start + 1 + end_offset;
            return description[start + 1..end].to_owned();
        }
    }
    description.trim().to_owned()
}

fn parse_row(
    row_number: usize,
    cells: &HashMap<&str, String>,
    locale: Locale,
    statement_year: Option<i32>,
) -> Result<DeblockRow, String> {
    let parse_date = |text: &str| -> Option<NaiveDate> {
        match locale {
            Locale::French => parse_french_date(text),
            Locale::English => statement_year.and_then(|year| parse_english_date(text, year)),
        }
    };
    let normalize_amount = |text: &str| -> String {
        match locale {
            Locale::French => normalize_amount_fr(text),
            Locale::English => normalize_amount_en(text),
        }
    };

    let booking_date =
        parse_date(cells.get("date").ok_or("missing date")?).ok_or_else(|| match locale {
            Locale::French => "unrecognized date format".to_owned(),
            Locale::English => "unrecognized date format (or missing statement year)".to_owned(),
        })?;
    let value_date = parse_date(cells.get("value_date").ok_or("missing value date")?).ok_or_else(
        || match locale {
            Locale::French => "unrecognized value-date format".to_owned(),
            Locale::English => {
                "unrecognized value-date format (or missing statement year)".to_owned()
            }
        },
    )?;
    let description = decode_entities(cells.get("description").map(String::as_str).unwrap_or(""));

    let signed = match (cells.get("debit"), cells.get("credit")) {
        (Some(_), Some(_)) => {
            return Err("amount appears in both the debit and credit columns".to_owned())
        }
        (None, None) => return Err("no amount found in the debit or credit column".to_owned()),
        (Some(text), None) => format!("-{}", normalize_amount(text)),
        (None, Some(text)) => normalize_amount(text),
    };
    let amount = transactions::validate_amount(&signed).map_err(|err| err.to_string())?;

    Ok(DeblockRow {
        row_number,
        booking_date,
        value_date,
        amount,
        merchant_candidate: merchant_candidate(&description),
        description,
    })
}

/// Locate the header row, trying the French label set first, then the
/// English one. Returns the header's index, its columns, and which locale
/// matched -- or `None` if neither did (a whole-file structural error to the
/// caller, not a per-row one).
fn locate_header(rows: &[Vec<Word>]) -> Option<(usize, Vec<ColumnSpec>, Locale)> {
    if let Some((index, columns)) = extraction::find_header_columns(rows, &HEADER_LABELS_FR) {
        return Some((index, columns, Locale::French));
    }
    let (index, columns) = extraction::find_header_columns(rows, &HEADER_LABELS_EN)?;
    Some((index, columns, Locale::English))
}

/// Parse the rows beneath an already-located header, assigning each a
/// sequential row number starting from `*next_row_number` (advanced past
/// however many rows were found). Pure and independent of PDF bytes, so it's
/// directly unit-testable with fabricated `Word` fixtures.
///
/// `statement_year` is only consulted for an English-language statement
/// (French dates carry their own year); if the header turns out to be the
/// English variant and no year was supplied, every row on this page is
/// rejected with a message asking for it, rather than guessing one.
fn parse_words(
    rows: &[Vec<Word>],
    statement_year: Option<i32>,
    next_row_number: &mut usize,
) -> Result<DeblockParse, StatementImportError> {
    let (header_index, columns, locale) =
        locate_header(rows).ok_or(StatementImportError::InvalidInput(
            "this doesn't look like a Deblock statement (expected header row not found)",
        ))?;

    let mut result = DeblockParse {
        rows: Vec::new(),
        rejected: Vec::new(),
    };
    for row in &rows[header_index + 1..] {
        let row_number = *next_row_number;
        *next_row_number += 1;
        let cells = extraction::bucket_row_by_columns(row, &columns, COLUMN_X_TOLERANCE);
        match parse_row(row_number, &cells, locale, statement_year) {
            Ok(parsed) => result.rows.push(parsed),
            Err(reason) => result.rejected.push(RowError { row_number, reason }),
        }
    }
    Ok(result)
}

/// Parse `bytes` (a whole PDF file) as a Deblock statement, in either the
/// French or English layout (detected per page from the header row).
/// `statement_year` is required only for an English-language statement,
/// whose dates never print a year (see the module docs); it's ignored for a
/// French one. Row numbers are sequential across the whole document, not
/// reset per page. The header row is re-located on every page
/// independently; a page where it can't be found fails the whole import.
pub fn parse(
    bytes: &[u8],
    statement_year: Option<i32>,
) -> Result<DeblockParse, StatementImportError> {
    let words = extraction::extract_words(bytes)?;

    let mut by_page: Vec<(u32, Vec<Word>)> = Vec::new();
    for word in words {
        match by_page.iter_mut().find(|(page, _)| *page == word.page) {
            Some((_, page_words)) => page_words.push(word),
            None => by_page.push((word.page, vec![word])),
        }
    }

    let mut combined = DeblockParse {
        rows: Vec::new(),
        rejected: Vec::new(),
    };
    let mut next_row_number = 1usize;
    for (_, page_words) in by_page {
        let rows = extraction::group_into_rows(page_words, extraction::ROW_Y_TOLERANCE);
        let page_result = parse_words(&rows, statement_year, &mut next_row_number)?;
        combined.rows.extend(page_result.rows);
        combined.rejected.extend(page_result.rejected);
    }
    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::dec;

    fn word(x: f64, y: f64, text: &str) -> Word {
        Word {
            page: 0,
            x,
            y,
            text: text.to_owned(),
        }
    }

    #[test]
    fn parse_french_date_accepts_a_known_month_and_rejects_unknown_ones() {
        assert_eq!(
            parse_french_date("29 avril 2024"),
            NaiveDate::from_ymd_opt(2024, 4, 29)
        );
        assert_eq!(parse_french_date("29 fevrier 2024"), None); // no accent -> unrecognized
        assert_eq!(parse_french_date("garbage"), None);
        assert_eq!(parse_french_date("29 avril 2024 extra"), None);
    }

    #[test]
    fn parse_english_date_uses_the_supplied_year_and_rejects_unknown_months() {
        assert_eq!(
            parse_english_date("June 01", 2025),
            NaiveDate::from_ymd_opt(2025, 6, 1)
        );
        assert_eq!(parse_english_date("Junuary 01", 2025), None);
        assert_eq!(parse_english_date("garbage", 2025), None);
    }

    #[test]
    fn normalize_amount_fr_converts_comma_to_period() {
        assert_eq!(normalize_amount_fr("9,99"), "9.99");
        assert_eq!(normalize_amount_fr(" 50,00 "), "50.00");
    }

    #[test]
    fn normalize_amount_en_strips_thousands_separators() {
        assert_eq!(normalize_amount_en("5.80"), "5.80");
        assert_eq!(normalize_amount_en("1'124.61"), "1124.61");
        assert_eq!(normalize_amount_en("1\u{2019}124.61"), "1124.61");
    }

    #[test]
    fn decode_entities_replaces_the_five_standard_entities() {
        assert_eq!(decode_entities("McDonald&apos;s"), "McDonald's");
        assert_eq!(decode_entities("Fish &amp; Chips"), "Fish & Chips");
        assert_eq!(decode_entities("no entities here"), "no entities here");
    }

    #[test]
    fn merchant_candidate_extracts_the_first_quoted_substring_or_falls_back() {
        assert_eq!(
            merchant_candidate("Virement \"MR JOHN DOE\""),
            "MR JOHN DOE"
        );
        assert_eq!(
            merchant_candidate("Frais \"Card delivery\""),
            "Card delivery"
        );
        assert_eq!(merchant_candidate("No quotes here"), "No quotes here");
    }

    #[test]
    fn parse_row_rejects_both_columns_and_neither_column() {
        let mut both = HashMap::new();
        both.insert("date", "29 avril 2024".to_owned());
        both.insert("value_date", "29 avril 2024".to_owned());
        both.insert("debit", "9,99".to_owned());
        both.insert("credit", "50,00".to_owned());
        assert!(parse_row(1, &both, Locale::French, None).is_err());

        let mut neither = HashMap::new();
        neither.insert("date", "29 avril 2024".to_owned());
        neither.insert("value_date", "29 avril 2024".to_owned());
        assert!(parse_row(1, &neither, Locale::French, None).is_err());
    }

    #[test]
    fn parse_row_produces_a_signed_amount_from_column_membership_alone() {
        let mut credit = HashMap::new();
        credit.insert("date", "29 avril 2024".to_owned());
        credit.insert("value_date", "29 avril 2024".to_owned());
        credit.insert("description", "Virement \"MR JOHN DOE\"".to_owned());
        credit.insert("credit", "50,00".to_owned());
        let row = parse_row(1, &credit, Locale::French, None).expect("valid credit row");
        assert_eq!(row.amount, dec("50.00"));
        assert_eq!(row.merchant_candidate, "MR JOHN DOE");

        let mut debit = HashMap::new();
        debit.insert("date", "29 avril 2024".to_owned());
        debit.insert("value_date", "29 avril 2024".to_owned());
        debit.insert("description", "Frais \"Card delivery\"".to_owned());
        debit.insert("debit", "9,99".to_owned());
        let row = parse_row(2, &debit, Locale::French, None).expect("valid debit row");
        assert_eq!(row.amount, dec("-9.99"));
    }

    #[test]
    fn parse_row_english_requires_the_statement_year() {
        let mut row = HashMap::new();
        row.insert("date".to_owned(), "June 01".to_owned());
        row.insert("value_date".to_owned(), "May 30".to_owned());
        row.insert(
            "description".to_owned(),
            "Card Payment \"Ugc Astoria\"".to_owned(),
        );
        row.insert("debit".to_owned(), "5.80".to_owned());
        let row: HashMap<&str, String> = row.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();

        assert!(parse_row(1, &row, Locale::English, None).is_err());
        let parsed = parse_row(1, &row, Locale::English, Some(2025)).expect("valid with year");
        assert_eq!(
            parsed.booking_date,
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()
        );
        assert_eq!(
            parsed.value_date,
            NaiveDate::from_ymd_opt(2025, 5, 30).unwrap()
        );
        assert_eq!(parsed.amount, dec("-5.80"));
        assert_eq!(parsed.merchant_candidate, "Ugc Astoria");
    }

    #[test]
    fn parse_words_reproduces_the_french_samples_shape() {
        let header = vec![
            word(66.0, 577.5, "Date"),
            word(145.65, 577.5, "Valeur"),
            word(225.3, 577.5, "Opération"),
            word(384.6, 577.5, "Débit"),
            word(424.425, 577.5, "Crédit"),
        ];
        let credit_row = vec![
            word(66.0, 556.5, "29 avril 2024"),
            word(145.65, 556.5, "29 avril 2024"),
            word(225.3, 556.5, "Virement \"MR JOHN DOE\""),
            word(424.425, 556.5, "50,00"),
        ];
        let debit_row = vec![
            word(66.0, 535.5, "29 avril 2024"),
            word(145.65, 535.5, "29 avril 2024"),
            word(225.3, 535.5, "Frais \"Card delivery\""),
            word(384.6, 535.5, "9,99"),
        ];
        let rows = vec![header, credit_row, debit_row];

        let mut next_row_number = 1usize;
        let parsed = parse_words(&rows, None, &mut next_row_number).expect("recognized header");
        assert_eq!(parsed.rejected.len(), 0);
        assert_eq!(parsed.rows.len(), 2);
        assert!(parsed.rows[0].amount > BigDecimal::from(0));
        assert!(parsed.rows[1].amount < BigDecimal::from(0));
    }

    #[test]
    fn parse_words_reproduces_the_english_samples_shape() {
        let header = vec![
            word(66.0, 264.375, "Date"),
            word(145.65, 264.375, "Value date"),
            word(225.3, 264.375, "Transaction"),
            word(384.6, 264.375, "Debit"),
            word(424.425, 264.375, "Credit"),
        ];
        let debit_row = vec![
            word(66.0, 285.3375, "June 01"),
            word(145.65, 285.3375, "May 30"),
            word(225.3, 285.3375, "Card Payment \"Ugc Astoria\""),
            word(384.6, 285.3375, "5.80"),
        ];
        let credit_row = vec![
            word(66.0, 390.15, "June 03"),
            word(145.65, 390.15, "June 03"),
            word(225.3, 390.15, "Transfer \"MR JOHN DOE\""),
            word(424.425, 390.15, "100.00"),
        ];
        let rows = vec![header, debit_row, credit_row];

        // No year supplied: every row rejected, none dropped silently.
        let mut next_row_number = 1usize;
        let without_year =
            parse_words(&rows, None, &mut next_row_number).expect("header recognized");
        assert_eq!(without_year.rows.len(), 0);
        assert_eq!(without_year.rejected.len(), 2);

        let mut next_row_number = 1usize;
        let with_year =
            parse_words(&rows, Some(2025), &mut next_row_number).expect("header recognized");
        assert_eq!(with_year.rejected.len(), 0);
        assert_eq!(with_year.rows.len(), 2);
        assert!(with_year.rows[0].amount < BigDecimal::from(0));
        assert!(with_year.rows[1].amount > BigDecimal::from(0));
    }
}
