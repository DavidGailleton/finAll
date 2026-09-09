//! `/transactions` — every account's transactions, filterable by account and
//! booking-date range, paginated with "Load more".
//!
//! View only: editing and deleting stay on `/accounts/:id`. Follows the
//! `Resource` + `<Suspense>` loading / empty / error pattern used elsewhere.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::accounts::api::list_accounts;
use crate::components::Layout;
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;
use crate::transactions::api::list_transactions;
use crate::transactions::types::TransactionDto;

/// `/transactions`
#[component]
pub fn TransactionsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <Layout>
                <TransactionsView />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn TransactionsView() -> impl IntoView {
    // Filter state. An empty string means "no filter on this axis".
    let account_id = RwSignal::new(String::new());
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());

    let accounts = Resource::new(|| (), |_| async move { list_accounts().await });

    // First page: refetched whenever a filter changes.
    let first_page = Resource::new(
        move || (account_id.get(), from.get(), to.get()),
        |(account_id, from, to)| async move {
            list_transactions(
                some_if_set(account_id),
                some_if_set(from),
                some_if_set(to),
                None,
            )
            .await
        },
    );

    // Later pages, fetched on demand by "Load more" and appended client-side.
    let extra = RwSignal::new(Vec::<TransactionDto>::new());
    let next_cursor = RwSignal::new(None::<String>);
    let more_cursor = RwSignal::new(None::<String>);

    let more_page = Resource::new(
        move || (account_id.get(), from.get(), to.get(), more_cursor.get()),
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
        <h1>"Transactions"</h1>

        <div class="transaction-filters">
            <div class="field">
                <label for="filter-account">"Account"</label>
                <Suspense fallback=|| {
                    view! { <select id="filter-account" disabled></select> }
                }>
                    {move || {
                        accounts
                            .get()
                            .map(|result| match result {
                                Err(_) => {
                                    view! { <select id="filter-account" disabled></select> }
                                        .into_any()
                                }
                                Ok(list) => {
                                    view! {
                                        <select
                                            id="filter-account"
                                            on:change=move |ev| account_id.set(event_target_value(&ev))
                                        >
                                            <option value="">"All accounts"</option>
                                            {list
                                                .into_iter()
                                                .map(|account| {
                                                    view! {
                                                        <option value=account.id>{account.account_name}</option>
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
            </div>
            <div class="field">
                <label for="filter-from">"From"</label>
                <input
                    id="filter-from"
                    type="date"
                    on:change=move |ev| from.set(event_target_value(&ev))
                />
            </div>
            <div class="field">
                <label for="filter-to">"To"</label>
                <input
                    id="filter-to"
                    type="date"
                    on:change=move |ev| to.set(event_target_value(&ev))
                />
            </div>
        </div>

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
                                <p class="empty-state">"No transactions match this filter."</p>
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
                                                <th scope="col">"Account"</th>
                                                <th scope="col">"Merchant"</th>
                                                <th scope="col">"Category"</th>
                                                <th scope="col">"Amount"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {page
                                                .transactions
                                                .into_iter()
                                                .map(transaction_row)
                                                .collect_view()}
                                            {move || {
                                                extra.get().into_iter().map(transaction_row).collect_view()
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
    }
}

/// `Some(value)` unless it is the empty "no filter" string.
fn some_if_set(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// One view-only transaction row for the global list.
fn transaction_row(transaction: TransactionDto) -> impl IntoView {
    view! {
        <tr>
            <td>{transaction.booking_date}</td>
            <td>
                <A href=format!(
                    "/accounts/{}",
                    transaction.account_id,
                )>{transaction.account_name}</A>
            </td>
            <td>{transaction.merchant_name.unwrap_or_else(|| "—".to_owned())}</td>
            <td>{transaction.category_name.unwrap_or_else(|| "—".to_owned())}</td>
            <td class="transaction-amount">
                {format!("{} {}", transaction.amount, transaction.asset_code)}
            </td>
        </tr>
    }
}
