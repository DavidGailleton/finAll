//! `/` — the dashboard. One display currency and one period drive the whole
//! screen: a net-worth hero (figure + composition bars + this-period pill) and
//! an income-vs-expense panel (bars + category breakdown).
//!
//! Follows the `Resource` + `<Suspense>` loading/empty/error pattern used by the
//! other pages (see [`crate::pages::accounts`]). The income/expense report stays
//! idle until both dates are chosen, so the first render never reads the clock.

use leptos::prelude::*;
use leptos_meta::Title;

use crate::assets::currency::api::list_currencies;
use crate::components::{
    BarList, BarRow, IncomeExpenseBars, Layout, Money, PageHeader, Panel, Pill, ScrollableTable,
};
use crate::income_expense::api::income_expense_report;
use crate::income_expense::types::{IncomeExpenseLineDto, IncomeExpenseReportDto};
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
                <Dashboard />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn Dashboard() -> impl IntoView {
    // One currency and one period for the whole dashboard. Not persisted (no
    // user-settings store yet); currency defaults to EUR, the anchor currency.
    let currency = RwSignal::new("EUR".to_owned());
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());

    let currencies = Resource::new(|| (), |_| async move { list_currencies().await });

    let net_worth_report = Resource::new(
        move || currency.get(),
        |code| async move { net_worth(code).await },
    );

    // Idle until both bounds are chosen — no clock read on first render.
    let ie_report = Resource::new(
        move || {
            let (f, t) = (from.get(), to.get());
            (!f.is_empty() && !t.is_empty()).then_some((currency.get(), f, t))
        },
        |params| async move {
            match params {
                Some((code, f, t)) => Some(income_expense_report(code, f, t).await),
                None => None,
            }
        },
    );

    // The signed net for the current period, if the report has resolved.
    let period_net = Signal::derive(move || match ie_report.get() {
        Some(Some(Ok(report))) => Some((report.net.clone(), report.display_currency_code.clone())),
        _ => None,
    });

    view! {
        <PageHeader
            title="Dashboard"
            description="Where your money stands, and how it moved over a period you choose."
        />
        <div class="bento">
            <Panel span=12 title="Net worth" widget=true>
                <div class="hero">
                    <div class="hero__row">
                        <div aria-live="polite">
                            <Suspense fallback=|| {
                                view! { <span class="loading">"Loading\u{2026}"</span> }
                            }>
                                {move || {
                                    net_worth_report
                                        .get()
                                        .map(|result| match result {
                                            Err(err) => {
                                                view! {
                                                    <p class="form-error">
                                                        {server_error_message(&err)}
                                                    </p>
                                                }
                                                    .into_any()
                                            }
                                            Ok(report) => {
                                                view! {
                                                    <Money
                                                        amount=report.total
                                                        code=report.display_currency_code
                                                        figure=true
                                                    />
                                                }
                                                    .into_any()
                                            }
                                        })
                                }}
                            </Suspense>
                        </div>
                        {move || {
                            period_net
                                .get()
                                .map(|(net, code)| {
                                    let tone = if net.starts_with('-') { "down" } else { "up" };
                                    view! {
                                        <Pill tone=tone>
                                            <Money amount=net code=code signed=true />
                                            " net this period"
                                        </Pill>
                                    }
                                })
                        }}
                    </div>
                </div>

                <Suspense fallback=|| view! { <p class="loading">"Loading currencies\u{2026}"</p> }>
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
                                            <label for="dashboard-currency">"Display currency"</label>
                                            <select
                                                id="dashboard-currency"
                                                prop:value=move || currency.get()
                                                on:change=move |ev| {
                                                    currency.set(event_target_value(&ev))
                                                }
                                            >
                                                {list
                                                    .into_iter()
                                                    .map(|c| {
                                                        view! {
                                                            <option value=c.alphabetic_code.clone()>
                                                                {format!(
                                                                    "{} — {}",
                                                                    c.alphabetic_code,
                                                                    c.currency_name,
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
                    <Suspense fallback=|| ()>
                        {move || {
                            net_worth_report
                                .get()
                                .map(|result| match result {
                                    Err(_) => ().into_any(),
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
            </Panel>

            <Panel span=12 title="Income & expense" widget=true>
                <div class="filters">
                    <div class="field">
                        <label for="ie-from">"From"</label>
                        <input
                            id="ie-from"
                            type="date"
                            prop:value=move || from.get()
                            on:change=move |ev| from.set(event_target_value(&ev))
                        />
                    </div>
                    <div class="field">
                        <label for="ie-to">"To"</label>
                        <input
                            id="ie-to"
                            type="date"
                            prop:value=move || to.get()
                            on:change=move |ev| to.set(event_target_value(&ev))
                        />
                    </div>
                    <p class="field-note">
                        "Valued in " {move || currency.get()} ", set in the panel above."
                    </p>
                </div>

                <div aria-live="polite">
                    <Suspense fallback=|| view! { <p class="loading">"Loading report\u{2026}"</p> }>
                        {move || {
                            ie_report
                                .get()
                                .map(|slot| match slot {
                                    None => {
                                        view! {
                                            <p class="empty-state">
                                                "Choose a start and an end date to see the breakdown."
                                            </p>
                                        }
                                            .into_any()
                                    }
                                    Some(Err(err)) => {
                                        view! {
                                            <p class="form-error">{server_error_message(&err)}</p>
                                        }
                                            .into_any()
                                    }
                                    Some(Ok(report))
                                        if report.income_lines.is_empty()
                                            && report.expense_lines.is_empty()
                                            && report.unvalued_lines.is_empty() =>
                                    {
                                        view! {
                                            <p class="empty-state">
                                                "No income or expenses in this period."
                                            </p>
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
            </Panel>
        </div>
    }
}

/// Turn an RFC 3339 timestamp into its `YYYY-MM-DD` date portion.
fn date_part(timestamp: &str) -> &str {
    timestamp.split('T').next().unwrap_or(timestamp)
}

#[component]
fn NetWorthBody(report: NetWorthReportDto) -> impl IntoView {
    let NetWorthReportDto {
        display_currency_code,
        display_minor_units: _,
        total: _,
        complete,
        rates_as_of,
        lines,
    } = report;

    let code = display_currency_code;
    let value_header = format!("Value in {code}");
    let table_code = code.clone();

    let bar_rows: Vec<BarRow> = lines
        .iter()
        .filter_map(|line| {
            line.converted_amount.clone().map(|amount| BarRow {
                label: line.currency_code.clone(),
                amount,
                code: code.clone(),
            })
        })
        .collect();

    let table_rows = lines
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
                            None => {
                                view! { <span class="empty-state">"not available"</span> }.into_any()
                            }
                        }}
                    </td>
                </tr>
            }
        })
        .collect_view();

    view! {
        {(!complete)
            .then(|| {
                view! {
                    <p class="warning">
                        "Some balances have no exchange rate and are left out of the total."
                    </p>
                }
            })}

        {(!bar_rows.is_empty()).then(|| view! { <BarList rows=bar_rows /> })}

        <details class="data-detail">
            <summary>"Full breakdown by currency"</summary>
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
                <tbody>{table_rows}</tbody>
            </ScrollableTable>
            {rates_as_of
                .map(|ts| {
                    let as_of = date_part(&ts).to_owned();
                    view! {
                        <p class="chart-note">{format!("Exchange rates as of {as_of}")}</p>
                    }
                })}
        </details>
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
    let net_tone = if net.starts_with('-') { "down" } else { "up" };
    let warning = format!(
        "Some amounts have no exchange rate for {code} as of {} and are left out of the totals.",
        date_part(&rates_as_of),
    );
    let footnote = format!("{from} to {to}, valued as of {}", date_part(&rates_as_of));

    let expense_bars: Vec<BarRow> = category_bars(&expense_lines, &code);
    let pill_code = code.clone();

    view! {
        <IncomeExpenseBars income=total_income expense=total_expense code=code.clone() />

        <div class="cluster">
            <Pill tone=net_tone>
                <Money amount=net code=pill_code signed=true />
                " net"
            </Pill>
        </div>

        {(!complete).then(|| view! { <p class="warning">{warning.clone()}</p> })}

        {(!expense_bars.is_empty())
            .then(|| {
                view! {
                    <div class="stack">
                        <span class="stat__label">"Where it went"</span>
                        <BarList rows=expense_bars />
                    </div>
                }
            })}

        <details class="data-detail">
            <summary>"Full breakdown by category"</summary>
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
            <p class="chart-note">{footnote}</p>
        </details>
    }
}

/// Build bar rows from category lines that have a converted net value.
fn category_bars(lines: &[IncomeExpenseLineDto], code: &str) -> Vec<BarRow> {
    lines
        .iter()
        .filter_map(|line| {
            line.converted_net.clone().map(|amount| {
                let label = line.category_name.clone().unwrap_or_else(|| {
                    if line.category_deleted {
                        "(deleted category)".to_owned()
                    } else {
                        "Uncategorised".to_owned()
                    }
                });
                BarRow {
                    label,
                    amount,
                    code: code.to_owned(),
                }
            })
        })
        .collect()
}

#[component]
fn IncomeExpenseSection(
    heading: &'static str,
    code: String,
    net_header: String,
    lines: Vec<IncomeExpenseLineDto>,
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
