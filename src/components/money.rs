use leptos::prelude::*;

/// Renders a monetary amount the way a statement does: the sign as a glyph
/// **and** a colour **and** assistive text (never colour alone, per WCAG
/// SC 1.4.1), the figure in tabular monospace, and the currency code in muted
/// sans beside it.
///
/// Presentational only: `amount` already arrives as an exact decimal string
/// from a DTO (e.g. `"-1234.50"`), and `decimals` only controls how many of
/// its fractional digits are *displayed* (rounded half-up, in exact decimal
/// string arithmetic — never through `f32`/`f64`). The source DTO and any
/// stored value are never touched.
#[component]
pub fn Money(
    /// The amount as an exact decimal string, optionally with a leading `-`.
    #[prop(into)]
    amount: String,
    /// The currency's alphabetic code (e.g. `"EUR"`).
    #[prop(into)]
    code: String,
    /// Number of fractional digits to display, rounded half-up. Defaults to
    /// `0` (the app's general policy); pass `2` on pages that need cents.
    #[prop(default = 0)]
    decimals: u8,
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
    let magnitude = round_decimal_string(&magnitude, decimals);

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

/// Reformats a non-negative exact decimal string (no leading `-`) to exactly
/// `decimals` fractional digits, rounding half-up. Pure string/integer
/// arithmetic — no floating point, so no precision loss on values of any
/// magnitude. `decimals = 0` omits the decimal point entirely.
fn round_decimal_string(magnitude: &str, decimals: u8) -> String {
    let decimals = decimals as usize;
    let (int_part, frac_part) = magnitude.split_once('.').unwrap_or((magnitude, ""));

    if frac_part.len() <= decimals {
        let mut out = int_part.to_owned();
        if decimals > 0 {
            out.push('.');
            out.push_str(frac_part);
            out.push_str(&"0".repeat(decimals - frac_part.len()));
        }
        return out;
    }

    let round_up = frac_part.as_bytes()[decimals] >= b'5';
    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(decimals))
        .map(|b| b - b'0')
        .collect();

    if round_up {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }

    let split_at = digits.len() - decimals;
    let (int_digits, frac_digits) = digits.split_at(split_at);
    let to_str = |ds: &[u8]| ds.iter().map(|d| (d + b'0') as char).collect::<String>();

    let mut out = to_str(int_digits);
    if decimals > 0 {
        out.push('.');
        out.push_str(&to_str(frac_digits));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::round_decimal_string;

    #[test]
    fn pads_when_fewer_digits_than_requested() {
        assert_eq!(round_decimal_string("85", 2), "85.00");
        assert_eq!(round_decimal_string("1.2", 2), "1.20");
        assert_eq!(round_decimal_string("1.23", 2), "1.23");
    }

    #[test]
    fn rounds_half_up_without_carry() {
        assert_eq!(round_decimal_string("2568.1700", 2), "2568.17");
        assert_eq!(round_decimal_string("2568.1750", 2), "2568.18");
        assert_eq!(round_decimal_string("48.8300", 2), "48.83");
    }

    #[test]
    fn rounds_half_up_with_carry_into_integer_part() {
        assert_eq!(round_decimal_string("9.996", 2), "10.00");
        assert_eq!(round_decimal_string("99.995", 2), "100.00");
    }

    #[test]
    fn decimals_zero_drops_the_point_entirely() {
        assert_eq!(round_decimal_string("1234.60", 0), "1235");
        assert_eq!(round_decimal_string("1234.40", 0), "1234");
        assert_eq!(round_decimal_string("85", 0), "85");
    }
}
