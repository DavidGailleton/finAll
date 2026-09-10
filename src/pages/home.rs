//! `/` — the dashboard. Its first panel is the net worth report (the signed-in
//! user's balances across every account, summed into a chosen display
//! currency); its second is income vs expense over a chosen period, grouped by
//! category.
//!
//! Follows the `Resource` + `<Suspense>` loading/empty/error pattern used by the
//! other pages (see [`crate::pages::accounts`]).

use leptos::prelude::*;
use leptos_meta::Title;

use crate::assets::currency::api::list_currencies;
use crate::components::{Layout, Money, PageHeader, Panel, ScrollableTable};
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
        <Title text="Dashboard · finAll" />
        <RequireAuth>
            <Layout>
                <PageHeader
                    title="Dashboard"
                    description="Where your money stands right now, and how it moved over a period you choose."
                />
                <div class="bento">
                    <Panel title="Net worth" span=7 primary=true>
                        <NetWorthReport />
                    </Panel>
                    <Panel title="Income vs expense" span=5>
                        <IncomeExpenseReport />
                    </Panel>
                </div>
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
        <Suspense fallback=|| view! { <p class="loading">"Loading currencies…"</p> }>
            {move || {
                currencies
                    .get()
                    .map(|result| match result {
                        Err(err) => {
                            view! { <p class="form-error">{server_error_message(&err)}</p> }
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

        <div aria-live="polite">
            <Suspense fallback=|| view! { <p class="loading">"Loading net worth…"</p> }>
                {move || {
                    report
                        .get()
                        .map(|result| match result {
                            Err(err) => {
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
                                    .into_any()
                            }
                            Ok(report) if report.lines.is_empty() => {
                                view! {
                                    <p class="empty-state">
                                        <strong>"No balances yet"</strong>
                                        "Add an account and record a transaction to see your net worth."
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(report) => view! { <NetWorthBody report=report /> }.into_any(),
                        })
                }}
            </Suspense>
        </div>
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
    let table_code = code.clone();

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
            let converted = line.converted_amount.clone();
            let balance_amount = line.amount.clone();
            let balance_code = line.currency_code.clone();
            let value_code = table_code.clone();
            view! {
                <tr>
                    <td>{line.currency_code}</td>
                    <td class="num">
                        <Money amount=balance_amount code=balance_code />
                    </td>
                    <td class="num">{rate}</td>
                    <td class="date">{rate_date}</td>
                    <td class="num">
                        {match converted {
                            Some(amount) => {
                                view! { <Money amount=amount code=value_code /> }.into_any()
                            }
                            None => view! { <span class="empty-state">"not available"</span> }.into_any(),
                        }}
                    </td>
                </tr>
            }
        })
        .collect_view();

    view! {
        <div class="stat">
            <span class="stat__label">{format!("Total, valued in {code}")}</span>
            <Money amount=total code=code.clone() figure=true />
        </div>

        {(!complete)
            .then(|| {
                view! {
                    <p class="warning">
                        "Some balances have no exchange rate and are left out of the total."
                    </p>
                }
            })}

        <ScrollableTable caption="Net worth by currency">
            <thead>
                <tr>
                    <th scope="col">"Currency"</th>
                    <th scope="col" class="num">"Balance"</th>
                    <th scope="col" class="num">"Rate"</th>
                    <th scope="col">"Rate date"</th>
                    <th scope="col" class="num">{value_header}</th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </ScrollableTable>

        {rates_as_of
            .map(|ts| {
                let as_of = date_part(&ts).to_owned();
                view! { <p class="field-note">{format!("Exchange rates as of {as_of}")}</p> }
            })}
    }
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
        <div class="filters">
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
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
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

        <div aria-live="polite">
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
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
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
        </div>
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
    let warning = format!(
        "Some amounts have no exchange rate for {code} as of {} and are left out of the totals.",
        date_part(&rates_as_of),
    );
    let footnote = format!("{from} to {to}, valued as of {}", date_part(&rates_as_of));

    view! {
        <div class="stat">
            <span class="stat__label">{format!("Net in {code}")}</span>
            <Money amount=net code=code.clone() figure=true signed=true />
        </div>
        <p class="field-note">
            "Income " <Money amount=total_income code=code.clone() />
            " · Expenses " <Money amount=total_expense code=code.clone() />
        </p>

        {(!complete)
            .then(|| view! { <p class="warning">{warning.clone()}</p> })}

        <IncomeExpenseSection
            heading="Income"
            code=code.clone()
            net_header=net_header.clone()
            lines=income_lines
        />
        <IncomeExpenseSection
            heading="Expenses"
            code=code.clone()
            net_header=net_header.clone()
            lines=expense_lines
        />
        <IncomeExpenseSection
            heading="Not valued"
            code=code.clone()
            net_header=net_header
            lines=unvalued_lines
        />

        <p class="field-note">{footnote}</p>
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

    let caption = format!("{heading} by category");

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
                        <td colspan="3" class="field-note">{parts}</td>
                    </tr>
                }
            });
            view! {
                <tr>
                    <td>{category}</td>
                    <td>{kind}</td>
                    <td class="num">{net}</td>
                </tr>
                {detail}
            }
        })
        .collect_view();

    view! {
        <section>
            <h3>{heading}</h3>
            <ScrollableTable caption=caption>
                <thead>
                    <tr>
                        <th scope="col">"Category"</th>
                        <th scope="col">"Kind"</th>
                        <th scope="col" class="num">{net_header}</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </ScrollableTable>
        </section>
    }
    .into_any()
}
