//! `/transactions` — every account's transactions, filterable by account and
//! booking-date range, paginated with "Load more".
//!
//! Add, edit, delete, and transfer linking all happen here through the shared
//! [`crate::pages::ledger`] table and forms, the same ones `/accounts/:id` uses.

use leptos::prelude::*;
use leptos_meta::Title;

use crate::accounts::api::list_accounts;
use crate::components::{Layout, PageHeader, Panel};
use crate::pages::guard::RequireAuth;
use crate::pages::ledger::{AddTransactionForm, LedgerActions, LedgerFilter, LedgerTable};

/// `/transactions`
#[component]
pub fn TransactionsPage() -> impl IntoView {
    view! {
        <Title text="Transactions · finAll" />
        <RequireAuth>
            <Layout>
                <PageHeader
                    title="Transactions"
                    description="Every movement across all accounts. Filter by account or date, then load more as you go."
                />
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

    let actions = LedgerActions::new();
    let filter = LedgerFilter {
        account_id: account_id.into(),
        from: from.into(),
        to: to.into(),
    };

    view! {
        <div class="bento">
            <Panel title="Record" span=12>
                <div class="ledger-actions">
                    <AddTransactionForm action=actions.create_transaction />
                </div>
            </Panel>

            <Panel title="Ledger" span=12>
                <div class="filters">
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
                                                    on:change=move |ev| {
                                                        account_id.set(event_target_value(&ev))
                                                    }
                                                >
                                                    <option value="">"All accounts"</option>
                                                    {list
                                                        .into_iter()
                                                        .map(|account| {
                                                            view! {
                                                                <option value=account
                                                                    .id>{account.account_name}</option>
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

                <LedgerTable filter=filter show_account=true actions=actions />
            </Panel>
        </div>
    }
}
