use leptos::prelude::*;

/// Renders a monetary amount the way a statement does: the sign as a glyph
/// **and** a colour **and** assistive text (never colour alone, per WCAG
/// SC 1.4.1), the figure in tabular monospace, and the currency code in muted
/// sans beside it.
///
/// Presentational only. `amount` already arrives as an exact decimal string
/// from a DTO (e.g. `"-1234.50"`); this does no arithmetic or rounding.
#[component]
pub fn Money(
    /// The amount as an exact decimal string, optionally with a leading `-`.
    #[prop(into)]
    amount: String,
    /// The currency's alphabetic code (e.g. `"EUR"`).
    #[prop(into)]
    code: String,
    /// Render at headline / figure size.
    #[prop(optional)]
    figure: bool,
    /// Prefix a `+` on non-negative amounts, for explicit gain/loss columns.
    #[prop(optional)]
    signed: bool,
) -> impl IntoView {
    let negative = amount.starts_with('-');
    let magnitude = amount
        .strip_prefix('-')
        .unwrap_or(amount.as_str())
        .to_owned();

    let (glyph, sr, tone) = if negative {
        ("\u{2212}", "negative ", " money--loss")
    } else if signed {
        ("+", "positive ", " money--gain")
    } else {
        ("", "", "")
    };

    let class = format!("money{tone}{}", if figure { " money--figure" } else { "" });

    view! {
        <span class=class>
            {(!sr.is_empty()).then(|| view! { <span class="sr-only">{sr}</span> })}
            {(!glyph.is_empty()).then(|| view! { <span aria-hidden="true">{glyph}</span> })}
            {magnitude}
            <span class="money__code">{code}</span>
        </span>
    }
}
