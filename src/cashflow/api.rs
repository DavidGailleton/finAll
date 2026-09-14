//! The server function the browser calls to read the cashflow Sankey diagram.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::cashflow::types::CashflowSankeyDto;

/// The signed-in user's cashflow for `[from, to]`, restricted to `Bank` and
/// `Cash` accounts, valued in `display_currency_code`, as an income → expense
/// Sankey diagram.
///
/// See [`crate::server::cashflow::report`] for the category scoping and
/// layout rules.
#[server]
pub async fn cashflow_sankey(
    display_currency_code: String,
    from: String,
    to: String,
) -> Result<CashflowSankeyDto, ServerFnError> {
    use std::sync::Arc;

    use crate::cashflow::types::{SankeyLinkDto, SankeyNodeDto};
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::cashflow::{self, CashflowError};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(CashflowError::Unauthorized)?;

    let report = cashflow::report(
        &pool,
        &cache,
        user.user_id,
        &display_currency_code,
        &from,
        &to,
    )
    .await?;

    Ok(CashflowSankeyDto {
        display_currency_code: report.display_currency_code,
        total_income: report.total_income.to_string(),
        total_expense: report.total_expense.to_string(),
        complete: report.complete,
        width: report.layout.width,
        height: report.layout.height,
        nodes: report
            .layout
            .nodes
            .into_iter()
            .map(|node| SankeyNodeDto {
                label: node.label,
                value: node.value.to_string(),
                side: node.side,
                x: node.x,
                y: node.y,
                width: node.width,
                height: node.height,
            })
            .collect(),
        links: report
            .layout
            .links
            .into_iter()
            .map(|link| SankeyLinkDto {
                value: link.value.to_string(),
                side: link.side,
                path: link.path,
            })
            .collect(),
    })
}
