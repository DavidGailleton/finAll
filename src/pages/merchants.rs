//! `/merchants` — a single page listing the signed-in user's merchants with an
//! inline "Add merchant" form and per-row rename / delete. Each merchant has an
//! optional default category, chosen from the user's categories.
//!
//! Follows the `Resource` + `<Suspense>` loading / empty / error pattern and the
//! inline-row editing used by `/categories`.

use leptos::prelude::*;

use crate::categories::api::list_categories;
use crate::categories::types::CategoryDto;
use crate::components::{Button, FormError, Layout, SelectField, TextField};
use crate::merchants::api::{list_merchants, CreateMerchant, DeleteMerchant, UpdateMerchant};
use crate::merchants::types::MerchantDto;
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;

/// `/merchants`
#[component]
pub fn MerchantsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <Layout>
                <MerchantsList />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn MerchantsList() -> impl IntoView {
    let create = ServerAction::<CreateMerchant>::new();
    let edit = ServerAction::<UpdateMerchant>::new();
    let delete = ServerAction::<DeleteMerchant>::new();

    // Refetch after any successful create / rename / delete.
    let merchants = Resource::new(
        move || {
            (
                create.version().get(),
                edit.version().get(),
                delete.version().get(),
            )
        },
        |_| async move { list_merchants().await },
    );

    // The user's categories, for the default-category picker and to render each
    // row's category name.
    let categories = Resource::new(|| (), |_| async move { list_categories().await });

    view! {
        <h1>"Merchants"</h1>
        <Suspense fallback=|| {
            view! { <p class="loading">"Loading merchants…"</p> }
        }>
            {move || {
                Some((merchants.get()?, categories.get()?))
                    .map(|(merchants_result, categories_result)| {
                        let categories = match categories_result {
                            Err(err) => {
                                return view! {
                                    <p class="form-error" role="alert">
                                        {server_error_message(&err)}
                                    </p>
                                }
                                    .into_any();
                            }
                            Ok(categories) => categories,
                        };
                        match merchants_result {
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
                                    <div>
                                        <p class="empty-state">
                                            "You don't have any merchants yet."
                                        </p>
                                        <AddMerchantForm
                                            categories=categories
                                            action=create
                                        />
                                    </div>
                                }
                                    .into_any()
                            }
                            Ok(list) => {
                                let rows_categories = categories.clone();
                                view! {
                                    <div>
                                        <div class="table-scroll">
                                            <table class="merchants-table">
                                                <thead>
                                                    <tr>
                                                        <th scope="col">"Name"</th>
                                                        <th scope="col">"Default category"</th>
                                                        <th scope="col">"Actions"</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {list
                                                        .into_iter()
                                                        .map(|merchant| {
                                                            view! {
                                                                <MerchantRow
                                                                    merchant=merchant
                                                                    categories=rows_categories.clone()
                                                                    edit=edit
                                                                    delete=delete
                                                                />
                                                            }
                                                        })
                                                        .collect_view()}
                                                </tbody>
                                            </table>
                                        </div>
                                        <AddMerchantForm categories=categories action=create />
                                    </div>
                                }
                                    .into_any()
                            }
                        }
                    })
            }}
        </Suspense>
    }
}

/// The display name of a merchant's default category, or `"—"` when unset or the
/// category is no longer active.
fn category_label(merchant: &MerchantDto, categories: &[CategoryDto]) -> String {
    merchant
        .default_category_id
        .as_ref()
        .and_then(|id| categories.iter().find(|category| &category.id == id))
        .map(|category| category.category_name.clone())
        .unwrap_or_else(|| "—".to_owned())
}

#[component]
fn MerchantRow(
    merchant: MerchantDto,
    categories: Vec<CategoryDto>,
    edit: ServerAction<UpdateMerchant>,
    delete: ServerAction<DeleteMerchant>,
) -> impl IntoView {
    let editing = RwSignal::new(false);

    // Close the edit form once its save succeeds.
    Effect::new(move |_| {
        if matches!(edit.value().get(), Some(Ok(_))) {
            editing.set(false);
        }
    });

    let name = merchant.merchant_name.clone();
    let default_category = category_label(&merchant, &categories);
    let merchant_id = merchant.id.clone();

    view! {
        <tr>
            <td>{name}</td>
            <td>{default_category}</td>
            <td class="merchant-actions">
                <button
                    type="button"
                    class="btn"
                    on:click=move |_| editing.update(|open| *open = !*open)
                >
                    {move || if editing.get() { "Cancel" } else { "Edit" }}
                </button>
                <DeleteMerchantForm merchant_id=merchant_id action=delete />
                <Show when=move || editing.get() fallback=|| ()>
                    <EditMerchantForm
                        merchant=merchant.clone()
                        categories=categories.clone()
                        action=edit
                    />
                </Show>
            </td>
        </tr>
    }
}

#[component]
fn EditMerchantForm(
    merchant: MerchantDto,
    categories: Vec<CategoryDto>,
    action: ServerAction<UpdateMerchant>,
) -> impl IntoView {
    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    let selected = merchant.default_category_id.clone();

    view! {
        <ActionForm action=action>
            <input type="hidden" name="id" value=merchant.id.clone() />
            <TextField label="Name" name="merchant_name" value=merchant.merchant_name.clone() />
            <SelectField
                label="Default category"
                name="default_category_id"
                required=false
            >
                <option value="" selected=selected.is_none()>
                    "None"
                </option>
                {categories
                    .into_iter()
                    .map(|category| {
                        let is_selected = selected.as_deref() == Some(category.id.as_str());
                        view! {
                            <option value=category.id selected=is_selected>
                                {category.category_name}
                            </option>
                        }
                    })
                    .collect_view()}
            </SelectField>
            <FormError message=error />
            <Button pending=action.pending()>"Save"</Button>
        </ActionForm>
    }
}

#[component]
fn DeleteMerchantForm(merchant_id: String, action: ServerAction<DeleteMerchant>) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let merchant_id = RwSignal::new(merchant_id);

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
                <input type="hidden" name="id" value=move || merchant_id.get() />
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
fn AddMerchantForm(
    categories: Vec<CategoryDto>,
    action: ServerAction<CreateMerchant>,
) -> impl IntoView {
    let adding = RwSignal::new(false);
    // Held in a `StoredValue` so the `<Show>` children closure (an `Fn`) can
    // rebuild the options on each toggle without moving the list.
    let categories = StoredValue::new(categories);

    // Collapse the form once a merchant is created.
    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            adding.set(false);
        }
    });

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <div class="add-merchant">
            <Show
                when=move || adding.get()
                fallback=move || {
                    view! {
                        <button type="button" class="btn" on:click=move |_| adding.set(true)>
                            "Add merchant"
                        </button>
                    }
                }
            >
                <ActionForm action=action>
                    <TextField label="Name" name="merchant_name" />
                    <SelectField
                        label="Default category"
                        name="default_category_id"
                        required=false
                    >
                        <option value="" selected>
                            "None"
                        </option>
                        {move || {
                            categories
                                .get_value()
                                .into_iter()
                                .map(|category| {
                                    view! {
                                        <option value=category.id>
                                            {category.category_name}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </SelectField>
                    <FormError message=error />
                    <Button pending=action.pending()>"Create merchant"</Button>
                    <button type="button" class="btn" on:click=move |_| adding.set(false)>
                        "Cancel"
                    </button>
                </ActionForm>
            </Show>
        </div>
    }
}
