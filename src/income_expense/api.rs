//! The server function the browser calls to read the income-vs-expense report.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::income_expense::types::IncomeExpenseReportDto;

/// Income vs expense for the signed-in user over `[from, to]` (inclusive,
/// booking date), grouped by category, valued in `display_currency_code` at the
/// exchange rate in force on `to`.
///
/// Transfers are excluded. Per category the figure is the signed sum of its
/// transactions; each group is placed on the income or expense side by the sign
/// of that (converted) net, with the category's own kind shown only as a label.
/// A `(category, currency)` subtotal with no as-of rate is returned without a
/// converted value; `complete` is then `false` and the totals exclude it.
#[server]
pub async fn income_expense_report(
    display_currency_code: String,
    from: String,
    to: String,
) -> Result<IncomeExpenseReportDto, ServerFnError> {
    use std::sync::Arc;

    use crate::income_expense::types::{IncomeExpenseCurrencyDto, IncomeExpenseLineDto};
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::income_expense::{self, GroupLine, IncomeExpenseError};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(IncomeExpenseError::Unauthorized)?;

    let report = income_expense::report(
        &pool,
        &cache,
        user.user_id,
        &display_currency_code,
        &from,
        &to,
    )
    .await?;

    fn to_line(group: GroupLine) -> IncomeExpenseLineDto {
        IncomeExpenseLineDto {
            category_id: group.category_id.map(|id| id.to_string()),
            category_name: group.category_name,
            category_kind: group.category_kind,
            category_deleted: group.category_deleted,
            converted_net: group.converted_net.map(|net| net.to_string()),
            complete: group.complete,
            currencies: group
                .currencies
                .into_iter()
                .map(|part| IncomeExpenseCurrencyDto {
                    currency_code: part.currency_code,
                    amount: part.amount.to_string(),
                    converted_amount: part.converted_amount.map(|amount| amount.to_string()),
                    rate: part.rate.map(|rate| rate.to_string()),
                })
                .collect(),
        }
    }

    Ok(IncomeExpenseReportDto {
        from: report.from.to_string(),
        to: report.to.to_string(),
        display_currency_code: report.display.alphabetic_code,
        display_minor_units: report.display.minor_units,
        income_lines: report.income_lines.into_iter().map(to_line).collect(),
        expense_lines: report.expense_lines.into_iter().map(to_line).collect(),
        unvalued_lines: report.unvalued_lines.into_iter().map(to_line).collect(),
        total_income: report.total_income.to_string(),
        total_expense: report.total_expense.to_string(),
        net: report.net.to_string(),
        complete: report.complete,
        rates_as_of: report.to.to_string(),
    })
}
