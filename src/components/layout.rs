use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::auth::api::Logout;
use crate::auth::types::SessionUser;
use crate::components::Button;

/// App shell for authenticated pages: a skip link, the left-hand primary
/// navigation (sign-out and the current user pinned to the bottom), and the
/// routed page content in the single `<main>` landmark.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let action = ServerAction::<Logout>::new();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            navigate("/login", Default::default());
        }
    });

    // Provided by `RequireAuth` once the session check resolves.
    let user = use_context::<SessionUser>();
    let who = user.map(|user| user.display_name.unwrap_or(user.email));

    view! {
        <a class="skip-link" href="#main">"Skip to content"</a>
        <div class="app-shell">
            <nav class="sidebar" aria-label="Primary">
                <a href="/" class="brand">"fin" <b>"All"</b></a>
                <ul class="nav">
                    <li><A href="/" exact=true attr:class="nav-link">"Dashboard"</A></li>
                    <li><A href="/accounts" attr:class="nav-link">"Accounts"</A></li>
                    <li><A href="/transactions" attr:class="nav-link">"Transactions"</A></li>
                    <li><A href="/categories" attr:class="nav-link">"Categories"</A></li>
                    <li><A href="/merchants" attr:class="nav-link">"Merchants"</A></li>
                </ul>
                <div class="sidebar-footer">
                    {who.map(|who| view! { <span class="who">{who}</span> })}
                    <ActionForm action=action>
                        <Button variant="secondary" small=true pending=action.pending()>
                            "Log out"
                        </Button>
                    </ActionForm>
                </div>
            </nav>
            <main id="main" class="app-main">
                {children()}
            </main>
        </div>
    }
}
