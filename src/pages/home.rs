//! `/` — the dashboard. One display currency and one period drive the whole
//! screen: a net-worth hero (figure + assets/liabilities breakdown), a money
//! flow panel (12-month trend + income/expense totals for the chosen period),
//! and a cashflow Sankey diagram (Bank + Cash accounts only).
//!
//! Follows the `Resource` + `<Suspense>` loading/empty/error pattern used by the
//! other pages (see [`crate::pages::accounts`]). The period-driven panels stay
//! idle until a preset is picked, so the first render never reads the clock —
//! the period itself is a `[from, to]` pair resolved server-side, on demand,
//! by [`crate::periods::api::resolve_period`] when the visitor picks one of
//! the [`PeriodPreset`] buttons.

use leptos::prelude::*;
use leptos_meta::Title;

use crate::assets::currency::api::list_currencies;
use crate::cashflow::api::cashflow_sankey;
use crate::components::{
    BarList, BarRow, IncomeExpenseBars, Layout, Money, PageHeader, Panel, Pill, Sankey,
    ScrollableTable,
};
use crate::money_flow::api::money_flow_report;
use crate::money_flow::types::{MoneyFlowMonthDto, MoneyFlowReportDto};
use crate::net_worth::api::net_worth;
use crate::net_worth::types::{AccountGroupDto, ClassificationGroupDto, NetWorthReportDto};
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;
use crate::periods::api::resolve_period;
use crate::periods::types::PeriodPreset;

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

    // Resolving a preset is an explicit, on-demand server round trip (see
    // `crate::periods`) — nothing here reads a clock on first render.
    let selected_preset = RwSignal::new(None::<PeriodPreset>);
    let period_action = Action::new(|preset: &PeriodPreset| {
        let preset = *preset;
        async move { resolve_period(preset).await }
    });
    Effect::new(move |_| {
        if let Some(Ok(range)) = period_action.value().get() {
            from.set(range.from);
            to.set(range.to);
        }
    });

    let currencies = Resource::new(|| (), |_| async move { list_currencies().await });

    let net_worth_report = Resource::new(
        move || currency.get(),
        |code| async move { net_worth(code).await },
    );

    // Idle until both bounds are chosen — no clock read on first render.
    let money_flow_res = Resource::new(
        move || {
            let (f, t) = (from.get(), to.get());
            (!f.is_empty() && !t.is_empty()).then_some((currency.get(), f, t))
        },
        |params| async move {
            match params {
                Some((code, f, t)) => Some(money_flow_report(code, f, t).await),
                None => None,
            }
        },
    );

    let cashflow_res = Resource::new(
        move || {
            let (f, t) = (from.get(), to.get());
            (!f.is_empty() && !t.is_empty()).then_some((currency.get(), f, t))
        },
        |params| async move {
            match params {
                Some((code, f, t)) => Some(cashflow_sankey(code, f, t).await),
                None => None,
            }
        },
    );

    // The signed net for the current period, if the report has resolved.
    let period_net = Signal::derive(move || match money_flow_res.get() {
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
                                                        decimals=2
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
                                            <Money amount=net code=code decimals=2 signed=true />
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
                                    Ok(report)
                                        if report
                                            .classifications
                                            .iter()
                                            .all(|c| c.account_groups.is_empty()) =>
                                    {
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

            <Panel span=12 title="Money flow" widget=true>
                <div class="filters">
                    <div
                        class="period-picker"
                        role="group"
                        aria-label="Choose a period"
                    >
                        {PeriodPreset::ALL
                            .into_iter()
                            .map(|preset| {
                                let pressed =
                                    move || selected_preset.get() == Some(preset);
                                view! {
                                    <button
                                        type="button"
                                        class="period-picker__option"
                                        aria-pressed=move || pressed().to_string()
                                        disabled=move || period_action.pending().get()
                                        on:click=move |_| {
                                            selected_preset.set(Some(preset));
                                            period_action.dispatch(preset);
                                        }
                                    >
                                        {preset.label()}
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                    <p class="field-note">
                        "Valued in " {move || currency.get()} ", set in the panel above. The trend"
                        " below always shows the 12 months ending in the chosen period."
                    </p>
                    {move || {
                        period_action
                            .value()
                            .get()
                            .and_then(|result| result.err())
                            .map(|err| {
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
                            })
                    }}
                </div>

                <div aria-live="polite">
                    <Suspense fallback=|| view! { <p class="loading">"Loading report\u{2026}"</p> }>
                        {move || {
                            money_flow_res
                                .get()
                                .map(|slot| match slot {
                                    None => {
                                        view! {
                                            <p class="empty-state">
                                                "Choose a period above to see the breakdown."
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
                                    Some(Ok(report)) => {
                                        view! { <MoneyFlowBody report=report /> }.into_any()
                                    }
                                })
                        }}
                    </Suspense>
                </div>
            </Panel>

            <Panel span=12 title="Cashflow" note="Bank and Cash accounts only" widget=true>
                <div aria-live="polite">
                    <Suspense fallback=|| view! { <p class="loading">"Loading cashflow\u{2026}"</p> }>
                        {move || {
                            cashflow_res
                                .get()
                                .map(|slot| match slot {
                                    None => {
                                        view! {
                                            <p class="empty-state">
                                                "Choose a period above to see the breakdown."
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
                                    Some(Ok(sankey)) => view! { <Sankey data=sankey /> }.into_any(),
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

/// `"2026-01"` -> `"Jan 2026"`. Falls back to the raw string for anything
/// that doesn't parse — display formatting only, never used for a value.
fn month_label(month: &str) -> String {
    let mut parts = month.splitn(2, '-');
    let (Some(year), Some(number)) = (parts.next(), parts.next()) else {
        return month.to_owned();
    };
    let name = match number {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return month.to_owned(),
    };
    format!("{name} {year}")
}

/// Parse a decimal-string magnitude to `f64` for a **pixel size only**; every
/// displayed number stays the original server string, same convention as
/// `BarList`/`IncomeExpenseBars`.
fn magnitude(s: &str) -> f64 {
    s.trim_start_matches('-')
        .parse::<f64>()
        .unwrap_or(0.0)
        .abs()
}

#[component]
fn NetWorthBody(report: NetWorthReportDto) -> impl IntoView {
    let NetWorthReportDto {
        display_currency_code,
        display_minor_units: _,
        total: _,
        complete,
        rates_as_of,
        classifications,
    } = report;

    let code = display_currency_code;
    let value_header = format!("Value in {code}");

    let sections = classifications
        .into_iter()
        .filter(|group| !group.account_groups.is_empty())
        .map(|group| {
            let ClassificationGroupDto {
                classification,
                total,
                account_groups,
            } = group;
            let bar_code = code.clone();
            let bar_rows: Vec<BarRow> = account_groups
                .iter()
                .map(|g| BarRow {
                    label: g.account_type.label().to_owned(),
                    amount: g.total.clone(),
                    code: bar_code.clone(),
                })
                .collect();

            let heading_code = code.clone();
            let groups_view = account_groups
                .into_iter()
                .map(|account_group| account_group_view(account_group, &code, &value_header))
                .collect_view();

            view! {
                <section class="stack">
                    <div class="cluster">
                        <h3>{classification.label()}</h3>
                        <Money amount=total code=heading_code decimals=2 />
                    </div>
                    <BarList rows=bar_rows />
                    {groups_view}
                </section>
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

        {sections}

        {rates_as_of
            .map(|ts| {
                let as_of = date_part(&ts).to_owned();
                view! { <p class="chart-note">{format!("Exchange rates as of {as_of}")}</p> }
            })}
    }
}

/// One account-type group's `<details>` block: its accounts, native balance,
/// and converted value.
fn account_group_view(
    group: AccountGroupDto,
    display_code: &str,
    value_header: &str,
) -> impl IntoView {
    let AccountGroupDto {
        account_type,
        total,
        accounts,
    } = group;

    let caption = format!("{} accounts", account_type.label());
    let value_header = value_header.to_owned();

    let rows = accounts
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
            let value_code = display_code.to_owned();
            view! {
                <tr>
                    <td>{line.account_name}</td>
                    <td class="num">
                        <Money amount=balance_amount code=balance_code decimals=2 />
                    </td>
                    <td class="num">{rate}</td>
                    <td class="date">{rate_date}</td>
                    <td class="num">
                        {match converted {
                            Some(amount) => {
                                view! {
                                    <Money amount=amount code=value_code decimals=2 />
                                }
                                    .into_any()
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
        <details class="data-detail">
            <summary>
                {account_type.label()} " · "
                <Money amount=total code=display_code.to_owned() decimals=2 />
            </summary>
            <ScrollableTable caption=caption>
                <thead>
                    <tr>
                        <th scope="col">"Account"</th>
                        <th scope="col" class="num">"Balance"</th>
                        <th scope="col" class="num">"Rate"</th>
                        <th scope="col">"Rate date"</th>
                        <th scope="col" class="num">{value_header}</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </ScrollableTable>
        </details>
    }
}

#[component]
fn MoneyFlowBody(report: MoneyFlowReportDto) -> impl IntoView {
    let MoneyFlowReportDto {
        display_currency_code,
        display_minor_units: _,
        months,
        total_income,
        total_expense,
        net,
        complete,
    } = report;

    let code = display_currency_code;
    let net_tone = if net.starts_with('-') { "down" } else { "up" };

    let max = months
        .iter()
        .flat_map(|m| [magnitude(&m.income), magnitude(&m.expense)])
        .fold(0.0_f64, f64::max);
    let denom = if max > 0.0 { max } else { 1.0 };

    let bars = months
        .into_iter()
        .map(
            |MoneyFlowMonthDto {
                 month,
                 income,
                 expense,
             }| {
                let income_pct = (magnitude(&income) / denom * 100.0).clamp(0.0, 100.0);
                let expense_pct = (magnitude(&expense) / denom * 100.0).clamp(0.0, 100.0);
                let label = month_label(&month);
                view! {
                    <li class="moneyflow__month">
                        <div class="moneyflow__bars" aria-hidden="true">
                            <span
                                class="moneyflow__bar moneyflow__bar--in chart-grow-y"
                                style=format!("height:{income_pct:.1}%")
                            ></span>
                            <span
                                class="moneyflow__bar moneyflow__bar--out chart-grow-y"
                                style=format!("height:{expense_pct:.1}%")
                            ></span>
                        </div>
                        <span class="moneyflow__label">{label}</span>
                    </li>
                }
            },
        )
        .collect_view();

    view! {
        <ul class="moneyflow__trend">{bars}</ul>

        <IncomeExpenseBars income=total_income expense=total_expense code=code.clone() />

        <div class="cluster">
            <Pill tone=net_tone>
                <Money amount=net code=code decimals=2 signed=true />
                " net"
            </Pill>
        </div>

        {(!complete)
            .then(|| {
                view! {
                    <p class="warning">
                        "Some amounts have no exchange rate and are left out of the totals."
                    </p>
                }
            })}
    }
}
