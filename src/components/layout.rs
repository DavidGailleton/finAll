use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::accounts::api::list_accounts;
use crate::accounts::types::{AccountDto, AccountType};
use crate::auth::api::Logout;
use crate::auth::types::SessionUser;
use crate::balances::api::account_balance;
use crate::components::{Icon, Money, ThemeToggle};

/// App shell for authenticated pages, matched to Sure: a skip link, an 84px
/// icon rail, the accounts sidebar (groups by type, per-row balances), and the
/// routed page content under a sticky top bar.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let logout = ServerAction::<Logout>::new();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if matches!(logout.value().get(), Some(Ok(_))) {
            navigate("/login", Default::default());
        }
    });

    // Provided by `RequireAuth` once the session check resolves.
    let user = use_context::<SessionUser>();
    let who = user.map(|user| user.display_name.unwrap_or(user.email));

    let sidebar_hidden = RwSignal::new(false);

    view! {
        <a class="skip-link" href="#main">"Skip to content"</a>
        <div class="app-shell" class:sidebar-collapsed=move || sidebar_hidden.get()>
            <nav class="rail" aria-label="Primary">
                <a href="/" class="rail__brand" aria-label="finAll — dashboard">"f"</a>
                <ul class="rail__nav">
                    <RailLink href="/" exact=true icon="chart-pie" label="Dashboard" />
                    <RailLink href="/accounts" icon="wallet" label="Accounts" />
                    <RailLink href="/transactions" icon="arrow-left-right" label="Transactions" />
                    <RailLink href="/categories" icon="shapes" label="Categories" />
                    <RailLink href="/merchants" icon="store" label="Merchants" />
                </ul>
                <div class="rail__foot">
                    <ActionForm action=logout>
                        <button
                            type="submit"
                            class="btn btn--icon"
                            aria-label="Log out"
                            disabled=move || logout.pending().get()
                        >
                            <Icon name="log-out" size="sm" />
                        </button>
                    </ActionForm>
                </div>
            </nav>

            <aside class="accounts-sidebar" aria-label="Accounts">
                <div class="accounts-sidebar__head">
                    <h2>"Accounts"</h2>
                    <button
                        type="button"
                        class="btn btn--icon"
                        aria-label="Hide accounts sidebar"
                        on:click=move |_| sidebar_hidden.set(true)
                    >
                        <Icon name="panel-left" size="sm" />
                    </button>
                </div>
                <AccountsSidebar />
                {who.map(|who| view! { <p class="field-note">{who}</p> })}
            </aside>

            <main id="main" class="app-main">
                <div class="topbar">
                    <div class="topbar__left">
                        <button
                            type="button"
                            class="btn btn--icon"
                            aria-label="Toggle accounts sidebar"
                            aria-expanded=move || if sidebar_hidden.get() { "false" } else { "true" }
                            on:click=move |_| sidebar_hidden.update(|hidden| *hidden = !*hidden)
                        >
                            <Icon name="panel-left" size="sm" />
                        </button>
                    </div>
                    <ThemeToggle />
                </div>
                <div class="app-main__body">{children()}</div>
            </main>
        </div>
    }
}

/// One icon-rail nav item (Sure `_nav_item`): indicator bar + icon chip + label.
#[component]
fn RailLink(
    href: &'static str,
    #[prop(optional)] exact: bool,
    icon: &'static str,
    label: &'static str,
) -> impl IntoView {
    view! {
        <li>
            <A href=href exact=exact attr:class="rail-link">
                <span class="rail-link__bar" aria-hidden="true"></span>
                <span class="rail-link__chip">
                    <Icon name=icon size="sm" />
                </span>
                <span class="rail-link__label">{label}</span>
            </A>
        </li>
    }
}

/// Maps an account type to its Lucide icon slug.
fn type_icon(account_type: AccountType) -> &'static str {
    match account_type {
        AccountType::Cash => "banknote",
        AccountType::Bank => "landmark",
        AccountType::Credit => "credit-card",
        AccountType::Investment => "trending-up",
        AccountType::Crypto => "bitcoin",
        AccountType::Loan => "hand-coins",
        AccountType::Other => "circle-help",
    }
}

#[component]
fn AccountsSidebar() -> impl IntoView {
    let accounts = Resource::new(|| (), |_| async move { list_accounts().await });

    view! {
        <Suspense fallback=|| view! { <p class="loading">"Loading accounts…"</p> }>
            {move || {
                accounts
                    .get()
                    .map(|result| match result {
                        Err(_) => {
                            view! { <p class="field-note">"Accounts unavailable"</p> }.into_any()
                        }
                        Ok(list) if list.is_empty() => {
                            view! {
                                <a href="/accounts" class="btn btn--ghost btn--small">
                                    "Add an account"
                                </a>
                            }
                                .into_any()
                        }
                        Ok(list) => {
                            let groups = AccountType::ALL
                                .iter()
                                .filter_map(|account_type| {
                                    let account_type = *account_type;
                                    let members: Vec<AccountDto> = list
                                        .iter()
                                        .filter(|a| a.account_type == account_type)
                                        .cloned()
                                        .collect();
                                    (!members.is_empty())
                                        .then(|| {
                                            view! {
                                                <SidebarGroup
                                                    label=account_type.label()
                                                    icon=type_icon(account_type)
                                                    accounts=members
                                                />
                                            }
                                        })
                                })
                                .collect_view();
                            view! {
                                <div class="stack">
                                    {groups}
                                    <a href="/accounts" class="btn btn--ghost btn--small">
                                        "New account"
                                    </a>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}

#[component]
fn SidebarGroup(
    label: &'static str,
    icon: &'static str,
    accounts: Vec<AccountDto>,
) -> impl IntoView {
    let rows = accounts
        .into_iter()
        .map(|account| view! { <SidebarAccountRow account=account icon=icon /> })
        .collect_view();

    view! {
        <details class="acct-group" open>
            <summary>
                <Icon name="chevron-right" size="sm" class="icon--chev" />
                <span>{label}</span>
            </summary>
            <div class="acct-group__body">{rows}</div>
        </details>
    }
}

#[component]
fn SidebarAccountRow(account: AccountDto, icon: &'static str) -> impl IntoView {
    let account_id = account.id.clone();
    let href = format!("/accounts/{}", account.id);
    let name = account.account_name.clone();
    let label = account.account_type.label();

    let balance = Resource::new(
        move || account_id.clone(),
        |id| async move { account_balance(id).await },
    );

    view! {
        <A href=href attr:class="acct-row">
            <span class="filled-icon filled-icon--sm">
                <Icon name=icon size="sm" />
            </span>
            <span class="acct-row__main">
                <span class="acct-row__name">{name}</span>
                <span class="acct-row__sub">{label}</span>
            </span>
            <Suspense fallback=|| view! { <span class="acct-row__bal loading">"…"</span> }>
                {move || {
                    balance
                        .get()
                        .map(|result| match result {
                            Ok(balance) => {
                                view! {
                                    <span class="acct-row__bal">
                                        <Money amount=balance.amount code=balance.currency_code />
                                    </span>
                                }
                                    .into_any()
                            }
                            Err(_) => {
                                view! { <span class="acct-row__bal field-note">"—"</span> }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        </A>
    }
}
