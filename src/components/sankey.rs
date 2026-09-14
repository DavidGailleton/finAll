use leptos::prelude::*;

use crate::cashflow::types::{CashflowSankeyDto, SankeySide};
use crate::components::{BarList, BarRow};

/// The cashflow Sankey diagram: an inline `<svg>` (decorative — the amounts it
/// encodes are never conveyed by colour or shape alone) plus a text
/// "Full breakdown" list of the same income/expense categories and amounts,
/// so the same information stays available without seeing or parsing the
/// diagram (WCAG 2.2 SC 1.1.1).
#[component]
pub fn Sankey(data: CashflowSankeyDto) -> impl IntoView {
    let CashflowSankeyDto {
        display_currency_code,
        total_income: _,
        total_expense: _,
        complete: _,
        nodes,
        links,
        width,
        height,
    } = data;

    if links.is_empty() {
        return view! {
            <p class="empty-state">
                "No Bank or Cash income or expenses in this period."
            </p>
        }
        .into_any();
    }

    let view_box = format!("0 0 {width} {height}");
    let code = display_currency_code;

    let link_paths = links
        .into_iter()
        .map(|link| {
            let class = match link.side {
                SankeySide::Income => "sankey__link sankey__link--in",
                SankeySide::Expense => "sankey__link sankey__link--out",
                SankeySide::Hub => "sankey__link",
            };
            view! { <path class=class d=link.path></path> }
        })
        .collect_view();

    let income_rows: Vec<BarRow> = nodes
        .iter()
        .filter(|n| n.side == SankeySide::Income)
        .map(|n| BarRow {
            label: n.label.clone(),
            amount: n.value.clone(),
            code: code.clone(),
        })
        .collect();
    let expense_rows: Vec<BarRow> = nodes
        .iter()
        .filter(|n| n.side == SankeySide::Expense)
        .map(|n| BarRow {
            label: n.label.clone(),
            // The layout strips the sign to size the diagram; re-apply it here
            // so the breakdown list keeps this app's income/expense convention
            // (expense amounts shown negative).
            amount: format!("-{}", n.value),
            code: code.clone(),
        })
        .collect();

    let node_shapes = nodes
        .iter()
        .map(|node| {
            let class = match node.side {
                SankeySide::Income => "sankey__node sankey__node--in",
                SankeySide::Expense => "sankey__node sankey__node--out",
                SankeySide::Hub => "sankey__node sankey__node--hub",
            };
            // Expense magnitudes are stripped of their sign to size the
            // diagram (see `crate::server::cashflow::FlowAmount`); shown
            // signed here, matching every other report's negative-expense
            // convention.
            let title = match node.side {
                SankeySide::Hub => node.label.clone(),
                SankeySide::Income => format!("{}: {} {code}", node.label, node.value),
                SankeySide::Expense => format!("{}: -{} {code}", node.label, node.value),
            };
            view! {
                <rect class=class x=node.x y=node.y width=node.width height=node.height>
                    <title>{title}</title>
                </rect>
            }
        })
        .collect_view();

    // On-graph text labels, one per node tall enough to hold a readable
    // line — a very thin sliver (a small category) still carries its label
    // in the `<title>` tooltip and in the "Full breakdown" list below, so
    // nothing is lost when a label is skipped here.
    const MIN_LABELLED_HEIGHT: f64 = 14.0;
    let node_labels = nodes
        .iter()
        .filter(|node| node.height >= MIN_LABELLED_HEIGHT)
        .map(|node| {
            let y = node.y + node.height / 2.0;
            let (x, anchor, text) = match node.side {
                SankeySide::Hub => (node.x + node.width / 2.0, "middle", node.label.clone()),
                SankeySide::Income => (
                    node.x + node.width + 6.0,
                    "start",
                    format!("{} — {} {code}", node.label, node.value),
                ),
                SankeySide::Expense => (
                    node.x - 6.0,
                    "end",
                    format!("{} — -{} {code}", node.label, node.value),
                ),
            };
            let class = match node.side {
                SankeySide::Income => "sankey__label sankey__label--in",
                SankeySide::Expense => "sankey__label sankey__label--out",
                SankeySide::Hub => "sankey__label sankey__label--hub",
            };
            view! {
                <text class=class x=x y=y text-anchor=anchor dominant-baseline="central">
                    {text}
                </text>
            }
        })
        .collect_view();

    view! {
        // A horizontal scroll region, not a shrink-to-fit `<svg>`: the on-graph
        // labels are set in real CSS px, which the SVG's `viewBox` scaling would
        // otherwise shrink below readable size on a narrow viewport.
        <div class="table-scroll" role="region" aria-label="Cashflow diagram" tabindex="0">
            <svg
                viewBox=view_box
                class="sankey"
                role="img"
                aria-label="Income categories flowing through to expense categories"
            >
                {link_paths}
                {node_shapes}
                {node_labels}
            </svg>
        </div>

        <details class="data-detail">
            <summary>"Full breakdown"</summary>
            {(!income_rows.is_empty())
                .then(|| {
                    view! {
                        <div class="stack">
                            <span class="stat__label">"Income"</span>
                            <BarList rows=income_rows />
                        </div>
                    }
                })}
            {(!expense_rows.is_empty())
                .then(|| {
                    view! {
                        <div class="stack">
                            <span class="stat__label">"Expenses"</span>
                            <BarList rows=expense_rows />
                        </div>
                    }
                })}
        </details>
    }
    .into_any()
}
