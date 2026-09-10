use leptos::prelude::*;

use crate::components::Money;

/// One row of a [`BarList`].
#[derive(Clone)]
pub struct BarRow {
    /// Row label (currency code, category name…).
    pub label: String,
    /// The value as an exact decimal string, optionally with a leading `-`.
    pub amount: String,
    /// The value's currency code.
    pub code: String,
}

/// A quiet horizontal bar chart: one labelled bar per row, width proportional to
/// the largest magnitude, single-hue green (red for negatives). The bars are
/// `aria-hidden` decoration — the label and the `<Money>` value carry the data,
/// so the chart is fully readable without seeing colour.
///
/// Bar geometry parses the decimal string to `f64` for a **pixel width only**;
/// every displayed number is the original server string, untouched.
#[component]
pub fn BarList(rows: Vec<BarRow>) -> impl IntoView {
    fn magnitude(s: &str) -> f64 {
        s.trim_start_matches('-')
            .parse::<f64>()
            .unwrap_or(0.0)
            .abs()
    }

    let max = rows
        .iter()
        .map(|r| magnitude(&r.amount))
        .fold(0.0_f64, f64::max);
    let denom = if max > 0.0 { max } else { 1.0 };

    let bars = rows
        .into_iter()
        .map(|row| {
            let negative = row.amount.starts_with('-');
            let pct = (magnitude(&row.amount) / denom * 100.0).clamp(0.0, 100.0);
            let fill_class = if negative {
                "barlist__fill barlist__fill--down chart-grow"
            } else {
                "barlist__fill chart-grow"
            };
            view! {
                <li class="barlist__row">
                    <span class="barlist__label">{row.label}</span>
                    <span class="barlist__track" aria-hidden="true">
                        <span class=fill_class style=format!("width:{pct:.1}%")></span>
                    </span>
                    <span class="barlist__value">
                        <Money amount=row.amount code=row.code />
                    </span>
                </li>
            }
        })
        .collect_view();

    view! { <ul class="barlist">{bars}</ul> }
}
