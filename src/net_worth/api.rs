//! The server function the browser calls to read the signed-in user's net
//! worth.
//!
//! The body runs only on the server (`ssr`); server-only imports live inside it
//! so this module still compiles for the browser target, where the call becomes
//! a network request.

use leptos::prelude::*;

use crate::net_worth::types::NetWorthReportDto;

/// The signed-in user's total balance across all accounts, valued in
/// `display_currency_code`, broken down by balance-sheet classification
/// (assets vs liabilities) and account type, with the explicit rate and
/// valuation timestamp used for each conversion.
///
/// An account that cannot be valued for lack of an exchange rate is returned
/// with no `converted_amount`; the report's `complete` flag is then `false`
/// and `total` (and its group's total) excludes it.
#[server]
pub async fn net_worth(display_currency_code: String) -> Result<NetWorthReportDto, ServerFnError> {
    use std::sync::Arc;

    use crate::net_worth::types::{
        AccountGroupDto, ClassificationGroupDto, NetWorthAccountLineDto,
    };
    use crate::server::assets::fx_cache::FxRateCache;
    use crate::server::auth::extract;
    use crate::server::net_worth::{self, NetWorthError};

    let pool = expect_context::<sqlx::PgPool>();
    let cache = expect_context::<Arc<FxRateCache>>();

    let user = extract::current_user(&pool)
        .await?
        .ok_or(NetWorthError::Unauthorized)?;

    let report = net_worth::report(&pool, &cache, user.user_id, &display_currency_code).await?;

    fn to_account_line(line: net_worth::ValuedLine) -> NetWorthAccountLineDto {
        NetWorthAccountLineDto {
            account_id: line.account_id.to_string(),
            account_name: line.account_name,
            amount: line.amount.to_string(),
            currency_code: line.currency_code,
            converted_amount: line.converted_amount.map(|amount| amount.to_string()),
            rate: line.rate.map(|rate| rate.to_string()),
            valuation_timestamp: line.valuation_timestamp.map(|ts| ts.to_rfc3339()),
            is_display_currency: line.is_display_currency,
        }
    }

    Ok(NetWorthReportDto {
        display_currency_code: report.display.alphabetic_code,
        display_minor_units: report.display.minor_units,
        total: report.total.to_string(),
        complete: report.complete,
        rates_as_of: report.rates_as_of.map(|ts| ts.to_rfc3339()),
        classifications: report
            .classifications
            .into_iter()
            .map(|group| ClassificationGroupDto {
                classification: group.classification,
                total: group.total.to_string(),
                account_groups: group
                    .account_groups
                    .into_iter()
                    .map(|account_group| AccountGroupDto {
                        account_type: account_group.account_type,
                        total: account_group.total.to_string(),
                        accounts: account_group
                            .accounts
                            .into_iter()
                            .map(to_account_line)
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    })
}
