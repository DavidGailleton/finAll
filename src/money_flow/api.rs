//! The server function the browser calls to read the money-flow report.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::money_flow::types::MoneyFlowReportDto;

/// The signed-in user's money flow: a 12-calendar-month income/expense trend
/// ending at `to`'s month, plus the `[from, to]` period's income/expense/net
/// totals, valued in `display_currency_code`.
///
/// See [`crate::server::money_flow::report`] for the valuation rules (same as
/// the income-vs-expense report: each day's amount at that day's own
/// booking-date rate).
#[server]
pub async fn money_flow_report(
    display_currency_code: String,
    from: String,
    to: String,
) -> Result<MoneyFlowReportDto, ServerFnError> {
    use std::sync::Arc;

    use crate::money_flow::types::MoneyFlowMonthDto;
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::money_flow::{self, MoneyFlowError};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(MoneyFlowError::Unauthorized)?;

    let report = money_flow::report(
        &pool,
        &cache,
        user.user_id,
        &display_currency_code,
        &from,
        &to,
    )
    .await?;

    Ok(MoneyFlowReportDto {
        display_currency_code: report.display.alphabetic_code,
        display_minor_units: report.display.minor_units,
        months: report
            .months
            .into_iter()
            .map(|month| MoneyFlowMonthDto {
                month: month.month.format("%Y-%m").to_string(),
                income: month.income.to_string(),
                expense: month.expense.to_string(),
            })
            .collect(),
        total_income: report.total_income.to_string(),
        total_expense: report.total_expense.to_string(),
        net: report.net.to_string(),
        complete: report.complete,
    })
}
