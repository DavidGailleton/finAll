use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::auth::api::Logout;
use crate::components::Button;

/// App shell for authenticated pages: a left-hand nav (sign-out pinned to the
/// bottom) beside the routed page content.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let action = ServerAction::<Logout>::new();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            navigate("/login", Default::default());
        }
    });

    view! {
        <div class="app-shell">
            <nav class="app-nav">
                <div class="app-nav-footer">
                    <ActionForm action=action>
                        <Button pending=action.pending()>"Log out"</Button>
                    </ActionForm>
                </div>
            </nav>
            <main class="app-main">{children()}</main>
        </div>
    }
}
