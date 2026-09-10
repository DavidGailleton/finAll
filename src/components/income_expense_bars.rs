use leptos::prelude::*;

use crate::components::Money;

/// Two bars on a shared scale — income (brand green) and expenses (loss red) —
/// with each total shown as text. The bars are `aria-hidden` decoration; the
/// head labels and `<Money>` totals carry the data.
///
/// Bar geometry parses the decimal strings to `f64` for **pixel widths only**;
/// the displayed totals are the original server strings.
#[component]
pub fn IncomeExpenseBars(
    /// Income total as a decimal string (magnitude).
    #[prop(into)]
    income: String,
    /// Expense total as a decimal string (magnitude).
    #[prop(into)]
    expense: String,
    /// Shared currency code.
    #[prop(into)]
    code: String,
) -> impl IntoView {
    fn magnitude(s: &str) -> f64 {
        s.trim_start_matches('-')
            .parse::<f64>()
            .unwrap_or(0.0)
            .abs()
    }

    let (inc, exp) = (magnitude(&income), magnitude(&expense));
    let denom = inc.max(exp).max(f64::MIN_POSITIVE);
    let inc_pct = (inc / denom * 100.0).clamp(0.0, 100.0);
    let exp_pct = (exp / denom * 100.0).clamp(0.0, 100.0);

    view! {
        <div class="ie-bars">
            <div class="ie-bars__group">
                <div class="ie-bars__head">
                    <span class="label">"Income"</span>
                    <Money amount=income code=code.clone() />
                </div>
                <div class="ie-bars__track" aria-hidden="true">
                    <div
                        class="ie-bars__fill ie-bars__fill--in chart-grow"
                        style=format!("width:{inc_pct:.1}%")
                    ></div>
                </div>
            </div>
            <div class="ie-bars__group">
                <div class="ie-bars__head">
                    <span class="label">"Expenses"</span>
                    <Money amount=expense code=code />
                </div>
                <div class="ie-bars__track" aria-hidden="true">
                    <div
                        class="ie-bars__fill ie-bars__fill--out chart-grow"
                        style=format!("width:{exp_pct:.1}%")
                    ></div>
                </div>
            </div>
        </div>
    }
}
