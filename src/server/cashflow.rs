//! Cashflow: an income → expense Sankey diagram for `Bank` + `Cash` accounts
//! only, over one chosen period.
//!
//! Reuses [`crate::server::income_expense::report`] (scoped to `Bank`/`Cash`
//! accounts) for the category-level income and expense magnitudes, then lays
//! out a simple two-hop Sankey: income categories → one "Cashflow" hub →
//! expense categories. No crossing-minimization is needed for two hops, so the
//! layout is deterministic geometry, not a general graph-layout algorithm —
//! see [`build_layout`]. This produces plain, non-money display geometry
//! (pixel positions, SVG path strings): the underlying amounts stay exact
//! decimal strings in the DTO: only their on-screen size is ever parsed to
//! `f64`, the same "parse to f64 for pixel width only" rule already used by
//! this app's other hand-built charts (`BarList`, `IncomeExpenseBars`).
//!
//! Every query is scoped by `user_id`.

use bigdecimal::BigDecimal;
use sqlx::types::Uuid;
use sqlx::PgPool;

use crate::accounts::types::AccountType;
use crate::cashflow::types::SankeySide;
use crate::server::assets::fx_cache::FxRateCache;
use crate::server::income_expense;

/// SVG viewBox size the layout is computed for; the `<svg>` element itself
/// scales to its container, so these are logical units, not pixels.
const CANVAS_WIDTH: f64 = 640.0;
const CANVAS_HEIGHT: f64 = 320.0;
const NODE_WIDTH: f64 = 14.0;
const HUB_WIDTH: f64 = 70.0;
/// Vertical gap between stacked nodes in the same column.
const GAP: f64 = 6.0;
/// Minimum node/link height so a very small category stays visible.
const MIN_SIZE: f64 = 1.5;

pub use income_expense::IncomeExpenseError as CashflowError;

/// One income or expense category's magnitude, before layout.
pub struct FlowAmount {
    pub label: String,
    /// Always `>= 0` — the category's magnitude, sign already stripped by the
    /// caller (income nets are positive, expense nets negative in the
    /// underlying report).
    pub magnitude: BigDecimal,
}

pub struct SankeyNode {
    pub label: String,
    pub value: BigDecimal,
    pub side: SankeySide,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct SankeyLink {
    pub value: BigDecimal,
    pub side: SankeySide,
    /// A closed SVG path (`d` attribute) for the filled ribbon.
    pub path: String,
}

pub struct SankeyLayout {
    pub nodes: Vec<SankeyNode>,
    pub links: Vec<SankeyLink>,
    pub width: f64,
    pub height: f64,
}

pub struct Report {
    pub display_currency_code: String,
    pub total_income: BigDecimal,
    pub total_expense: BigDecimal,
    pub complete: bool,
    pub layout: SankeyLayout,
}

/// The signed-in user's cashflow for `[from, to]`, restricted to `Bank` and
/// `Cash` accounts, valued in `display_currency_code`.
pub async fn report(
    pool: &PgPool,
    cache: &FxRateCache,
    user_id: Uuid,
    display_currency_code: &str,
    from: &str,
    to: &str,
) -> Result<Report, CashflowError> {
    let ie_report = income_expense::report(
        pool,
        cache,
        user_id,
        display_currency_code,
        from,
        to,
        Some(&[AccountType::Bank, AccountType::Cash]),
    )
    .await?;

    let category_label = |group: &income_expense::GroupLine| {
        group.category_name.clone().unwrap_or_else(|| {
            if group.category_deleted {
                "(deleted category)".to_owned()
            } else {
                "Uncategorised".to_owned()
            }
        })
    };

    // Only groups with a converted net can be sized — matches every other
    // report in this app, which excludes an unvalued group from its totals
    // rather than guessing at its size.
    let income: Vec<FlowAmount> = ie_report
        .income_lines
        .iter()
        .filter_map(|group| {
            group.converted_net.as_ref().map(|net| FlowAmount {
                label: category_label(group),
                magnitude: net.clone(),
            })
        })
        .collect();
    let expense: Vec<FlowAmount> = ie_report
        .expense_lines
        .iter()
        .filter_map(|group| {
            group.converted_net.as_ref().map(|net| FlowAmount {
                label: category_label(group),
                magnitude: -net,
            })
        })
        .collect();

    Ok(Report {
        display_currency_code: ie_report.display.alphabetic_code,
        total_income: ie_report.total_income,
        total_expense: ie_report.total_expense,
        complete: ie_report.complete,
        layout: build_layout(&income, &expense),
    })
}

/// Lay out the two-hop Sankey: `income` nodes on the left, one "Cashflow" hub
/// in the middle, `expense` nodes on the right.
///
/// Node/link heights are proportional to magnitude, scaled so the larger of
/// the two sides fills the canvas height (minus its own inter-node gaps); the
/// smaller side — and the hub, which always spans the larger side's total —
/// is proportionally shorter, so a surplus or deficit between income and
/// expense shows up as visible empty space rather than a fabricated category.
fn build_layout(income: &[FlowAmount], expense: &[FlowAmount]) -> SankeyLayout {
    fn magnitude_f64(amount: &BigDecimal) -> f64 {
        amount.to_string().parse::<f64>().unwrap_or(0.0).max(0.0)
    }

    let income_total: f64 = income.iter().map(|a| magnitude_f64(&a.magnitude)).sum();
    let expense_total: f64 = expense.iter().map(|a| magnitude_f64(&a.magnitude)).sum();

    let (dominant_total, dominant_count) = if income_total >= expense_total {
        (income_total, income.len())
    } else {
        (expense_total, expense.len())
    };
    let usable = (CANVAS_HEIGHT - GAP * dominant_count.saturating_sub(1) as f64).max(1.0);
    let scale = if dominant_total > 0.0 {
        usable / dominant_total
    } else {
        0.0
    };
    let hub_height = if dominant_total > 0.0 { usable } else { 0.0 };

    let hub_x = (CANVAS_WIDTH - HUB_WIDTH) / 2.0;
    let mut nodes = Vec::new();
    let mut links = Vec::new();

    let hub_top = (CANVAS_HEIGHT - hub_height) / 2.0;
    if hub_height > 0.0 {
        nodes.push(SankeyNode {
            label: "Cashflow".to_owned(),
            value: BigDecimal::from(0), // the hub has no single amount of its own
            side: SankeySide::Hub,
            x: hub_x,
            y: hub_top,
            width: HUB_WIDTH,
            height: hub_height,
        });
    }

    let mut income_cursor = hub_top;
    let mut hub_left_cursor = hub_top;
    for amount in income {
        let height = (magnitude_f64(&amount.magnitude) * scale).max(MIN_SIZE);
        let node_y = income_cursor;
        nodes.push(SankeyNode {
            label: amount.label.clone(),
            value: amount.magnitude.clone(),
            side: SankeySide::Income,
            x: 0.0,
            y: node_y,
            width: NODE_WIDTH,
            height,
        });
        links.push(SankeyLink {
            value: amount.magnitude.clone(),
            side: SankeySide::Income,
            path: ribbon_path(
                NODE_WIDTH,
                node_y,
                node_y + height,
                hub_x,
                hub_left_cursor,
                hub_left_cursor + height,
            ),
        });
        income_cursor += height + GAP;
        hub_left_cursor += height;
    }

    let mut expense_cursor = hub_top;
    let mut hub_right_cursor = hub_top;
    let hub_right_x = hub_x + HUB_WIDTH;
    let expense_x = CANVAS_WIDTH - NODE_WIDTH;
    for amount in expense {
        let height = (magnitude_f64(&amount.magnitude) * scale).max(MIN_SIZE);
        let node_y = expense_cursor;
        nodes.push(SankeyNode {
            label: amount.label.clone(),
            value: amount.magnitude.clone(),
            side: SankeySide::Expense,
            x: expense_x,
            y: node_y,
            width: NODE_WIDTH,
            height,
        });
        links.push(SankeyLink {
            value: amount.magnitude.clone(),
            side: SankeySide::Expense,
            path: ribbon_path(
                hub_right_x,
                hub_right_cursor,
                hub_right_cursor + height,
                expense_x,
                node_y,
                node_y + height,
            ),
        });
        expense_cursor += height + GAP;
        hub_right_cursor += height;
    }

    SankeyLayout {
        nodes,
        links,
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
    }
}

/// A filled ribbon between a `[y0_start, y0_end]` band at `x0` and a
/// `[y1_start, y1_end]` band at `x1`, curved with a horizontal cubic bezier.
fn ribbon_path(x0: f64, y0_start: f64, y0_end: f64, x1: f64, y1_start: f64, y1_end: f64) -> String {
    let mid_x = (x0 + x1) / 2.0;
    format!(
        "M{x0:.2},{y0_start:.2} \
         C{mid_x:.2},{y0_start:.2} {mid_x:.2},{y1_start:.2} {x1:.2},{y1_start:.2} \
         L{x1:.2},{y1_end:.2} \
         C{mid_x:.2},{y1_end:.2} {mid_x:.2},{y0_end:.2} {x0:.2},{y0_end:.2} Z"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).expect("valid decimal literal")
    }

    fn amount(label: &str, magnitude: &str) -> FlowAmount {
        FlowAmount {
            label: label.to_owned(),
            magnitude: dec(magnitude),
        }
    }

    #[test]
    fn empty_input_produces_no_nodes_or_links() {
        let layout = build_layout(&[], &[]);
        assert!(layout.nodes.is_empty());
        assert!(layout.links.is_empty());
    }

    #[test]
    fn node_heights_are_proportional_to_magnitude() {
        let income = [amount("Salary", "300"), amount("Refunds", "100")];
        let layout = build_layout(&income, &[]);

        let salary = layout
            .nodes
            .iter()
            .find(|n| n.label == "Salary")
            .expect("salary node");
        let refunds = layout
            .nodes
            .iter()
            .find(|n| n.label == "Refunds")
            .expect("refunds node");

        // 300 vs 100 -> a 3:1 height ratio.
        assert!((salary.height / refunds.height - 3.0).abs() < 0.01);
    }

    #[test]
    fn the_hub_spans_the_dominant_sides_total() {
        let income = [amount("Salary", "100")];
        let expense = [amount("Rent", "40"), amount("Food", "20")];
        let layout = build_layout(&income, &expense);

        let hub = layout
            .nodes
            .iter()
            .find(|n| n.side == SankeySide::Hub)
            .expect("a hub node");
        let income_node = layout
            .nodes
            .iter()
            .find(|n| n.side == SankeySide::Income)
            .expect("income node");

        // Income (100) dominates expense (60), so the hub matches the income
        // node's height and the expense side stays visibly shorter.
        assert!((hub.height - income_node.height).abs() < 0.01);
        let expense_total: f64 = layout
            .nodes
            .iter()
            .filter(|n| n.side == SankeySide::Expense)
            .map(|n| n.height)
            .sum();
        assert!(expense_total < hub.height);
    }

    #[test]
    fn income_and_expense_totals_match_the_underlying_report() {
        // A sanity check that build_layout doesn't alter magnitudes: the sum
        // of node values on each side equals the input magnitudes' sum.
        let income = [amount("Salary", "250.50"), amount("Interest", "4.25")];
        let expense = [amount("Rent", "100")];
        let layout = build_layout(&income, &expense);

        let income_sum: BigDecimal = layout
            .nodes
            .iter()
            .filter(|n| n.side == SankeySide::Income)
            .fold(BigDecimal::from(0), |acc, n| acc + &n.value);
        assert_eq!(income_sum, dec("254.75"));

        let expense_sum: BigDecimal = layout
            .nodes
            .iter()
            .filter(|n| n.side == SankeySide::Expense)
            .fold(BigDecimal::from(0), |acc, n| acc + &n.value);
        assert_eq!(expense_sum, dec("100"));
    }
}
