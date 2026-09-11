//! The shared transaction ledger: the table of transactions plus the add /
//! edit / delete forms. Used by both `/accounts/:id` (one account, no Account
//! column) and `/transactions` (every account, with an Account column and
//! filters).
//!
//! A row whose transaction is one leg of a transfer is shown as a transfer —
//! labelled, naming the other account. It is still edited and deleted as an
//! ordinary transaction; the transfer is only a link, made and broken from the
//! edit form's "Paired transaction" field.

use leptos::prelude::*;

use crate::accounts::api::list_accounts;
use crate::accounts::types::AccountType;
use crate::assets::currency::api::list_currencies;
use crate::categories::api::list_categories;
use crate::categories::types::CategoryDto;
use crate::components::{Button, FormError, Icon, SelectField, TextField};
use crate::merchants::api::list_merchants;
use crate::merchants::types::MerchantDto;
use crate::pages::server_error_message;
use crate::transactions::api::{
    list_transactions, CreateTransaction, DeleteTransaction, UpdateTransaction,
};
use crate::transactions::types::TransactionDto;
use crate::transfers::api::{paired_transaction_options, LinkTransfer, UnlinkTransfer};

/// `"2026-01-05"` → `"January 5, 2026"`; the raw string back on any parse failure.
fn long_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut parts = iso.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_owned();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month), Ok(day)) => match MONTHS.get(month.wrapping_sub(1)) {
            Some(name) => format!("{name} {day}, {year}"),
            None => iso.to_owned(),
        },
        _ => iso.to_owned(),
    }
}

/// Tidy a stored `NUMERIC(38, 18)` string for display: keep at least two
/// fractional digits, drop pointless trailing zeros beyond that.
/// `"100.000000000000000000"` → `"100.00"`, `"1.2300"` → `"1.23"`.
fn trim_amount(raw: &str) -> String {
    let (int, frac) = raw.split_once('.').unwrap_or((raw, ""));
    let frac = frac.trim_end_matches('0');
    if frac.len() >= 2 {
        format!("{int}.{frac}")
    } else {
        format!("{int}.{frac:0<2}")
    }
}

/// The first alphanumeric character of a name, for the row avatar; `"?"` when
/// there is none.
fn first_initial(seed: &str) -> String {
    seed.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_owned())
}

/// Split a flat, newest-first transaction list into consecutive same-day groups.
fn group_by_date(rows: Vec<TransactionDto>) -> Vec<(String, Vec<TransactionDto>)> {
    let mut groups: Vec<(String, Vec<TransactionDto>)> = Vec::new();
    for row in rows {
        match groups.last_mut() {
            Some((date, bucket)) if *date == row.booking_date => bucket.push(row),
            _ => groups.push((row.booking_date.clone(), vec![row])),
        }
    }
    groups
}

/// A day's per-currency total of its non-transfer amounts, tidy-formatted.
fn day_subtotals(rows: &[TransactionDto]) -> Vec<(String, String)> {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;

    let mut totals: Vec<(String, BigDecimal)> = Vec::new();
    for row in rows {
        if row.transfer_id.is_some() {
            continue;
        }
        let Ok(amount) = BigDecimal::from_str(&row.amount) else {
            continue;
        };
        match totals.iter_mut().find(|(code, _)| *code == row.asset_code) {
            Some((_, total)) => *total += &amount,
            None => totals.push((row.asset_code.clone(), amount)),
        }
    }
    totals
        .into_iter()
        .map(|(code, total)| (code, trim_amount(&total.to_string())))
        .collect()
}

/// The account a transaction is being added on, when it is fixed (the
/// `/accounts/:id` page). Absent on `/transactions`, where the form shows an
/// account picker.
#[derive(Clone)]
pub struct AccountContext {
    pub id: String,
    pub default_asset_id: String,
    /// A `Cash` account shows a single "Date" field (booking = value date).
    pub account_type: AccountType,
}

/// The server actions the ledger drives. `ServerAction` is `Copy`, so this
/// bundle is passed by value and its `.version()`s key the list's refetch.
#[derive(Clone, Copy)]
pub struct LedgerActions {
    pub create_transaction: ServerAction<CreateTransaction>,
    pub update_transaction: ServerAction<UpdateTransaction>,
    pub delete_transaction: ServerAction<DeleteTransaction>,
    pub link_transfer: ServerAction<LinkTransfer>,
    pub unlink_transfer: ServerAction<UnlinkTransfer>,
}

impl LedgerActions {
    pub fn new() -> Self {
        Self {
            create_transaction: ServerAction::new(),
            update_transaction: ServerAction::new(),
            delete_transaction: ServerAction::new(),
            link_transfer: ServerAction::new(),
            unlink_transfer: ServerAction::new(),
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
                actions.link_transfer.version().get(),
                actions.unlink_transfer.version().get(),
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

    view! {
        <div class="txn-list">
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
                            Ok(page) => {
                                let mut rows = page.transactions;
                                rows.extend(extra.get());
                                if rows.is_empty() {
                                    return view! {
                                        <p class="txn-empty">
                                            <strong>"No transactions yet"</strong>
                                            "Use \u{201C}Add transaction\u{201D} above to record the first one."
                                        </p>
                                    }
                                        .into_any();
                                }
                                view! {
                                    <div class="txn-list__head" aria-hidden="true">
                                        <span>"Transaction"</span>
                                        <span class="txn-list__head--cat">"Category"</span>
                                        <span class="txn-list__head--amt">"Amount"</span>
                                    </div>
                                    {group_by_date(rows)
                                        .into_iter()
                                        .map(|(date, rows)| {
                                            let count = rows.len();
                                            let subtotals = day_subtotals(&rows);
                                            view! {
                                                <section class="txn-group">
                                                    <header class="txn-group__head">
                                                        <span>
                                                            <span class="txn-group__date">
                                                                {long_date(&date)}
                                                            </span>
                                                            " \u{00b7} "
                                                            <span>{count}</span>
                                                        </span>
                                                        <span class="txn-group__total">
                                                            {subtotals
                                                                .into_iter()
                                                                .map(|(code, total)| {
                                                                    view! {
                                                                        <span>{total} " " {code}</span>
                                                                    }
                                                                })
                                                                .collect_view()}
                                                        </span>
                                                    </header>
                                                    <div class="txn-group__rows">
                                                        {rows
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
                                                    </div>
                                                </section>
                                            }
                                        })
                                        .collect_view()}
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
                                class="btn btn--secondary txn-list__more"
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

    // Close the edit form once a save, a link, or an unlink succeeds.
    Effect::new(move |_| {
        if matches!(actions.update_transaction.value().get(), Some(Ok(_)))
            || matches!(actions.link_transfer.value().get(), Some(Ok(_)))
            || matches!(actions.unlink_transfer.value().get(), Some(Ok(_)))
        {
            editing.set(false);
        }
    });

    let amount_is_negative = transaction.amount.starts_with('-');
    let asset_code = transaction.asset_code.clone();
    let transaction_id = transaction.id.clone();
    let dto_for_edit = transaction.clone();

    // Primary line: the merchant (or "Transfer", or a dash).
    let name = if is_transfer {
        "Transfer".to_owned()
    } else {
        transaction
            .merchant_name
            .clone()
            .unwrap_or_else(|| "\u{2014}".to_owned())
    };

    // Avatar: a transfer glyph, else the initial of the merchant/category.
    let avatar = if is_transfer {
        view! { <Icon name="arrow-left-right" size="sm" /> }.into_any()
    } else {
        let seed = transaction
            .merchant_name
            .clone()
            .or_else(|| transaction.category_name.clone())
            .unwrap_or_default();
        view! { {first_initial(&seed)} }.into_any()
    };

    // Sub-meta: transfer counterparty / account name / the account-currency value
    // of a foreign transaction, joined with " · ".
    let meta = {
        let mut parts: Vec<String> = Vec::new();
        if is_transfer {
            let counterparty = transaction
                .transfer_counterparty
                .clone()
                .unwrap_or_else(|| "\u{2014}".to_owned());
            let arrow = if amount_is_negative {
                "\u{2192}"
            } else {
                "\u{2190}"
            };
            parts.push(format!("{arrow} {counterparty}"));
        } else if show_account {
            parts.push(transaction.account_name.clone());
        }
        if transaction.asset_id != transaction.account_default_asset_id {
            match transaction.account_amount.clone() {
                Some(converted) => parts.push(format!(
                    "{} {}",
                    trim_amount(&converted),
                    transaction.account_currency_code,
                )),
                None => parts.push("conversion pending".to_owned()),
            }
        }
        parts.join(" \u{00b7} ")
    };

    let category = transaction
        .category_name
        .clone()
        .unwrap_or_else(|| "Uncategorized".to_owned());
    let category_title = category.clone();

    // Amount, Sure palette: money in is green, money out is neutral, a transfer
    // shows "± " on the absolute value.
    let magnitude = trim_amount(
        transaction
            .amount
            .strip_prefix('-')
            .unwrap_or(&transaction.amount),
    );
    let (sign_sr, sign_glyph) = if is_transfer {
        (String::new(), "\u{00b1}\u{00a0}".to_owned())
    } else if amount_is_negative {
        ("negative ".to_owned(), "\u{2212}".to_owned())
    } else {
        ("positive ".to_owned(), "+".to_owned())
    };
    let amount_in = !is_transfer && !amount_is_negative;

    let aria_label = format!("Edit transaction: {name}");

    view! {
        <div class="txn-row" class:txn-row--editing=move || editing.get()>
            <button
                type="button"
                class="txn-row__open"
                aria-expanded=move || if editing.get() { "true" } else { "false" }
                aria-label=aria_label
                on:click=move |_| editing.update(|open| *open = !*open)
            >
                <span class="txn-avatar" class:txn-avatar--transfer=is_transfer aria-hidden="true">
                    {avatar}
                </span>
                <span class="txn-row__body">
                    <span class="txn-row__title">
                        <span class="txn-row__name">{name}</span>
                        {is_transfer.then(|| view! { <span class="txn-tag">"Transfer"</span> })}
                    </span>
                    {(!meta.is_empty())
                        .then(|| view! { <span class="txn-row__meta">{meta}</span> })}
                </span>
            </button>

            <span class="txn-row__cat">
                <span class="txn-pill" title=category_title>{category}</span>
            </span>

            <span class="txn-row__amt" class:txn-amt--in=amount_in>
                {(!sign_sr.is_empty()).then(|| view! { <span class="sr-only">{sign_sr}</span> })}
                <span aria-hidden="true">{sign_glyph}</span>
                {magnitude}
                <span class="txn-amt__code">{asset_code}</span>
            </span>

            {view! {
                <Show when=move || editing.get() fallback=|| ()>
                    <div class="txn-row__edit">
                        <EditTransactionForm transaction=dto_for_edit.clone() actions=actions />
                        <DeleteTransactionForm
                            transaction_id=transaction_id.clone()
                            is_transfer=is_transfer
                            action=actions.delete_transaction
                        />
                    </div>
                </Show>
            }
                .into_any()}
        </div>
    }
}

#[component]
fn EditTransactionForm(transaction: TransactionDto, actions: LedgerActions) -> impl IntoView {
    let action = actions.update_transaction;
    let transaction_id = transaction.id.clone();

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
                            let is_cash = transaction.account_type == AccountType::Cash;
                            view! {
                                <ActionForm action=action>
                                    <input type="hidden" name="id" value=transaction.id.clone() />
                                    <TextField
                                        label=if is_cash { "Date" } else { "Booking date" }
                                        name="booking_date"
                                        input_type="date"
                                        value=transaction.booking_date.clone()
                                    />
                                    {(!is_cash)
                                        .then(|| {
                                            view! {
                                                <TextField
                                                    label="Value date"
                                                    name="value_date"
                                                    input_type="date"
                                                    required=false
                                                    value=value_date.clone()
                                                />
                                            }
                                        })}
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
        <PairedTransactionField
            transaction_id=transaction_id
            link=actions.link_transfer
            unlink=actions.unlink_transfer
        />
    }
}

/// The "Paired transaction" field of the edit form: link this transaction to a
/// matching one in another account, or clear the link. It acts on change (its
/// own server actions), separately from the transaction's own "Save".
#[component]
fn PairedTransactionField(
    transaction_id: String,
    link: ServerAction<LinkTransfer>,
    unlink: ServerAction<UnlinkTransfer>,
) -> impl IntoView {
    let id_for_options = transaction_id.clone();
    let options = Resource::new(
        move || {
            (
                id_for_options.clone(),
                link.version().get(),
                unlink.version().get(),
            )
        },
        |(id, ..)| async move { paired_transaction_options(id).await },
    );

    let error = Signal::derive(move || match (link.value().get(), unlink.value().get()) {
        (Some(Err(err)), _) | (_, Some(Err(err))) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <div class="field">
            <label for="paired_transaction">"Paired transaction"</label>
            <Suspense fallback=|| {
                view! { <select id="paired_transaction" disabled></select> }
            }>
                {move || {
                    let transaction_id = transaction_id.clone();
                    options
                        .get()
                        .map(move |result| match result {
                            Err(err) => {
                                view! { <p class="form-error">{server_error_message(&err)}</p> }
                                    .into_any()
                            }
                            Ok(options) => {
                                let selected = options
                                    .current_pair
                                    .as_ref()
                                    .map(|pair| pair.transaction_id.clone())
                                    .unwrap_or_default();
                                view! {
                                    <select
                                        id="paired_transaction"
                                        aria-describedby="paired_transaction_hint"
                                        prop:value=selected.clone()
                                        on:change=move |ev| {
                                            let picked = event_target_value(&ev);
                                            if picked.is_empty() {
                                                unlink
                                                    .dispatch(UnlinkTransfer {
                                                        transaction_id: transaction_id.clone(),
                                                    });
                                            } else {
                                                link.dispatch(LinkTransfer {
                                                    transaction_id: transaction_id.clone(),
                                                    paired_transaction_id: picked,
                                                });
                                            }
                                        }
                                    >
                                        <option value="">"Not a transfer"</option>
                                        {options
                                            .current_pair
                                            .map(|pair| {
                                                view! {
                                                    <option value=pair.transaction_id selected=true>
                                                        {pair.label}
                                                    </option>
                                                }
                                            })}
                                        {options
                                            .candidates
                                            .into_iter()
                                            .map(|candidate| {
                                                view! {
                                                    <option value=candidate
                                                        .transaction_id>{candidate.label}</option>
                                                }
                                            })
                                            .collect_view()}
                                    </select>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
            <p class="field__hint" id="paired_transaction_hint">
                "Link this to the matching transaction in another account to mark them a transfer."
            </p>
            <FormError message=error />
        </div>
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
    /// When this transaction is one leg of a transfer, deleting it also breaks
    /// the link; the confirm step says so.
    is_transfer: bool,
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
                {is_transfer
                    .then(|| {
                        view! {
                            <p class="field-note">
                                "This transaction is part of a transfer. Deleting it also removes the transfer link."
                            </p>
                        }
                    })}
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
    // A cash account has a single "Date" field. Reactive: on `/transactions` it
    // follows the account picker.
    let is_cash = RwSignal::new(
        fixed_account
            .as_ref()
            .is_some_and(|account| account.account_type == AccountType::Cash),
    );

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
                                                                    is_cash
                                                                        .set(account.account_type == AccountType::Cash);
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
                                            <Show
                                                when=move || is_cash.get()
                                                fallback=|| {
                                                    view! {
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
                                                    }
                                                }
                                            >
                                                <TextField
                                                    label="Date"
                                                    name="booking_date"
                                                    input_type="date"
                                                />
                                            </Show>
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

#[cfg(test)]
mod tests {
    use super::*;

    fn txn(date: &str, amount: &str, code: &str, transfer: bool) -> TransactionDto {
        TransactionDto {
            id: String::new(),
            account_id: String::new(),
            account_name: "Acct".to_owned(),
            account_type: AccountType::Bank,
            account_default_asset_id: "eur".to_owned(),
            account_currency_code: "EUR".to_owned(),
            amount: amount.to_owned(),
            account_amount: Some(amount.to_owned()),
            fx_rate: None,
            asset_id: "eur".to_owned(),
            asset_code: code.to_owned(),
            booking_date: date.to_owned(),
            value_date: None,
            category_id: None,
            category_name: None,
            merchant_id: None,
            merchant_name: None,
            transfer_id: transfer.then(|| "t".to_owned()),
            transfer_counterparty: None,
        }
    }

    #[test]
    fn long_date_formats_or_falls_back() {
        assert_eq!(long_date("2026-01-05"), "January 5, 2026");
        assert_eq!(long_date("2026-12-31"), "December 31, 2026");
        assert_eq!(long_date("nonsense"), "nonsense");
        assert_eq!(long_date("2026-13-01"), "2026-13-01");
    }

    #[test]
    fn trim_amount_keeps_two_decimals_and_drops_the_rest() {
        assert_eq!(trim_amount("100.000000000000000000"), "100.00");
        assert_eq!(trim_amount("1.2300"), "1.23");
        assert_eq!(trim_amount("-40.00"), "-40.00");
        assert_eq!(trim_amount("5"), "5.00");
        assert_eq!(trim_amount("1.2"), "1.20");
        assert_eq!(trim_amount("0.123456"), "0.123456");
    }

    #[test]
    fn first_initial_takes_the_first_letter() {
        assert_eq!(first_initial("acme corp"), "A");
        assert_eq!(first_initial("  7-eleven"), "7");
        assert_eq!(first_initial("\u{2014}"), "?");
        assert_eq!(first_initial(""), "?");
    }

    #[test]
    fn group_by_date_keeps_consecutive_same_day_rows_together() {
        let rows = vec![
            txn("2026-01-16", "10", "EUR", false),
            txn("2026-01-16", "20", "EUR", false),
            txn("2026-01-15", "5", "EUR", false),
        ];
        let groups = group_by_date(rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "2026-01-16");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].1.len(), 1);
    }

    #[test]
    fn day_subtotals_sum_per_currency_and_skip_transfers() {
        let rows = vec![
            txn("2026-01-16", "10.00", "EUR", false),
            txn("2026-01-16", "-2.50", "EUR", false),
            txn("2026-01-16", "100.00", "USD", false),
            txn("2026-01-16", "999.00", "EUR", true), // transfer leg, ignored
        ];
        let mut totals = day_subtotals(&rows);
        totals.sort();
        assert_eq!(
            totals,
            vec![
                ("EUR".to_owned(), "7.50".to_owned()),
                ("USD".to_owned(), "100.00".to_owned()),
            ]
        );
    }
}
