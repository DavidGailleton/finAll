//! `/accounts` (list) and `/accounts/:id` (detail, with edit and delete).
//!
//! Both pages follow the `Resource` + `<Suspense>` pattern from
//! [`crate::pages::guard`], and render explicit loading, empty, and error
//! states. The per-account transaction ledger is the shared
//! [`crate::pages::ledger`] table.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::accounts::api::{
    get_account, list_accounts, CreateAccount, DeleteAccount, UpdateAccount,
};
use crate::accounts::types::AccountType;
use crate::assets::currency::api::list_currencies;
use crate::balances::api::account_balance;
use crate::components::{Button, FormError, Layout, SelectField, TextField};
use crate::pages::guard::RequireAuth;
use crate::pages::ledger::{
    AccountContext, AddTransactionForm, AddTransferForm, LedgerActions, LedgerFilter, LedgerTable,
};
use crate::pages::server_error_message;

/// `/accounts`
#[component]
pub fn AccountsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <Layout>
                <AccountsList />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn AccountsList() -> impl IntoView {
    let accounts = Resource::new(|| (), |_| async move { list_accounts().await });

    view! {
        <h1>"Accounts"</h1>
        <Suspense fallback=|| view! { <p class="loading">"Loading accounts…"</p> }>
            {move || {
                accounts
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
                        Ok(list) if list.is_empty() => {
                            view! {
                                <p class="empty-state">"You don't have any accounts yet."</p>
                            }
                                .into_any()
                        }
                        Ok(list) => {
                            view! {
                                <ul class="accounts-list">
                                    {list
                                        .into_iter()
                                        .map(|account| {
                                            view! {
                                                <li class="account-row">
                                                    <A href=format!(
                                                        "/accounts/{}",
                                                        account.id,
                                                    )>{account.account_name}</A>
                                                    <span class="account-type">
                                                        {account.account_type.label()}
                                                    </span>
                                                </li>
                                            }
                                        })
                                        .collect_view()}
                                </ul>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
        <AddAccountForm />
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
        <div class="add-account">
            <Show
                when=move || adding.get()
                fallback=move || {
                    view! {
                        <button type="button" class="btn" on:click=move |_| adding.set(true)>
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
                                    view! {
                                        <p class="form-error" role="alert">
                                            {server_error_message(&err)}
                                        </p>
                                    }
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
                                            <Button pending=create.pending()>"Create account"</Button>
                                            <button
                                                type="button"
                                                class="btn"
                                                on:click=move |_| adding.set(false)
                                            >
                                                "Cancel"
                                            </button>
                                        </ActionForm>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </Suspense>
            </Show>
        </div>
    }
}

/// `/accounts/:id`
#[component]
pub fn AccountDetailPage() -> impl IntoView {
    view! {
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
    let edit = ServerAction::<UpdateAccount>::new();
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

    // Refetch after a successful edit by depending on the action's version.
    let account = Resource::new(
        move || {
            (
                params.read().get("id").unwrap_or_default(),
                edit.version().get(),
            )
        },
        |(id, _)| async move { get_account(id).await },
    );

    // The account's total, valued in its own default currency. Refetched
    // whenever a ledger write lands (the account edit form never changes the
    // balance — the default currency is immutable — so it is not a key here).
    let balance = Resource::new(
        move || {
            (
                params.read().get("id").unwrap_or_default(),
                ledger_actions.create_transaction.version().get(),
                ledger_actions.update_transaction.version().get(),
                ledger_actions.delete_transaction.version().get(),
                ledger_actions.create_transfer.version().get(),
                ledger_actions.update_transfer.version().get(),
                ledger_actions.void_transfer.version().get(),
            )
        },
        |(id, ..)| async move { account_balance(id).await },
    );

    let edit_error = Signal::derive(move || match edit.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });
    let delete_error = Signal::derive(move || match delete.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <p>
            <A href="/accounts">"← Accounts"</A>
        </p>
        <Suspense fallback=|| view! { <p class="loading">"Loading account…"</p> }>
            {move || {
                account
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
                        Ok(account) => {
                            let account_id = account.id.clone();
                            let transactions_account_id = account.id.clone();
                            let transactions_default_asset_id = account.default_asset_id.clone();
                            let current_type = account.account_type;
                            view! {
                                <h1>{account.account_name.clone()}</h1>
                                <p class="account-type">{current_type.label()}</p>

                                <Suspense fallback=|| {
                                    view! { <p class="loading">"Loading balance…"</p> }
                                }>
                                    {move || {
                                        balance
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
                                                Ok(balance) => {
                                                    view! {
                                                        <p class="net-worth-total">
                                                            <strong>
                                                                {format!(
                                                                    "{} {}",
                                                                    balance.amount,
                                                                    balance.currency_code,
                                                                )}
                                                            </strong>
                                                        </p>
                                                    }
                                                        .into_any()
                                                }
                                            })
                                    }}
                                </Suspense>

                                <ActionForm action=edit>
                                    <input type="hidden" name="id" value=account.id.clone() />
                                    <input
                                        type="hidden"
                                        name="default_asset_id"
                                        value=account.default_asset_id.clone()
                                    />
                                    <TextField
                                        label="Name"
                                        name="account_name"
                                        value=account.account_name.clone()
                                    />
                                    <SelectField label="Type" name="account_type">
                                        {AccountType::ALL
                                            .iter()
                                            .map(|account_type| {
                                                let account_type = *account_type;
                                                view! {
                                                    <option
                                                        value=account_type.as_db_str()
                                                        selected=account_type == current_type
                                                    >
                                                        {account_type.label()}
                                                    </option>
                                                }
                                            })
                                            .collect_view()}
                                    </SelectField>
                                    <FormError message=edit_error />
                                    <Button pending=edit.pending()>"Save"</Button>
                                </ActionForm>

                                <DeleteAccountForm
                                    account_id=account_id
                                    action=delete
                                    error=delete_error
                                />

                                <TransactionsList
                                    account_id=transactions_account_id
                                    default_asset_id=transactions_default_asset_id
                                    actions=ledger_actions
                                />
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
                        class="btn"
                        on:click=move |_| confirming.set(true)
                    >
                        "Delete account"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <input type="hidden" name="id" value=move || account_id.get() />
                <FormError message=error />
                <Button pending=action.pending()>"Confirm delete"</Button>
                <button type="button" class="btn" on:click=move |_| confirming.set(false)>
                    "Cancel"
                </button>
            </ActionForm>
        </Show>
    }
}

/// The transaction ledger for one account: the shared [`LedgerTable`] fixed to
/// this account (no Account column), plus the "Add transaction" and "Add
/// transfer" forms.
#[component]
fn TransactionsList(
    account_id: String,
    default_asset_id: String,
    actions: LedgerActions,
) -> impl IntoView {
    let account_context = AccountContext {
        id: account_id.clone(),
        default_asset_id,
    };
    let filter_account_id = account_id;
    let filter = LedgerFilter {
        account_id: Signal::derive(move || filter_account_id.clone()),
        from: Signal::derive(String::new),
        to: Signal::derive(String::new),
    };

    view! {
        <section class="transactions">
            <h2>"Transactions"</h2>
            <LedgerTable filter=filter show_account=false actions=actions />
            <div class="ledger-actions">
                <AddTransactionForm
                    fixed_account=account_context.clone()
                    action=actions.create_transaction
                />
                <AddTransferForm
                    default_source=account_context
                    action=actions.create_transfer
                />
            </div>
        </section>
    }
}
