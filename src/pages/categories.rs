//! `/categories` — a single page listing the signed-in user's categories with
//! an inline "Add category" form and per-row rename / delete.
//!
//! Follows the `Resource` + `<Suspense>` loading / empty / error pattern and the
//! inline-row editing used by the transaction list on `/accounts/:id`.

use leptos::prelude::*;
use leptos_meta::Title;

use crate::categories::api::{list_categories, CreateCategory, DeleteCategory, UpdateCategory};
use crate::categories::types::{CategoryDto, CategoryKind};
use crate::components::{
    Button, FormError, Layout, PageHeader, Panel, ScrollableTable, SelectField, TextField,
};
use crate::pages::guard::RequireAuth;
use crate::pages::server_error_message;

/// `/categories`
#[component]
pub fn CategoriesPage() -> impl IntoView {
    view! {
        <Title text="Categories · finAll" />
        <RequireAuth>
            <Layout>
                <PageHeader
                    title="Categories"
                    description="Labels for income and spending. A category's kind is fixed once created."
                />
                <CategoriesList />
            </Layout>
        </RequireAuth>
    }
}

#[component]
fn CategoriesList() -> impl IntoView {
    let create = ServerAction::<CreateCategory>::new();
    let edit = ServerAction::<UpdateCategory>::new();
    let delete = ServerAction::<DeleteCategory>::new();

    // Refetch after any successful create / rename / delete.
    let categories = Resource::new(
        move || {
            (
                create.version().get(),
                edit.version().get(),
                delete.version().get(),
            )
        },
        |_| async move { list_categories().await },
    );

    view! {
        <div class="bento">
            <Panel title="Your categories" span=8>
                <div>
                    <Suspense fallback=|| {
                        view! { <p class="loading">"Loading categories…"</p> }
                    }>
                        {move || {
                            categories
                                .get()
                                .map(|result| match result {
                                    Err(err) => {
                                        view! {
                                            <p class="form-error">{server_error_message(&err)}</p>
                                        }
                                            .into_any()
                                    }
                                    Ok(list) if list.is_empty() => {
                                        view! {
                                            <p class="empty-state">
                                                <strong>"No categories yet"</strong>
                                                "Add one to start labelling transactions."
                                            </p>
                                        }
                                            .into_any()
                                    }
                                    Ok(list) => {
                                        view! {
                                            <ScrollableTable caption="Your categories">
                                                <thead>
                                                    <tr>
                                                        <th scope="col">"Name"</th>
                                                        <th scope="col">"Kind"</th>
                                                        <th scope="col">
                                                            <span class="sr-only">"Actions"</span>
                                                        </th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {list
                                                        .into_iter()
                                                        .map(|category| {
                                                            view! {
                                                                <CategoryRow
                                                                    category=category
                                                                    edit=edit
                                                                    delete=delete
                                                                />
                                                            }
                                                        })
                                                        .collect_view()}
                                                </tbody>
                                            </ScrollableTable>
                                        }
                                            .into_any()
                                    }
                                })
                        }}
                    </Suspense>
                </div>
            </Panel>
            <Panel title="Add category" span=4>
                <AddCategoryForm action=create />
            </Panel>
        </div>
    }
}

#[component]
fn CategoryRow(
    category: CategoryDto,
    edit: ServerAction<UpdateCategory>,
    delete: ServerAction<DeleteCategory>,
) -> impl IntoView {
    let editing = RwSignal::new(false);

    // Close the edit form once its save succeeds.
    Effect::new(move |_| {
        if matches!(edit.value().get(), Some(Ok(_))) {
            editing.set(false);
        }
    });

    let name = category.category_name.clone();
    let kind_label = category.kind.label();
    let category_id = category.id.clone();

    view! {
        <tr>
            <td>{name}</td>
            <td>{kind_label}</td>
            <td>
                <div class="row-actions">
                    <button
                        type="button"
                        class="btn btn--secondary btn--small"
                        aria-expanded=move || if editing.get() { "true" } else { "false" }
                        on:click=move |_| editing.update(|open| *open = !*open)
                    >
                        {move || if editing.get() { "Cancel" } else { "Edit" }}
                    </button>
                    <DeleteCategoryForm category_id=category_id action=delete />
                </div>
                <Show when=move || editing.get() fallback=|| ()>
                    <div class="row-form">
                        <EditCategoryForm category=category.clone() action=edit />
                    </div>
                </Show>
            </td>
        </tr>
    }
}

#[component]
fn EditCategoryForm(category: CategoryDto, action: ServerAction<UpdateCategory>) -> impl IntoView {
    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <ActionForm action=action>
            <input type="hidden" name="id" value=category.id.clone() />
            <TextField label="Name" name="category_name" value=category.category_name.clone() />
            <FormError message=error />
            <Button pending=action.pending()>"Save"</Button>
        </ActionForm>
    }
}

#[component]
fn DeleteCategoryForm(category_id: String, action: ServerAction<DeleteCategory>) -> impl IntoView {
    let confirming = RwSignal::new(false);
    let category_id = RwSignal::new(category_id);

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
                <input type="hidden" name="id" value=move || category_id.get() />
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
fn AddCategoryForm(action: ServerAction<CreateCategory>) -> impl IntoView {
    let adding = RwSignal::new(false);

    // Collapse the form once a category is created.
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
                        "Add category"
                    </button>
                }
            }
        >
            <ActionForm action=action>
                <TextField label="Name" name="category_name" />
                <SelectField label="Kind" name="kind">
                    <option value="" disabled selected>
                        "Select a kind"
                    </option>
                    {CategoryKind::ALL
                        .iter()
                        .map(|kind| {
                            let kind = *kind;
                            view! { <option value=kind.as_db_str()>{kind.label()}</option> }
                        })
                        .collect_view()}
                </SelectField>
                <FormError message=error />
                <div class="form-actions">
                    <Button pending=action.pending()>"Create category"</Button>
                    <button
                        type="button"
                        class="btn btn--secondary"
                        on:click=move |_| adding.set(false)
                    >
                        "Cancel"
                    </button>
                </div>
            </ActionForm>
        </Show>
    }
}
