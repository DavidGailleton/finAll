//! `/` — the dashboard homepage. Its first panel is the net worth report: the
//! signed-in user's balances across every account, summed into a chosen display
//! currency with a per-currency breakdown.
//!
//! Follows the `Resource` + `<Suspense>` loading/empty/error pattern used by the
//! other pages (see [`crate::pages::accounts`]).

use leptos::prelude::*;

use crate::assets::currency::api::list_currencies;
use crate::components::Layout;
use crate::net_worth::api::net_worth;
use crate::net_worth::types::NetWorthReportDto;
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;

/// `/`
#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <RequireAuth>
            <Layout>
                <NetWorthReport />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn NetWorthReport() -> impl IntoView {
    // The display currency is chosen on the page and not persisted (there is no
    // user-settings store yet); it defaults to EUR, the app's anchor currency.
    let display_currency = RwSignal::new("EUR".to_owned());

    let currencies = Resource::new(|| (), |_| async move { list_currencies().await });

    // Refetch the report whenever the chosen currency changes.
    let report = Resource::new(
        move || display_currency.get(),
        |code| async move { net_worth(code).await },
    );

    view! {
        <h1>"Net worth"</h1>

        <Suspense fallback=|| view! { <p class="loading">"Loading currencies…"</p> }>
            {move || {
                currencies
                    .get()
                    .map(|result| match result {
                        Err(err) => {
                            view! {
                                <p class="form-error" role="alert">
                                    {server_error_message(&err)}
                                </p>
                            }
                                .into_any()
                        }
                        Ok(list) => {
                            view! {
                                <div class="field">
                                    <label for="display-currency">"Display currency"</label>
                                    <select
                                        id="display-currency"
                                        name="display-currency"
                                        prop:value=move || display_currency.get()
                                        on:change=move |ev| {
                                            display_currency.set(event_target_value(&ev))
                                        }
                                    >
                                        {list
                                            .into_iter()
                                            .map(|currency| {
                                                view! {
                                                    <option value=currency
                                                        .alphabetic_code
                                                        .clone()>
                                                        {format!(
                                                            "{} — {}",
                                                            currency.alphabetic_code,
                                                            currency.currency_name,
                                                        )}
                                                    </option>
                                                }
                                            })
                                            .collect_view()}
                                    </select>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>

        <Suspense fallback=|| view! { <p class="loading">"Loading net worth…"</p> }>
            {move || {
                report
                    .get()
                    .map(|result| match result {
                        Err(err) => {
                            view! {
                                <p class="form-error" role="alert">
                                    {server_error_message(&err)}
                                </p>
                            }
                                .into_any()
                        }
                        Ok(report) if report.lines.is_empty() => {
                            view! {
                                <p class="empty-state">"No account balances yet."</p>
                            }
                                .into_any()
                        }
                        Ok(report) => view! { <NetWorthBody report=report /> }.into_any(),
                    })
            }}
        </Suspense>
    }
}

/// Turn an RFC 3339 timestamp string into its `YYYY-MM-DD` date portion, to
/// match the date convention used elsewhere in the app.
fn date_part(timestamp: &str) -> &str {
    timestamp.split('T').next().unwrap_or(timestamp)
}

#[component]
fn NetWorthBody(report: NetWorthReportDto) -> impl IntoView {
    let NetWorthReportDto {
        display_currency_code,
        display_minor_units: _,
        total,
        complete,
        rates_as_of,
        lines,
    } = report;

    let code = display_currency_code;
    let value_header = format!("Value in {code}");

    let rows = lines
        .into_iter()
        .map(|line| {
            let dash = "—".to_owned();
            let rate = line.rate.clone().unwrap_or_else(|| dash.clone());
            let rate_date = line
                .valuation_timestamp
                .as_deref()
                .map(|ts| date_part(ts).to_owned())
                .unwrap_or_else(|| dash.clone());
            let converted = match &line.converted_amount {
                Some(amount) => format!("{amount} {code}"),
                None => "not available".to_owned(),
            };
            let balance = format!("{} {}", line.amount, line.currency_code);
            view! {
                <tr>
                    <td>{line.currency_code}</td>
                    <td class="transaction-amount">{balance}</td>
                    <td class="transaction-amount">{rate}</td>
                    <td>{rate_date}</td>
                    <td class="transaction-amount">{converted}</td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <p class="net-worth-total">
            <strong>{format!("{total} {code}")}</strong>
        </p>

        {(!complete)
            .then(|| {
                view! {
                    <p class="net-worth-warning" role="alert">
                        "Some balances have no exchange rate and are not included in the total."
                    </p>
                }
            })}

        <div class="table-scroll">
            <table class="transactions-table">
                <thead>
                    <tr>
                        <th>"Currency"</th>
                        <th>"Balance"</th>
                        <th>"Rate"</th>
                        <th>"Rate date"</th>
                        <th>{value_header}</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>

        {rates_as_of
            .map(|ts| {
                let as_of = date_part(&ts).to_owned();
                view! { <p class="empty-state">{format!("Exchange rates as of {as_of}")}</p> }
            })}
    }
}
