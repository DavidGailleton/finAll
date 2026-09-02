use leptos::prelude::*;

/// Centered card layout for the auth pages: a heading plus its content.
#[component]
pub fn AuthCard(
    /// Heading text.
    title: &'static str,
    /// Card content (the form and any links).
    children: Children,
) -> impl IntoView {
    view! {
        <div class="auth-card">
            <h1>{title}</h1>
            {children()}
        </div>
    }
}
