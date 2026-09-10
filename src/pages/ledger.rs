//! The shared transaction ledger: the table of transactions plus the add /
//! edit / delete forms for transactions **and** transfers. Used by both
//! `/accounts/:id` (one account, no Account column) and `/transactions` (every
//! account, with an Account column and filters).
//!
//! A row whose transaction is one leg of a transfer is shown as a transfer —
//! labelled, naming the other account — and its actions edit or void the whole
//! transfer rather than the single leg.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::accounts::api::list_accounts;
use crate::assets::currency::api::list_currencies;
use crate::categories::api::list_categories;
use crate::categories::types::CategoryDto;
use crate::components::{Button, FormError, Money, ScrollableTable, SelectField, TextField};
use crate::merchants::api::list_merchants;
use crate::merchants::types::MerchantDto;
use crate::pages::server_error_message;
use crate::transactions::api::{
    list_transactions, CreateTransaction, DeleteTransaction, UpdateTransaction,
};
use crate::transactions::types::TransactionDto;
use crate::transfers::api::{get_transfer, CreateTransfer, UpdateTransfer, VoidTransfer};

/// The account a transaction is being added on, when it is fixed (the
/// `/accounts/:id` page). Absent on `/transactions`, where the form shows an
/// account picker.
#[derive(Clone)]
pub struct AccountContext {
    pub id: String,
    pub default_asset_id: String,
}

/// The six server actions the ledger drives. `ServerAction` is `Copy`, so this
/// bundle is passed by value and its `.version()`s key the list's refetch.
#[derive(Clone, Copy)]
pub struct LedgerActions {
    pub create_transaction: ServerAction<CreateTransaction>,
    pub update_transaction: ServerAction<UpdateTransaction>,
    pub delete_transaction: ServerAction<DeleteTransaction>,
    pub create_transfer: ServerAction<CreateTransfer>,
    pub update_transfer: ServerAction<UpdateTransfer>,
    pub void_transfer: ServerAction<VoidTransfer>,
}

impl LedgerActions {
    pub fn new() -> Self {
        Self {
            create_transaction: ServerAction::new(),
            update_transaction: ServerAction::new(),
            delete_transaction: ServerAction::new(),
            create_transfer: ServerAction::new(),
            update_transfer: ServerAction::new(),
            void_transfer: ServerAction::new(),
        }
    }
}

impl Default for LedgerActions {
    fn default() -> Self {
        Self::new()
    }
}

/// The reactive filter feeding the list. On `/accounts/:id` the account is a
/// constant and the dates are empty; on `/transactions` all three come from the
/// filter controls. An empty string means "no filter on this axis".
#[derive(Clone, Copy)]
pub struct LedgerFilter {
    pub account_id: Signal<String>,
    pub from: Signal<String>,
    pub to: Signal<String>,
}

/// `Some(value)` unless it is the empty "no filter" string.
fn some_if_set(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// The transactions table (with "Load more"), for one filter.
#[component]
pub fn LedgerTable(
    filter: LedgerFilter,
    /// Show a leading Account column (true on `/transactions`).
    show_account: bool,
    actions: LedgerActions,
) -> impl IntoView {
    // First page: refetched on a filter change or any successful write.
    let first_page = Resource::new(
        move || {
            (
                filter.account_id.get(),
                filter.from.get(),
                filter.to.get(),
                actions.create_transaction.version().get(),
                actions.update_transaction.version().get(),
                actions.delete_transaction.version().get(),
                actions.create_transfer.version().get(),
                actions.update_transfer.version().get(),
                actions.void_transfer.version().get(),
            )
        },
        |(account_id, from, to, ..)| async move {
            list_transactions(
                some_if_set(account_id),
                some_if_set(from),
                some_if_set(to),
                None,
            )
            .await
        },
    );

    let extra = RwSignal::new(Vec::<TransactionDto>::new());
    let next_cursor = RwSignal::new(None::<String>);
    let more_cursor = RwSignal::new(None::<String>);

    let more_page = Resource::new(
        move || {
            (
                filter.account_id.get(),
                filter.from.get(),
                filter.to.get(),
                more_cursor.get(),
            )
        },
        |(account_id, from, to, cursor)| async move {
            match cursor {
                Some(cursor) => list_transactions(
                    some_if_set(account_id),
                    some_if_set(from),
                    some_if_set(to),
                    Some(cursor),
                )
                .await
                .map(Some),
                None => Ok(None),
            }
        },
    );

    // First page (re)loaded: drop appended pages, reset paging.
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

    let caption = if show_account {
        "Transactions across all accounts"
    } else {
        "Transactions for this account"
    };

    view! {
        <div>
            <Suspense fallback=|| {
                view! { <p class="loading">"Loading transactions…"</p> }
            }>
                {move || {
                    first_page
                        .get()
                        .map(|result| match result {
                            Err(err) => {
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
                                    .into_any()
                            }
                            Ok(page) if page.transactions.is_empty() => {
                                view! {
                                    <p class="empty-state">
                                        <strong>"No transactions yet"</strong>
                                        "Use \u{201C}Add transaction\u{201D} above to record the first one."
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(page) => {
                                view! {
                                    <ScrollableTable caption=caption>
                                        <thead>
                                            <tr>
                                                {show_account
                                                    .then(|| view! { <th scope="col">"Account"</th> })}
                                                <th scope="col">"Date"</th>
                                                <th scope="col">"Merchant"</th>
                                                <th scope="col">"Category"</th>
                                                <th scope="col" class="num">"Amount"</th>
                                                <th scope="col">
                                                    <span class="sr-only">"Actions"</span>
                                                </th>
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
                                                            show_account=show_account
                                                            actions=actions
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
                                                                show_account=show_account
                                                                actions=actions
                                                            />
                                                        }
                                                    })
                                                    .collect_view()
                                            }}
                                        </tbody>
                                    </ScrollableTable>
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
                                class="btn btn--secondary"
                                on:click=move |_| more_cursor.set(Some(cursor.clone()))
                            >
                                "Load more"
                            </button>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn TransactionRow(
    transaction: TransactionDto,
    show_account: bool,
    actions: LedgerActions,
) -> impl IntoView {
    let editing = RwSignal::new(false);
    let is_transfer = transaction.transfer_id.is_some();

    // Close the edit form once its save (transaction or transfer) succeeds.
    Effect::new(move |_| {
        if matches!(actions.update_transaction.value().get(), Some(Ok(_)))
            || matches!(actions.update_transfer.value().get(), Some(Ok(_)))
        {
            editing.set(false);
        }
    });

    let booking_date = transaction.booking_date.clone();
    let account_id = transaction.account_id.clone();
    let account_name = transaction.account_name.clone();
    let amount_is_negative = transaction.amount.starts_with('-');
    let amount = transaction.amount.clone();
    let asset_code = transaction.asset_code.clone();
    let transfer_id = transaction.transfer_id.clone().unwrap_or_default();
    let transaction_id = transaction.id.clone();

    let (label_cell, detail_cell) = if is_transfer {
        let counterparty = transaction
            .transfer_counterparty
            .clone()
            .unwrap_or_else(|| "—".to_owned());
        let (word, direction) = if amount_is_negative {
            ("to", "\u{2192} ")
        } else {
            ("from", "\u{2190} ")
        };
        (
            "Transfer".to_owned(),
            view! {
                <span aria-hidden="true">{direction}</span>
                <span class="sr-only">{word} " "</span>
                {counterparty}
            }
            .into_any(),
        )
    } else {
        (
            transaction
                .merchant_name
                .clone()
                .unwrap_or_else(|| "—".to_owned()),
            transaction
                .category_name
                .clone()
                .unwrap_or_else(|| "—".to_owned())
                .into_any(),
        )
    };

    let dto_for_edit = transaction.clone();

    view! {
        <tr>
            {show_account
                .then(|| {
                    view! {
                        <td>
                            <A href=format!("/accounts/{account_id}")>{account_name}</A>
                        </td>
                    }
                })}
            <td class="date">{booking_date}</td>
            <td>{label_cell}</td>
            <td>{detail_cell}</td>
            <td class="num">
                <Money amount=amount code=asset_code />
            </td>
            <td>
                <div class="row-actions">
                    <button
                        type="button"
                        class="btn btn--secondary btn--small"
                        aria-expanded=move || if editing.get() { "true" } else { "false" }
                        on:click=move |_| editing.update(|open| *open = !*open)
                    >
                        {move || {
                            if editing.get() {
                                "Cancel"
                            } else if is_transfer {
                                "Edit transfer"
                            } else {
                                "Edit"
                            }
                        }}
                    </button>
                    {if is_transfer {
                        view! {
                            <VoidTransferForm
                                transfer_id=transfer_id.clone()
                                action=actions.void_transfer
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <DeleteTransactionForm
                                transaction_id=transaction_id.clone()
                                action=actions.delete_transaction
                            />
                        }
                            .into_any()
                    }}
                </div>
                {if is_transfer {
                    view! {
                        <Show when=move || editing.get() fallback=|| ()>
                            <div class="row-form">
                                <EditTransferForm
                                    transfer_id=transfer_id.clone()
                                    action=actions.update_transfer
                                />
                            </div>
                        </Show>
                    }
                        .into_any()
                } else {
                    view! {
                        <Show when=move || editing.get() fallback=|| ()>
                            <div class="row-form">
                                <EditTransactionForm
                                    transaction=dto_for_edit.clone()
                                    action=actions.update_transaction
                                />
                            </div>
                        </Show>
                    }
                        .into_any()
                }}
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
                            view! { <p class="form-error">{server_error_message(&err)}</p> }
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
                                        hint="Positive is money in, negative is money out."
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
                                        category=transaction.category_id.clone().unwrap_or_default()
                                        merchant=transaction.merchant_id.clone().unwrap_or_default()
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
                    <button
                        type="button"
                        class="btn btn--danger btn--small"
                        aria-expanded="false"
                        on:click=move |_| confirming.set(true)
                    >
                        "Delete"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <input type="hidden" name="id" value=move || transaction_id.get() />
                <FormError message=error />
                <div class="form-actions">
                    <Button variant="danger" small=true pending=action.pending()>
                        "Confirm delete"
                    </Button>
                    <button
                        type="button"
                        class="btn btn--secondary btn--small"
                        on:click=move |_| confirming.set(false)
                    >
                        "Cancel"
                    </button>
                </div>
            </ActionForm>
        </Show>
    }
}

#[component]
pub fn AddTransactionForm(
    /// Fixed account (the `/accounts/:id` page); `None` shows an account picker.
    #[prop(optional)]
    fixed_account: Option<AccountContext>,
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

    let has_fixed_account = fixed_account.is_some();
    let initial_account = fixed_account
        .as_ref()
        .map(|account| account.id.clone())
        .unwrap_or_default();
    let initial_currency = fixed_account
        .as_ref()
        .map(|account| account.default_asset_id.clone())
        .unwrap_or_default();

    // Only fetch the pick lists once the form is opened.
    let form_data = Resource::new(
        move || adding.get(),
        move |adding| async move {
            if !adding {
                return Ok::<_, ServerFnError>((Vec::new(), Vec::new(), Vec::new(), Vec::new()));
            }
            let currencies = list_currencies().await?;
            let categories = list_categories().await?;
            let merchants = list_merchants().await?;
            let accounts = if has_fixed_account {
                Vec::new()
            } else {
                list_accounts().await?
            };
            Ok((currencies, categories, merchants, accounts))
        },
    );

    let account_id = RwSignal::new(initial_account);
    let currency_id = RwSignal::new(initial_currency);

    view! {
        <div class="disclosure add-form">
            <Show
                when=move || adding.get()
                fallback=move || {
                    view! {
                        <button
                            type="button"
                            class="btn"
                            aria-expanded="false"
                            on:click=move |_| adding.set(true)
                        >
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
                                        <p class="form-error">{server_error_message(&err)}</p>
                                    }
                                        .into_any()
                                }
                                Ok((currency_list, category_list, merchant_list, account_list)) => {
                                    let accounts_for_change = account_list.clone();
                                    view! {
                                        <ActionForm action=action>
                                            {if has_fixed_account {
                                                view! {
                                                    <input
                                                        type="hidden"
                                                        name="account_id"
                                                        prop:value=move || account_id.get()
                                                    />
                                                }
                                                    .into_any()
                                            } else {
                                                view! {
                                                    <div class="field">
                                                        <label for="account_id">"Account"</label>
                                                        <select
                                                            id="account_id"
                                                            name="account_id"
                                                            prop:value=move || account_id.get()
                                                            on:change=move |ev| {
                                                                let picked = event_target_value(&ev);
                                                                if let Some(account) = accounts_for_change
                                                                    .iter()
                                                                    .find(|account| account.id == picked)
                                                                {
                                                                    currency_id.set(account.default_asset_id.clone());
                                                                }
                                                                account_id.set(picked);
                                                            }
                                                        >
                                                            <option value="">"Select an account"</option>
                                                            {account_list
                                                                .into_iter()
                                                                .map(|account| {
                                                                    view! {
                                                                        <option value=account
                                                                            .id>{account.account_name}</option>
                                                                    }
                                                                })
                                                                .collect_view()}
                                                        </select>
                                                    </div>
                                                }
                                                    .into_any()
                                            }}
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
                                            <TextField
                                                label="Amount"
                                                name="amount"
                                                hint="Positive is money in, negative is money out."
                                            />
                                            <div class="field">
                                                <label for="asset_id">"Currency"</label>
                                                <select
                                                    id="asset_id"
                                                    name="asset_id"
                                                    prop:value=move || currency_id.get()
                                                    on:change=move |ev| currency_id.set(event_target_value(&ev))
                                                >
                                                    <option value="">"Select a currency"</option>
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
                                                </select>
                                            </div>
                                            <CategoryMerchantFields
                                                categories=category_list
                                                merchants=merchant_list
                                                category=String::new()
                                                merchant=String::new()
                                            />
                                            <FormError message=error />
                                            <div class="form-actions">
                                                <Button pending=action
                                                    .pending()>"Add transaction"</Button>
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
        </div>
    }
}

#[component]
pub fn AddTransferForm(
    /// Pre-select this account (and its currency) as the "From" side. Used on
    /// `/accounts/:id`; omitted on `/transactions`.
    #[prop(optional)]
    default_source: Option<AccountContext>,
    action: ServerAction<CreateTransfer>,
) -> impl IntoView {
    let adding = RwSignal::new(false);

    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            adding.set(false);
        }
    });

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    let form_data = Resource::new(
        move || adding.get(),
        |adding| async move {
            if !adding {
                return Ok::<_, ServerFnError>((Vec::new(), Vec::new()));
            }
            let accounts = list_accounts().await?;
            let currencies = list_currencies().await?;
            Ok((accounts, currencies))
        },
    );

    let (initial_source_account, initial_source_currency) = default_source
        .map(|account| (account.id, account.default_asset_id))
        .unwrap_or_default();
    let source_account = RwSignal::new(initial_source_account);
    let source_currency = RwSignal::new(initial_source_currency);
    let destination_account = RwSignal::new(String::new());
    let destination_currency = RwSignal::new(String::new());

    view! {
        <div class="disclosure add-form">
            <Show
                when=move || adding.get()
                fallback=move || {
                    view! {
                        <button
                            type="button"
                            class="btn"
                            aria-expanded="false"
                            on:click=move |_| adding.set(true)
                        >
                            "Add transfer"
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
                                        <p class="form-error">{server_error_message(&err)}</p>
                                    }
                                        .into_any()
                                }
                                Ok((account_list, currency_list)) => {
                                    let source_accounts = account_list.clone();
                                    let destination_accounts = account_list.clone();
                                    let source_account_options = account_list.clone();
                                    let destination_account_options = account_list.clone();
                                    let source_currency_options = currency_list.clone();
                                    view! {
                                        <ActionForm action=action>
                                            <fieldset>
                                                <legend>"From"</legend>
                                                <div class="field">
                                                    <label for="source-account">"Account"</label>
                                                    <select
                                                        id="source-account"
                                                        name="source[account_id]"
                                                        prop:value=move || source_account.get()
                                                        on:change=move |ev| {
                                                            let picked = event_target_value(&ev);
                                                            if let Some(account) = source_accounts
                                                                .iter()
                                                                .find(|account| account.id == picked)
                                                            {
                                                                source_currency.set(account.default_asset_id.clone());
                                                            }
                                                            source_account.set(picked);
                                                        }
                                                    >
                                                        <option value="">"Select an account"</option>
                                                        {source_account_options
                                                            .into_iter()
                                                            .map(|account| {
                                                                let this_id = account.id.clone();
                                                                view! {
                                                                    <option
                                                                        value=account.id
                                                                        disabled=move || destination_account.get() == this_id
                                                                    >
                                                                        {account.account_name}
                                                                    </option>
                                                                }
                                                            })
                                                            .collect_view()}
                                                    </select>
                                                </div>
                                                <div class="field">
                                                    <label for="source-currency">"Currency"</label>
                                                    <select
                                                        id="source-currency"
                                                        name="source[asset_id]"
                                                        prop:value=move || source_currency.get()
                                                        on:change=move |ev| {
                                                            source_currency.set(event_target_value(&ev))
                                                        }
                                                    >
                                                        <option value="">"Select a currency"</option>
                                                        {source_currency_options
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
                                                    </select>
                                                </div>
                                                <TextField label="Amount" name="source[amount]" />
                                            </fieldset>
                                            <fieldset>
                                                <legend>"To"</legend>
                                                <div class="field">
                                                    <label for="destination-account">"Account"</label>
                                                    <select
                                                        id="destination-account"
                                                        name="destination[account_id]"
                                                        prop:value=move || destination_account.get()
                                                        on:change=move |ev| {
                                                            let picked = event_target_value(&ev);
                                                            if let Some(account) = destination_accounts
                                                                .iter()
                                                                .find(|account| account.id == picked)
                                                            {
                                                                destination_currency
                                                                    .set(account.default_asset_id.clone());
                                                            }
                                                            destination_account.set(picked);
                                                        }
                                                    >
                                                        <option value="">"Select an account"</option>
                                                        {destination_account_options
                                                            .into_iter()
                                                            .map(|account| {
                                                                let this_id = account.id.clone();
                                                                view! {
                                                                    <option
                                                                        value=account.id
                                                                        disabled=move || source_account.get() == this_id
                                                                    >
                                                                        {account.account_name}
                                                                    </option>
                                                                }
                                                            })
                                                            .collect_view()}
                                                    </select>
                                                </div>
                                                <div class="field">
                                                    <label for="destination-currency">"Currency"</label>
                                                    <select
                                                        id="destination-currency"
                                                        name="destination[asset_id]"
                                                        prop:value=move || destination_currency.get()
                                                        on:change=move |ev| {
                                                            destination_currency.set(event_target_value(&ev))
                                                        }
                                                    >
                                                        <option value="">"Select a currency"</option>
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
                                                    </select>
                                                </div>
                                                <TextField label="Amount" name="destination[amount]" />
                                            </fieldset>
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
                                            <FormError message=error />
                                            <div class="form-actions">
                                                <Button pending=action.pending()>"Add transfer"</Button>
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
        </div>
    }
}

#[component]
fn EditTransferForm(transfer_id: String, action: ServerAction<UpdateTransfer>) -> impl IntoView {
    let id_for_resource = transfer_id.clone();
    let data = Resource::new(
        move || id_for_resource.clone(),
        |id| async move {
            let transfer = get_transfer(id).await?;
            let currencies = list_currencies().await?;
            Ok::<_, ServerFnError>((transfer, currencies))
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
                data.get()
                    .map(|result| match result {
                        Err(err) => {
                            view! { <p class="form-error">{server_error_message(&err)}</p> }
                                .into_any()
                        }
                        Ok((transfer, currency_list)) => {
                            let source_asset = transfer.source.asset_id.clone();
                            let destination_asset = transfer.destination.asset_id.clone();
                            let source_currencies = currency_list.clone();
                            view! {
                                <ActionForm action=action>
                                    <input type="hidden" name="id" value=transfer.id.clone() />
                                    <p class="field-note">
                                        "The two accounts can't be changed here — void this transfer and create a new one to move it."
                                    </p>
                                    <TextField
                                        label="Booking date"
                                        name="booking_date"
                                        input_type="date"
                                        value=transfer.booking_date.clone()
                                    />
                                    <TextField
                                        label="Value date"
                                        name="value_date"
                                        input_type="date"
                                        required=false
                                        value=transfer.value_date.clone().unwrap_or_default()
                                    />
                                    <TextField
                                        label="Amount leaving source"
                                        name="source_amount"
                                        value=transfer.source.amount.clone()
                                    />
                                    <SelectField label="Source currency" name="source_asset_id">
                                        {source_currencies
                                            .into_iter()
                                            .map(move |currency| {
                                                let is_selected = currency.id == source_asset;
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
                                    <TextField
                                        label="Amount arriving at destination"
                                        name="destination_amount"
                                        value=transfer.destination.amount.clone()
                                    />
                                    <SelectField
                                        label="Destination currency"
                                        name="destination_asset_id"
                                    >
                                        {currency_list
                                            .into_iter()
                                            .map(move |currency| {
                                                let is_selected = currency.id == destination_asset;
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
                                    <FormError message=error />
                                    <Button pending=action.pending()>"Save transfer"</Button>
                                </ActionForm>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}

#[component]
fn VoidTransferForm(transfer_id: String, action: ServerAction<VoidTransfer>) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let transfer_id = RwSignal::new(transfer_id);

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <Show
            when=move || confirming.get()
            fallback=move || {
                view! {
                    <button
                        type="button"
                        class="btn btn--danger btn--small"
                        aria-expanded="false"
                        on:click=move |_| confirming.set(true)
                    >
                        "Void transfer"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <input type="hidden" name="id" value=move || transfer_id.get() />
                <FormError message=error />
                <div class="form-actions">
                    <Button variant="danger" small=true pending=action.pending()>
                        "Confirm void"
                    </Button>
                    <button
                        type="button"
                        class="btn btn--secondary btn--small"
                        on:click=move |_| confirming.set(false)
                    >
                        "Cancel"
                    </button>
                </div>
            </ActionForm>
        </Show>
    }
}
