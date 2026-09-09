//! `/` — the dashboard homepage. Its first panel is the net worth report (the
//! signed-in user's balances across every account, summed into a chosen display
//! currency); its second is income vs expense over a chosen period, grouped by
//! category.
//!
//! Follows the `Resource` + `<Suspense>` loading/empty/error pattern used by the
//! other pages (see [`crate::pages::accounts`]).

use leptos::prelude::*;

use crate::assets::currency::api::list_currencies;
use crate::components::Layout;
use crate::income_expense::api::income_expense_report;
use crate::income_expense::types::IncomeExpenseReportDto;
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
                <IncomeExpenseReport />
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

/// Drop a leading minus so the UI shows a magnitude; totals like `net` are shown
/// signed.
fn magnitude(amount: &str) -> &str {
    amount.strip_prefix('-').unwrap_or(amount)
}

#[component]
fn IncomeExpenseReport() -> impl IntoView {
    // Its own controls, independent of the net worth panel's currency picker.
    let display_currency = RwSignal::new("EUR".to_owned());
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());

    let currencies = Resource::new(|| (), |_| async move { list_currencies().await });

    // Idle until both bounds are chosen — so the first render never reads the
    // clock and there is no hydration mismatch.
    let report = Resource::new(
        move || {
            let (from, to) = (from.get(), to.get());
            (!from.is_empty() && !to.is_empty()).then_some((display_currency.get(), from, to))
        },
        |params| async move {
            match params {
                Some((code, from, to)) => Some(income_expense_report(code, from, to).await),
                None => None,
            }
        },
    );

    view! {
        <h2>"Income vs expense"</h2>

        <div class="income-expense-controls">
            <div class="field">
                <label for="income-expense-from">"From"</label>
                <input
                    id="income-expense-from"
                    type="date"
                    prop:value=move || from.get()
                    on:change=move |ev| from.set(event_target_value(&ev))
                />
            </div>
            <div class="field">
                <label for="income-expense-to">"To"</label>
                <input
                    id="income-expense-to"
                    type="date"
                    prop:value=move || to.get()
                    on:change=move |ev| to.set(event_target_value(&ev))
                />
            </div>
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
                                        <label for="income-expense-currency">"Display currency"</label>
                                        <select
                                            id="income-expense-currency"
                                            name="income-expense-currency"
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
        </div>

        <Suspense fallback=|| view! { <p class="loading">"Loading report…"</p> }>
            {move || {
                report
                    .get()
                    .map(|slot| match slot {
                        None => {
                            view! {
                                <p class="empty-state">"Choose a start and an end date."</p>
                            }
                                .into_any()
                        }
                        Some(Err(err)) => {
                            view! {
                                <p class="form-error" role="alert">
                                    {server_error_message(&err)}
                                </p>
                            }
                                .into_any()
                        }
                        Some(Ok(report))
                            if report.income_lines.is_empty() && report.expense_lines.is_empty()
                                && report.unvalued_lines.is_empty() =>
                        {
                            view! {
                                <p class="empty-state">"No income or expenses in this period."</p>
                            }
                                .into_any()
                        }
                        Some(Ok(report)) => {
                            view! { <IncomeExpenseBody report=report /> }.into_any()
                        }
                    })
            }}
        </Suspense>
    }
}

#[component]
fn IncomeExpenseBody(report: IncomeExpenseReportDto) -> impl IntoView {
    let IncomeExpenseReportDto {
        from,
        to,
        display_currency_code,
        display_minor_units: _,
        income_lines,
        expense_lines,
        unvalued_lines,
        total_income,
        total_expense,
        net,
        complete,
        rates_as_of,
    } = report;

    let code = display_currency_code;
    let net_header = format!("Net in {code}");
    let summary = format!(
        "Income {} {code} · Expenses {} {code}",
        magnitude(&total_income),
        magnitude(&total_expense),
    );
    let warning = format!(
        "Some amounts have no exchange rate for {code} as of {} and are left out of the totals.",
        date_part(&rates_as_of),
    );
    let footnote = format!("{from} to {to}, valued as of {}", date_part(&rates_as_of));

    view! {
        <p class="net-worth-total">
            <strong>{format!("Net {net} {code}")}</strong>
        </p>
        <p class="empty-state">{summary}</p>

        {(!complete)
            .then(|| {
                view! { <p class="net-worth-warning" role="alert">{warning.clone()}</p> }
            })}

        <IncomeExpenseSection heading="Income" code=code.clone() net_header=net_header.clone() lines=income_lines />
        <IncomeExpenseSection heading="Expenses" code=code.clone() net_header=net_header.clone() lines=expense_lines />
        <IncomeExpenseSection heading="Not valued" code=code.clone() net_header=net_header lines=unvalued_lines />

        <p class="empty-state">{footnote}</p>
    }
}

#[component]
fn IncomeExpenseSection(
    heading: &'static str,
    code: String,
    net_header: String,
    lines: Vec<crate::income_expense::types::IncomeExpenseLineDto>,
) -> impl IntoView {
    if lines.is_empty() {
        return ().into_any();
    }

    let rows = lines
        .into_iter()
        .map(|line| {
            let category = line.category_name.clone().unwrap_or_else(|| {
                if line.category_deleted {
                    "(deleted category)".to_owned()
                } else {
                    "Uncategorised".to_owned()
                }
            });
            let kind = line
                .category_kind
                .map(|k| k.label().to_owned())
                .unwrap_or_else(|| "—".to_owned());
            let net = match &line.converted_net {
                Some(value) if !line.complete => format!("{value} {code} (partial)"),
                Some(value) => format!("{value} {code}"),
                None => "not valued".to_owned(),
            };
            let detail = (line.currencies.len() > 1 || !line.complete).then(|| {
                let parts = line
                    .currencies
                    .iter()
                    .map(|part| {
                        let converted = match &part.converted_amount {
                            Some(amount) => format!(" → {amount} {code}"),
                            None => " → not valued".to_owned(),
                        };
                        format!("{} {}{}", part.amount, part.currency_code, converted)
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                view! {
                    <tr>
                        <td colspan="3" class="income-expense-detail">{parts}</td>
                    </tr>
                }
            });
            view! {
                <tr>
                    <td>{category}</td>
                    <td>{kind}</td>
                    <td class="transaction-amount">{net}</td>
                </tr>
                {detail}
            }
        })
        .collect_view();

    view! {
        <section class="income-expense-section">
            <h3>{heading}</h3>
            <div class="table-scroll">
                <table class="transactions-table">
                    <thead>
                        <tr>
                            <th>"Category"</th>
                            <th>"Kind"</th>
                            <th>{net_header}</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        </section>
    }
    .into_any()
}
