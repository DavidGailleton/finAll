//! Bank-agnostic PDF table-extraction primitives. Turns `pdf-extract`'s
//! per-character callbacks into positioned text runs ("words"), groups those
//! into table rows by y-coordinate, and buckets each row's words into named
//! columns by x-coordinate. A per-bank parser (see `deblock.rs`) builds on
//! these three steps; nothing here assumes a particular bank's layout, so a
//! second bank's parser can reuse this file unchanged.
//!
//! `output_character` fires once per character, but `begin_word`/`end_word`
//! bound one whole text-showing operation (one `Tj`/`TJ` content-stream
//! call, confirmed by reading `pdf-extract`'s interpreter source) rather than
//! one whitespace-delimited word -- so a run may contain embedded spaces
//! (e.g. a whole "Virement "..."" table cell drawn as a single `Tj`). Row
//! grouping and column bucketing below don't assume otherwise: multiple runs
//! landing in the same column are concatenated, and a run's own embedded
//! spaces are preserved as-is.

use std::collections::HashMap;

use pdf_extract::{ColorSpace, Document, MediaBox, OutputDev, OutputError, Path, Transform};

use super::StatementImportError;

/// One positioned text run extracted from the PDF: a whole `Tj`/`TJ`
/// text-showing operation's characters, concatenated, positioned at its
/// first character's location. Never logged in full (see
/// `src/server/CLAUDE.md`: don't dump extracted statement content).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub page: u32,
    pub x: f64,
    pub y: f64,
    pub text: String,
}

/// Collects `output_character` calls into [`Word`]s using `begin_word`/
/// `end_word`/`end_line` as the run boundaries `pdf-extract`'s own
/// interpreter emits them at (see module docs).
struct WordCollector {
    page: u32,
    flip_ctm: Transform,
    current: Option<Word>,
    words: Vec<Word>,
}

impl WordCollector {
    fn new() -> Self {
        WordCollector {
            page: 0,
            flip_ctm: Transform::identity(),
            current: None,
            words: Vec::new(),
        }
    }

    fn flush_current(&mut self) {
        if let Some(word) = self.current.take() {
            if !word.text.trim().is_empty() {
                self.words.push(Word {
                    text: word.text.trim().to_owned(),
                    ..word
                });
            }
        }
    }

    fn into_words(mut self) -> Vec<Word> {
        self.flush_current();
        self.words
    }
}

impl OutputDev for WordCollector {
    fn begin_page(
        &mut self,
        page_num: u32,
        media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        self.page = page_num;
        // Flips the PDF's bottom-up y-axis so y increases top-to-bottom,
        // matching normal reading order -- same convention `pdf-extract`'s
        // own `PlainTextOutput` uses.
        self.flip_ctm = Transform::row_major(1., 0., 0., -1., 0., media_box.ury - media_box.lly);
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        self.flush_current();
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        _width: f64,
        _spacing: f64,
        _font_size: f64,
        char: &str,
    ) -> Result<(), OutputError> {
        let position = trm.post_transform(&self.flip_ctm);
        let (x, y) = (position.m31, position.m32);
        match &mut self.current {
            Some(word) => word.text.push_str(char),
            None => {
                self.current = Some(Word {
                    page: self.page,
                    x,
                    y,
                    text: char.to_owned(),
                })
            }
        }
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        self.flush_current();
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        self.flush_current();
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        self.flush_current();
        Ok(())
    }

    fn stroke(
        &mut self,
        _ctm: &Transform,
        _colorspace: &ColorSpace,
        _color: &[f64],
        _path: &Path,
    ) -> Result<(), OutputError> {
        Ok(())
    }

    fn fill(
        &mut self,
        _ctm: &Transform,
        _colorspace: &ColorSpace,
        _color: &[f64],
        _path: &Path,
    ) -> Result<(), OutputError> {
        Ok(())
    }
}

/// Load `bytes` and collect every text run on every page, in document order.
/// Fails only if the bytes aren't a readable PDF at all.
pub fn extract_words(bytes: &[u8]) -> Result<Vec<Word>, StatementImportError> {
    let doc = Document::load_mem(bytes)
        .map_err(|_| StatementImportError::InvalidInput("could not read this file as a PDF"))?;
    let mut collector = WordCollector::new();
    pdf_extract::output_doc(&doc, &mut collector)
        .map_err(|_| StatementImportError::InvalidInput("could not read this file as a PDF"))?;
    Ok(collector.into_words())
}

/// Words within this y-distance of each other are treated as the same
/// printed row (font-metric jitter, not a real line break).
pub const ROW_Y_TOLERANCE: f64 = 1.0;

/// Group `words` (already scoped to one page) into visual rows: sort by y,
/// start a new row whenever a word's y differs from its row's first word by
/// more than `y_tolerance`, then sort each row's words left-to-right by x.
pub fn group_into_rows(mut words: Vec<Word>, y_tolerance: f64) -> Vec<Vec<Word>> {
    words.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut rows: Vec<Vec<Word>> = Vec::new();
    for word in words {
        match rows.last_mut() {
            Some(row) if (word.y - row[0].y).abs() <= y_tolerance => row.push(word),
            _ => rows.push(vec![word]),
        }
    }

    for row in &mut rows {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    }
    rows
}

/// A named table column, anchored at its header cell's x-position.
pub struct ColumnSpec {
    pub name: &'static str,
    pub header_x: f64,
}

/// The column `word` belongs to: the rightmost column whose `header_x` is at
/// or before the word's own x (within `tolerance`), or `None` if `word`
/// starts to the left of every column. `columns` must be in ascending
/// `header_x` order. This (not nearest-by-distance) is the correct rule for
/// left-aligned columns whose content can run wider than the header label
/// itself -- a long cell's trailing words must not spill into the next
/// column once they cross the naive midpoint between two headers.
pub fn column_for_word(word: &Word, columns: &[ColumnSpec], tolerance: f64) -> Option<usize> {
    columns
        .iter()
        .rposition(|column| word.x + tolerance >= column.header_x)
}

/// Bucket one row's words into their columns (via [`column_for_word`]),
/// concatenating same-column words in x-order (space-joined) into that
/// column's cell text. A column with no assigned word in this row is simply
/// absent from the result, mirroring the PDF itself never drawing a
/// placeholder for a blank cell.
pub fn bucket_row_by_columns<'a>(
    row: &[Word],
    columns: &'a [ColumnSpec],
    tolerance: f64,
) -> HashMap<&'a str, String> {
    let mut cells: HashMap<&str, Vec<&str>> = HashMap::new();
    for word in row {
        if let Some(index) = column_for_word(word, columns, tolerance) {
            cells
                .entry(columns[index].name)
                .or_default()
                .push(&word.text);
        }
    }
    cells
        .into_iter()
        .map(|(name, parts)| (name, parts.join(" ")))
        .collect()
}

/// Find the first row containing a word exactly matching every printed label
/// in `labels`, in order, and build one [`ColumnSpec`] per label anchored to
/// that word's x -- named after the label's *canonical* column key (the
/// second element of each pair), not the printed text itself, so a caller
/// juggling more than one language/layout for the same bank (see
/// `deblock.rs`) can look cells up by one fixed set of keys regardless of
/// which label set actually matched. Returns the header row's index plus the
/// columns, or `None` if any label is missing -- callers treat `None` as
/// "not recognized as this bank's statement" (a whole-file structural
/// error), not a per-row one.
pub fn find_header_columns(
    rows: &[Vec<Word>],
    labels: &[(&'static str, &'static str)],
) -> Option<(usize, Vec<ColumnSpec>)> {
    for (index, row) in rows.iter().enumerate() {
        let mut columns = Vec::with_capacity(labels.len());
        for (label, canonical_name) in labels {
            let Some(word) = row.iter().find(|word| word.text == *label) else {
                columns.clear();
                break;
            };
            columns.push(ColumnSpec {
                name: canonical_name,
                header_x: word.x,
            });
        }
        if columns.len() == labels.len() {
            return Some((index, columns));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(x: f64, y: f64, text: &str) -> Word {
        Word {
            page: 0,
            x,
            y,
            text: text.to_owned(),
        }
    }

    #[test]
    fn group_into_rows_separates_rows_at_the_tolerance_boundary() {
        // y increases top-to-bottom in the extraction pipeline's coordinate
        // space (after `begin_page`'s flip transform), so the header row
        // (topmost) has the smallest y here -- unlike raw, unflipped PDF
        // coordinates where it would be the largest.
        let words = vec![
            word(66.0, 0.0, "Date"),
            word(145.65, 0.0, "Valeur"),
            word(66.0, 21.0, "29"),
            word(145.65, 21.1, "avril"), // within tolerance of the row above
        ];
        let rows = group_into_rows(words, 1.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[1].len(), 2);
        // Sorted left-to-right within a row.
        assert_eq!(rows[0][0].text, "Date");
        assert_eq!(rows[0][1].text, "Valeur");
    }

    #[test]
    fn group_into_rows_starts_a_new_row_past_the_tolerance() {
        let words = vec![word(66.0, 556.5, "a"), word(66.0, 560.0, "b")];
        let rows = group_into_rows(words, 1.0);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn column_for_word_assigns_a_long_description_past_the_naive_midpoint() {
        let columns = vec![
            ColumnSpec {
                name: "Opération",
                header_x: 225.3,
            },
            ColumnSpec {
                name: "Débit",
                header_x: 384.6,
            },
        ];
        // A long description word starting well past the midpoint between
        // the two headers (~305) must still land in "Opération", not "Débit",
        // since it's still to the left of the Débit column's own header.
        let long_word = word(370.0, 556.5, "DOE");
        assert_eq!(column_for_word(&long_word, &columns, 1.0), Some(0));

        let debit_amount = word(384.6, 556.5, "9,99");
        assert_eq!(column_for_word(&debit_amount, &columns, 1.0), Some(1));
    }

    #[test]
    fn column_for_word_returns_none_left_of_every_column() {
        let columns = vec![ColumnSpec {
            name: "Date",
            header_x: 66.0,
        }];
        let word = word(10.0, 556.5, "?");
        assert_eq!(column_for_word(&word, &columns, 1.0), None);
    }

    #[test]
    fn bucket_row_by_columns_concatenates_same_column_words_and_omits_blank_columns() {
        let columns = vec![
            ColumnSpec {
                name: "Opération",
                header_x: 225.3,
            },
            ColumnSpec {
                name: "Débit",
                header_x: 384.6,
            },
            ColumnSpec {
                name: "Crédit",
                header_x: 424.425,
            },
        ];
        let row = vec![
            word(225.3, 556.5, "Frais"),
            word(260.0, 556.5, "\"Card"),
            word(290.0, 556.5, "delivery\""),
            word(384.6, 556.5, "9,99"),
        ];
        let cells = bucket_row_by_columns(&row, &columns, 1.0);
        assert_eq!(
            cells.get("Opération"),
            Some(&"Frais \"Card delivery\"".to_owned())
        );
        assert_eq!(cells.get("Débit"), Some(&"9,99".to_owned()));
        assert_eq!(cells.get("Crédit"), None);
    }

    #[test]
    fn find_header_columns_locates_the_header_row_and_its_x_positions() {
        let rows = vec![
            vec![word(66.0, 577.5, "Date"), word(145.65, 577.5, "Valeur")],
            vec![word(66.0, 556.5, "29")],
        ];
        let (index, columns) =
            find_header_columns(&rows, &[("Date", "date"), ("Valeur", "value_date")])
                .expect("found");
        assert_eq!(index, 0);
        assert_eq!(columns[0].header_x, 66.0);
        assert_eq!(columns[1].header_x, 145.65);
    }

    #[test]
    fn find_header_columns_returns_none_when_a_label_is_missing() {
        let rows = vec![vec![word(66.0, 577.5, "Date")]];
        assert!(
            find_header_columns(&rows, &[("Date", "date"), ("Valeur", "value_date")]).is_none()
        );
    }
}
