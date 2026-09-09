//! `/accounts` (list) and `/accounts/:id` (detail, with edit and delete).
//!
//! Both pages follow the `Resource` + `<Suspense>` pattern from
//! [`crate::pages::guard`], and render explicit loading, empty, and error
//! states.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::accounts::api::{
    get_account, list_accounts, CreateAccount, DeleteAccount, UpdateAccount,
};
use crate::accounts::types::AccountType;
use crate::assets::currency::api::list_currencies;
use crate::categories::api::list_categories;
use crate::categories::types::CategoryDto;
use crate::components::{Button, FormError, Layout, SelectField, TextField};
use crate::merchants::api::list_merchants;
use crate::merchants::types::MerchantDto;
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;
use crate::transactions::api::{
    list_transactions, CreateTransaction, DeleteTransaction, UpdateTransaction,
};
use crate::transactions::types::TransactionDto;

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

#[component]
fn TransactionsList(account_id: String, default_asset_id: String) -> impl IntoView {
    let create = ServerAction::<CreateTransaction>::new();
    let edit = ServerAction::<UpdateTransaction>::new();
    let delete = ServerAction::<DeleteTransaction>::new();

    // First page: SSR-rendered, refetched after any successful create / edit /
    // delete by depending on the actions' versions.
    let first_account_id = account_id.clone();
    let first_page = Resource::new(
        move || {
            (
                first_account_id.clone(),
                create.version().get(),
                edit.version().get(),
                delete.version().get(),
            )
        },
        |(account_id, ..)| async move { list_transactions(Some(account_id), None, None, None).await },
    );

    // Later pages, fetched on demand by "Load more" and appended client-side.
    let extra = RwSignal::new(Vec::<TransactionDto>::new());
    let next_cursor = RwSignal::new(None::<String>);
    let more_cursor = RwSignal::new(None::<String>);

    let more_account_id = account_id.clone();
    let more_page = Resource::new(
        move || (more_account_id.clone(), more_cursor.get()),
        |(account_id, cursor)| async move {
            match cursor {
                Some(cursor) => list_transactions(Some(account_id), None, None, Some(cursor))
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );

    // The first page (re)loaded: drop any appended pages, reset paging.
    Effect::new(move |_| {
        if let Some(Ok(page)) = first_page.get() {
            extra.set(Vec::new());
            more_cursor.set(None);
            next_cursor.set(page.next_cursor);
        }
    });

    // A "Load more" fetch returned: append its rows, advance the cursor.
    Effect::new(move |_| {
        if let Some(Ok(Some(page))) = more_page.get() {
            let mut rows = page.transactions;
            extra.update(|existing| existing.append(&mut rows));
            next_cursor.set(page.next_cursor);
        }
    });

    view! {
        <section class="transactions">
            <h2>"Transactions"</h2>
            <Suspense fallback=|| {
                view! { <p class="loading">"Loading transactions…"</p> }
            }>
                {move || {
                    first_page
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
                            Ok(page) if page.transactions.is_empty() => {
                                view! {
                                    <p class="empty-state">
                                        "No transactions on this account yet."
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(page) => {
                                view! {
                                    <div class="table-scroll">
                                        <table class="transactions-table">
                                            <thead>
                                                <tr>
                                                    <th scope="col">"Date"</th>
                                                    <th scope="col">"Merchant"</th>
                                                    <th scope="col">"Category"</th>
                                                    <th scope="col">"Amount"</th>
                                                    <th scope="col">"Actions"</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {page
                                                    .transactions
                                                    .into_iter()
                                                    .map(|transaction| {
                                                        view! {
                                                            <TransactionRow
                                                                transaction=transaction
                                                                edit=edit
                                                                delete=delete
                                                            />
                                                        }
                                                    })
                                                    .collect_view()}
                                                {move || {
                                                    extra
                                                        .get()
                                                        .into_iter()
                                                        .map(|transaction| {
                                                            view! {
                                                                <TransactionRow
                                                                    transaction=transaction
                                                                    edit=edit
                                                                    delete=delete
                                                                />
                                                            }
                                                        })
                                                        .collect_view()
                                                }}
                                            </tbody>
                                        </table>
                                    </div>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
            {move || {
                next_cursor
                    .get()
                    .map(|cursor| {
                        view! {
                            <button
                                type="button"
                                class="btn"
                                on:click=move |_| more_cursor.set(Some(cursor.clone()))
                            >
                                "Load more"
                            </button>
                        }
                    })
            }}
            <AddTransactionForm
                account_id=account_id
                default_asset_id=default_asset_id
                action=create
            />
        </section>
    }
}

#[component]
fn TransactionRow(
    transaction: TransactionDto,
    edit: ServerAction<UpdateTransaction>,
    delete: ServerAction<DeleteTransaction>,
) -> impl IntoView {
    let editing = RwSignal::new(false);

    // Close the edit form once its save succeeds.
    Effect::new(move |_| {
        if matches!(edit.value().get(), Some(Ok(_))) {
            editing.set(false);
        }
    });

    let booking_date = transaction.booking_date.clone();
    let merchant = transaction
        .merchant_name
        .clone()
        .unwrap_or_else(|| "—".to_owned());
    let category = transaction
        .category_name
        .clone()
        .unwrap_or_else(|| "—".to_owned());
    let amount_display = format!("{} {}", transaction.amount, transaction.asset_code);
    let transaction_id = transaction.id.clone();

    view! {
        <tr>
            <td>{booking_date}</td>
            <td>{merchant}</td>
            <td>{category}</td>
            <td class="transaction-amount">{amount_display}</td>
            <td class="transaction-actions">
                <button
                    type="button"
                    class="btn"
                    on:click=move |_| editing.update(|open| *open = !*open)
                >
                    {move || if editing.get() { "Cancel" } else { "Edit" }}
                </button>
                <DeleteTransactionForm transaction_id=transaction_id action=delete />
                <Show when=move || editing.get() fallback=|| ()>
                    <EditTransactionForm transaction=transaction.clone() action=edit />
                </Show>
            </td>
        </tr>
    }
}

#[component]
fn EditTransactionForm(
    transaction: TransactionDto,
    action: ServerAction<UpdateTransaction>,
) -> impl IntoView {
    // Only mounted while a row is being edited, so fetch the pick lists on mount.
    let form_data = Resource::new(
        || (),
        |_| async move {
            let currencies = list_currencies().await?;
            let categories = list_categories().await?;
            let merchants = list_merchants().await?;
            Ok::<_, ServerFnError>((currencies, categories, merchants))
        },
    );

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <Suspense fallback=|| {
            view! { <p class="loading">"Loading…"</p> }
        }>
            {move || {
                let transaction = transaction.clone();
                form_data
                    .get()
                    .map(move |result| match result {
                        Err(err) => {
                            view! {
                                <p class="form-error" role="alert">
                                    {server_error_message(&err)}
                                </p>
                            }
                                .into_any()
                        }
                        Ok((currency_list, category_list, merchant_list)) => {
                            let selected = transaction.asset_id.clone();
                            let value_date = transaction.value_date.clone().unwrap_or_default();
                            view! {
                                <ActionForm action=action>
                                    <input type="hidden" name="id" value=transaction.id.clone() />
                                    <TextField
                                        label="Booking date"
                                        name="booking_date"
                                        input_type="date"
                                        value=transaction.booking_date.clone()
                                    />
                                    <TextField
                                        label="Value date"
                                        name="value_date"
                                        input_type="date"
                                        required=false
                                        value=value_date
                                    />
                                    <TextField
                                        label="Amount"
                                        name="amount"
                                        value=transaction.amount.clone()
                                    />
                                    <SelectField label="Currency" name="asset_id">
                                        {currency_list
                                            .into_iter()
                                            .map(move |currency| {
                                                let is_selected = currency.id == selected;
                                                view! {
                                                    <option value=currency.id selected=is_selected>
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
                                    <CategoryMerchantFields
                                        categories=category_list
                                        merchants=merchant_list
                                        category=transaction
                                            .category_id
                                            .clone()
                                            .unwrap_or_default()
                                        merchant=transaction
                                            .merchant_id
                                            .clone()
                                            .unwrap_or_default()
                                    />
                                    <FormError message=error />
                                    <Button pending=action.pending()>"Save"</Button>
                                </ActionForm>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}

/// The optional Category and Merchant `<select>`s shared by the add and edit
/// transaction forms. Picking a merchant that has a default category fills the
/// category field in (still overridable); nothing is derived server-side.
#[component]
fn CategoryMerchantFields(
    categories: Vec<CategoryDto>,
    merchants: Vec<MerchantDto>,
    /// Category id to preselect, `""` for none.
    category: String,
    /// Merchant id to preselect, `""` for none.
    merchant: String,
) -> impl IntoView {
    let category_id = RwSignal::new(category);
    let merchant_id = RwSignal::new(merchant);

    let merchant_defaults = merchants.clone();
    let on_merchant_change = move |ev| {
        let picked = event_target_value(&ev);
        if let Some(default) = merchant_defaults
            .iter()
            .find(|merchant| merchant.id == picked)
            .and_then(|merchant| merchant.default_category_id.clone())
        {
            category_id.set(default);
        }
        merchant_id.set(picked);
    };

    view! {
        <div class="field">
            <label for="merchant_id">"Merchant"</label>
            <select
                id="merchant_id"
                name="merchant_id"
                prop:value=move || merchant_id.get()
                on:change=on_merchant_change
            >
                <option value="">"None"</option>
                {merchants
                    .into_iter()
                    .map(|merchant| {
                        view! { <option value=merchant.id>{merchant.merchant_name}</option> }
                    })
                    .collect_view()}
            </select>
        </div>
        <div class="field">
            <label for="category_id">"Category"</label>
            <select
                id="category_id"
                name="category_id"
                prop:value=move || category_id.get()
                on:change=move |ev| category_id.set(event_target_value(&ev))
            >
                <option value="">"None"</option>
                {categories
                    .into_iter()
                    .map(|category| {
                        view! {
                            <option value=category.id>
                                {format!("{} ({})", category.category_name, category.kind.label())}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
        </div>
    }
}

#[component]
fn DeleteTransactionForm(
    transaction_id: String,
    action: ServerAction<DeleteTransaction>,
) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let transaction_id = RwSignal::new(transaction_id);

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <Show
            when=move || confirming.get()
            fallback=move || {
                view! {
                    <button type="button" class="btn" on:click=move |_| confirming.set(true)>
                        "Delete"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <input type="hidden" name="id" value=move || transaction_id.get() />
                <FormError message=error />
                <Button pending=action.pending()>"Confirm delete"</Button>
                <button type="button" class="btn" on:click=move |_| confirming.set(false)>
                    "Cancel"
                </button>
            </ActionForm>
        </Show>
    }
}

#[component]
fn AddTransactionForm(
    account_id: String,
    default_asset_id: String,
    action: ServerAction<CreateTransaction>,
) -> impl IntoView {
    let adding = RwSignal::new(false);

    // Collapse the form once a transaction is recorded.
    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            adding.set(false);
        }
    });

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    // Only fetch the pick lists once the form is opened.
    let form_data = Resource::new(
        move || adding.get(),
        |adding| async move {
            if !adding {
                return Ok::<_, ServerFnError>((Vec::new(), Vec::new(), Vec::new()));
            }
            let currencies = list_currencies().await?;
            let categories = list_categories().await?;
            let merchants = list_merchants().await?;
            Ok((currencies, categories, merchants))
        },
    );

    let account_id = RwSignal::new(account_id);
    let default_asset_id = RwSignal::new(default_asset_id);

    view! {
        <div class="add-transaction">
            <Show
                when=move || adding.get()
                fallback=move || {
                    view! {
                        <button type="button" class="btn" on:click=move |_| adding.set(true)>
                            "Add transaction"
                        </button>
                    }
                }
            >
                <Suspense fallback=|| {
                    view! { <p class="loading">"Loading…"</p> }
                }>
                    {move || {
                        form_data
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
                                Ok((currency_list, category_list, merchant_list)) => {
                                    let selected = default_asset_id.get();
                                    view! {
                                        <ActionForm action=action>
                                            <input
                                                type="hidden"
                                                name="account_id"
                                                value=move || account_id.get()
                                            />
                                            <TextField
                                                label="Booking date"
                                                name="booking_date"
                                                input_type="date"
                                            />
                                            <TextField
                                                label="Value date"
                                                name="value_date"
                                                input_type="date"
                                                required=false
                                            />
                                            <TextField label="Amount" name="amount" />
                                            <SelectField label="Currency" name="asset_id">
                                                {currency_list
                                                    .into_iter()
                                                    .map(move |currency| {
                                                        let is_selected = currency.id == selected;
                                                        view! {
                                                            <option value=currency.id selected=is_selected>
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
                                            <CategoryMerchantFields
                                                categories=category_list
                                                merchants=merchant_list
                                                category=String::new()
                                                merchant=String::new()
                                            />
                                            <FormError message=error />
                                            <Button pending=action.pending()>"Add transaction"</Button>
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
