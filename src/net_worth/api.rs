//! The server function the browser calls to read the signed-in user's net
//! worth.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::net_worth::types::NetWorthReportDto;

/// The signed-in user's total balance across all accounts, valued in
/// `display_currency_code`, with a per-currency breakdown and the explicit rate
/// and valuation timestamp used for each conversion.
///
/// A currency that cannot be valued for lack of an exchange rate is returned as
/// a line with no `converted_amount`; the report's `complete` flag is then
/// `false` and `total` excludes it.
#[server]
pub async fn net_worth(display_currency_code: String) -> Result<NetWorthReportDto, ServerFnError> {
    use crate::net_worth::types::NetWorthLineDto;
    use crate::server::auth::extract;
    use crate::server::net_worth::{self, NetWorthError};

    let pool = expect_context::<sqlx::PgPool>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(NetWorthError::Unauthorized)?;

    let report = net_worth::report(&pool, user.user_id, &display_currency_code).await?;

    Ok(NetWorthReportDto {
        display_currency_code: report.display.alphabetic_code,
        display_minor_units: report.display.minor_units,
        total: report.total.to_string(),
        complete: report.complete,
        rates_as_of: report.rates_as_of.map(|ts| ts.to_rfc3339()),
        lines: report
            .lines
            .into_iter()
            .map(|line| NetWorthLineDto {
                currency_code: line.currency_code,
                amount: line.amount.to_string(),
                converted_amount: line.converted_amount.map(|amount| amount.to_string()),
                rate: line.rate.map(|rate| rate.to_string()),
                valuation_timestamp: line.valuation_timestamp.map(|ts| ts.to_rfc3339()),
                is_display_currency: line.is_display_currency,
            })
            .collect(),
    })
}
