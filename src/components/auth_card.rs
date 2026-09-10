use leptos::prelude::*;

/// Centered card layout for the auth pages: the wordmark, a heading, then its
/// content. Sits in a full-height centering shell.
#[component]
pub fn AuthCard(
    /// Heading text.
    title: &'static str,
    /// Card content (the form and any links).
    children: Children,
) -> impl IntoView {
    view! {
        <main class="auth-shell">
            <div class="auth-card">
                <p class="auth-card__brand">"fin" <b>"All"</b></p>
                <h1>{title}</h1>
                {children()}
            </div>
        </main>
    }
}
