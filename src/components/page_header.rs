use leptos::prelude::*;

/// The `<h1>` row above a page's bento grid: title, optional description, and an
/// optional trailing action slot (a button or link).
#[component]
pub fn PageHeader(
    #[prop(into)] title: String,
    #[prop(optional, into)] description: Option<String>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="page-header">
            <h1>{title}</h1>
            {children.map(|children| view! { <div class="page-header__action">{children()}</div> })}
            {description
                .map(|description| view! { <p class="page-header__desc">{description}</p> })}
        </div>
    }
}
