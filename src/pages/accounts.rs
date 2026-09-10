//! `/accounts` (list) and `/accounts/:id` (detail, with delete).
//!
//! Both pages follow the `Resource` + `<Suspense>` pattern from
//! [`crate::pages::guard`], and render explicit loading, empty, and error
//! states. The per-account transaction ledger is the shared
//! [`crate::pages::ledger`] table.

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::accounts::api::{get_account, list_accounts, CreateAccount, DeleteAccount};
use crate::accounts::types::{AccountDto, AccountType};
use crate::assets::currency::api::list_currencies;
use crate::balances::api::account_balance;
use crate::components::{
    Button, FormError, Icon, Layout, Money, PageHeader, Panel, SelectField, TextField,
};
use crate::pages::guard::RequireAuth;
use crate::pages::ledger::{
    AccountContext, AddTransactionForm, LedgerActions, LedgerFilter, LedgerTable,
};
use crate::pages::server_error_message;

/// `/accounts`
#[component]
pub fn AccountsPage() -> impl IntoView {
    view! {
        <Title text="Accounts · finAll" />
        <RequireAuth>
            <Layout>
                <PageHeader
                    title="Accounts"
                    description="Every account you track, cash to crypto."
                />
                <AccountsList />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn AccountsList() -> impl IntoView {
    let accounts = Resource::new(|| (), |_| async move { list_accounts().await });

    view! {
        <div class="bento">
            <Panel span=8 title="Your accounts">
                <Suspense fallback=|| {
                    view! { <p class="loading">"Loading accounts…"</p> }
                }>
                    {move || {
                        accounts
                            .get()
                            .map(|result| match result {
                                Err(err) => {
                                    view! { <p class="form-error">{server_error_message(&err)}</p> }
                                        .into_any()
                                }
                                Ok(list) if list.is_empty() => {
                                    view! {
                                        <p class="empty-state">
                                            <strong>"No accounts yet"</strong>
                                            "Add your first account to start tracking balances and transactions."
                                        </p>
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
                                                    let count = members.len();
                                                    view! {
                                                        <AccountGroup
                                                            label=account_type.label()
                                                            icon=type_icon(account_type)
                                                            count=count
                                                            accounts=members
                                                        />
                                                    }
                                                })
                                        })
                                        .collect_view();
                                    view! { <div class="stack">{groups}</div> }.into_any()
                                }
                            })
                    }}
                </Suspense>
            </Panel>
            <Panel title="Add account" span=4>
                <AddAccountForm />
            </Panel>
        </div>
    }
}

/// Maps an account type to its Lucide icon slug (mirrors `layout::type_icon`).
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

/// One account-type section (Sure `_account_groups`): an inset well with a
/// `TYPE · n` mini-header over a ruled list of rows. Balances stay in each
/// account's own currency, so there is no cross-currency subtotal.
#[component]
fn AccountGroup(
    label: &'static str,
    icon: &'static str,
    count: usize,
    accounts: Vec<AccountDto>,
) -> impl IntoView {
    let rows = accounts
        .into_iter()
        .map(|account| view! { <AccountRow account=account icon=icon /> })
        .collect_view();

    view! {
        <section class="inset-well">
            <div class="inset-well__head">
                <span>{label}</span>
                <span class="sep">"·"</span>
                <span>{count}</span>
            </div>
            <div class="inset-well__list">{rows}</div>
        </section>
    }
}

/// One account row: a link to its detail page and its current balance, loaded on
/// its own so a missing exchange rate degrades just this row.
#[component]
fn AccountRow(account: AccountDto, icon: &'static str) -> impl IntoView {
    let account_id = account.id.clone();
    let name = account.account_name.clone();
    let sub = account.account_type.label();
    let href = format!("/accounts/{}", account.id);

    let balance = Resource::new(
        move || account_id.clone(),
        |id| async move { account_balance(id).await },
    );

    view! {
        <div class="acct-list-row">
            <span class="filled-icon filled-icon--md">
                <Icon name=icon size="sm" />
            </span>
            <span class="acct-list-row__main">
                <a href=href>{name}</a>
                <span class="acct-list-row__sub">{sub}</span>
            </span>
            <Suspense fallback=|| {
                view! { <span class="acct-list-row__bal loading">"…"</span> }
            }>
                {move || {
                    balance
                        .get()
                        .map(|result| match result {
                            Ok(balance) => {
                                view! {
                                    <span class="acct-list-row__bal">
                                        <Money
                                            amount=balance.amount
                                            code=balance.currency_code
                                        />
                                    </span>
                                }
                                    .into_any()
                            }
                            Err(_) => {
                                view! {
                                    <span class="acct-list-row__bal field-note">
                                        "balance unavailable"
                                    </span>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        </div>
    }
}

#[component]
fn AddAccountForm() -> impl IntoView {
    let create = ServerAction::<CreateAccount>::new();
    let navigate = use_navigate();
    let adding = RwSignal::new(false);

    // Go to the new account's detail page once it is created.
    Effect::new(move |_| {
        if let Some(Ok(account)) = create.value().get() {
            navigate(&format!("/accounts/{}", account.id), Default::default());
        }
    });

    let create_error = Signal::derive(move || match create.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    // Only fetch the currency list once the form is opened.
    let currencies = Resource::new(
        move || adding.get(),
        |adding| async move {
            if adding {
                list_currencies().await
            } else {
                Ok(Vec::new())
            }
        },
    );

    view! {
        <Show
            when=move || adding.get()
            fallback=move || {
                view! {
                    <button
                        type="button"
                        class="btn btn--secondary"
                        aria-expanded="false"
                        on:click=move |_| adding.set(true)
                    >
                        "Add account"
                    </button>
                }
            }
        >
            <Suspense fallback=|| {
                view! { <p class="loading">"Loading currencies…"</p> }
            }>
                {move || {
                    currencies
                        .get()
                        .map(|result| match result {
                            Err(err) => {
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
                                    .into_any()
                            }
                            Ok(currency_list) => {
                                view! {
                                    <ActionForm action=create>
                                        <TextField label="Name" name="account_name" />
                                        <SelectField label="Type" name="account_type">
                                            <option value="" disabled selected>
                                                "Select a type"
                                            </option>
                                            {AccountType::ALL
                                                .iter()
                                                .map(|account_type| {
                                                    let account_type = *account_type;
                                                    view! {
                                                        <option value=account_type.as_db_str()>
                                                            {account_type.label()}
                                                        </option>
                                                    }
                                                })
                                                .collect_view()}
                                        </SelectField>
                                        <SelectField label="Currency" name="default_asset_id">
                                            <option value="" disabled selected>
                                                "Select a currency"
                                            </option>
                                            {currency_list
                                                .into_iter()
                                                .map(|currency| {
                                                    view! {
                                                        <option value=currency.id>
                                                            {format!(
                                                                "{} — {}",
                                                                currency.alphabetic_code,
                                                                currency.currency_name,
                                                            )}
                                                        </option>
                                                    }
                                                })
                                                .collect_view()}
                                        </SelectField>
                                        <FormError message=create_error />
                                        <div class="form-actions">
                                            <Button pending=create.pending()>"Create account"</Button>
                                            <button
                                                type="button"
                                                class="btn btn--secondary"
                                                on:click=move |_| adding.set(false)
                                            >
                                                "Cancel"
                                            </button>
                                        </div>
                                    </ActionForm>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        </Show>
    }
}

/// `/accounts/:id`
#[component]
pub fn AccountDetailPage() -> impl IntoView {
    view! {
        <Title text="Account · finAll" />
        <RequireAuth>
            <Layout>
                <AccountDetail />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn AccountDetail() -> impl IntoView {
    let params = use_params_map();
    let delete = ServerAction::<DeleteAccount>::new();
    let navigate = use_navigate();

    // One bundle of ledger actions, shared by the transactions table and the
    // balance line so both refetch after any transaction or transfer change.
    let ledger_actions = LedgerActions::new();

    // Once the account is deleted, leave the detail page for the list.
    Effect::new(move |_| {
        if matches!(delete.value().get(), Some(Ok(_))) {
            navigate("/accounts", Default::default());
        }
    });

    let account = Resource::new(
        move || params.read().get("id").unwrap_or_default(),
        |id| async move { get_account(id).await },
    );

    // The account's total, valued in its own default currency. Refetched
    // whenever a ledger write lands.
    let balance = Resource::new(
        move || {
            (
                params.read().get("id").unwrap_or_default(),
                ledger_actions.create_transaction.version().get(),
                ledger_actions.update_transaction.version().get(),
                ledger_actions.delete_transaction.version().get(),
                ledger_actions.link_transfer.version().get(),
                ledger_actions.unlink_transfer.version().get(),
            )
        },
        |(id, ..)| async move { account_balance(id).await },
    );

    let delete_error = Signal::derive(move || match delete.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <A href="/accounts" attr:class="back-link">"Back to accounts"</A>
        <Suspense fallback=|| view! { <p class="loading">"Loading account…"</p> }>
            {move || {
                account
                    .get()
                    .map(|result| match result {
                        Err(err) => {
                            view! { <p class="form-error">{server_error_message(&err)}</p> }
                                .into_any()
                        }
                        Ok(account) => {
                            let account_id = account.id.clone();
                            let transactions_account_id = account.id.clone();
                            let transactions_default_asset_id = account.default_asset_id.clone();
                            let current_type = account.account_type;
                            view! {
                                <Title text=format!("{} · finAll", account.account_name) />
                                <PageHeader title=account.account_name.clone() />
                                <div class="bento">
                                    <Panel title="Balance" span=5 primary=true>
                                        <span class="badge">{current_type.label()}</span>
                                        <div aria-live="polite">
                                            <Suspense fallback=|| {
                                                view! { <p class="loading">"Loading balance…"</p> }
                                            }>
                                                {move || {
                                                    balance
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
                                                            Ok(balance) => {
                                                                view! {
                                                                    <Money
                                                                        amount=balance.amount
                                                                        code=balance.currency_code
                                                                        figure=true
                                                                    />
                                                                }
                                                                    .into_any()
                                                            }
                                                        })
                                                }}
                                            </Suspense>
                                        </div>
                                    </Panel>

                                    <Panel title="Settings" span=7>
                                        <DeleteAccountForm
                                            account_id=account_id
                                            action=delete
                                            error=delete_error
                                        />
                                    </Panel>

                                    <Panel title="Transactions" span=12>
                                        <TransactionsList
                                            account_id=transactions_account_id
                                            default_asset_id=transactions_default_asset_id
                                            account_type=current_type
                                            actions=ledger_actions
                                        />
                                    </Panel>
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
fn DeleteAccountForm(
    account_id: String,
    action: ServerAction<DeleteAccount>,
    error: Signal<Option<String>>,
) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let account_id = RwSignal::new(account_id);

    view! {
        <Show
            when=move || confirming.get()
            fallback=move || {
                view! {
                    <button
                        type="button"
                        class="btn btn--danger"
                        aria-expanded="false"
                        on:click=move |_| confirming.set(true)
                    >
                        "Delete account"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <input type="hidden" name="id" value=move || account_id.get() />
                <p class="field-note">
                    "This removes the account and its transactions. It can't be undone."
                </p>
                <FormError message=error />
                <div class="form-actions">
                    <Button variant="danger" pending=action.pending()>"Delete permanently"</Button>
                    <button
                        type="button"
                        class="btn btn--secondary"
                        on:click=move |_| confirming.set(false)
                    >
                        "Cancel"
                    </button>
                </div>
            </ActionForm>
        </Show>
    }
}

/// The transaction ledger for one account: the shared [`LedgerTable`] fixed to
/// this account (no Account column), plus the "Add transaction" form.
#[component]
fn TransactionsList(
    account_id: String,
    default_asset_id: String,
    account_type: AccountType,
    actions: LedgerActions,
) -> impl IntoView {
    let account_context = AccountContext {
        id: account_id.clone(),
        default_asset_id,
        account_type,
    };
    let filter_account_id = account_id;
    let filter = LedgerFilter {
        account_id: Signal::derive(move || filter_account_id.clone()),
        from: Signal::derive(String::new),
        to: Signal::derive(String::new),
    };

    view! {
        <div class="ledger-actions">
            <AddTransactionForm fixed_account=account_context action=actions.create_transaction />
        </div>
        <LedgerTable filter=filter show_account=false actions=actions />
    }
}
