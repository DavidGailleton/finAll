//! `/accounts` (list) and `/accounts/:id` (detail, with edit and delete).
//!
//! Both pages follow the `Resource` + `<Suspense>` pattern from
//! [`crate::pages::guard`], and render explicit loading, empty, and error
//! states.

use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::accounts::api::{get_account, list_accounts, DeleteAccount, UpdateAccount};
use crate::accounts::types::AccountType;
use crate::components::{Button, FormError, Layout, SelectField, TextField};
use crate::pages::guard::RequireAuth;
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
