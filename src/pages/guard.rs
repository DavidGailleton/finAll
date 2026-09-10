use leptos::prelude::*;
use leptos_router::components::Redirect;

use crate::auth::api::current_user;
use crate::pages::server_error_message;

/// Wraps a page that only signed-out visitors should see (login, signup).
///
/// While the session check is in flight nothing is rendered. A visitor who is
/// already signed in is redirected to `/`; otherwise the children render.
#[component]
pub fn GuestOnly(children: ChildrenFn) -> impl IntoView {
    let session = Resource::new(|| (), |_| async move { current_user().await });

    view! {
        <Suspense fallback=|| ()>
            {move || {
                session
                    .get()
                    .map(|result| match result {
                        Ok(Some(_)) => view! { <Redirect path="/" /> }.into_any(),
                        _ => children(),
                    })
            }}
        </Suspense>
    }
}

/// Wraps a page that only signed-in visitors should see (everything except
/// login and signup).
///
/// While the session check is in flight nothing is rendered. A visitor who is
/// not signed in is redirected to `/login`; if the check itself fails (a
/// transport or backend error) an error message is shown in place, rather than
/// redirecting an authenticated visitor away; otherwise the children render and
/// the [`SessionUser`](crate::auth::types::SessionUser) is provided as context
/// for the shell to read.
#[component]
pub fn RequireAuth(children: ChildrenFn) -> impl IntoView {
    let session = Resource::new(|| (), |_| async move { current_user().await });

    view! {
        <Suspense fallback=|| ()>
            {move || {
                session
                    .get()
                    .map(|result| match result {
                        Ok(Some(user)) => {
                            provide_context(user);
                            children()
                        }
                        Ok(None) => view! { <Redirect path="/login" /> }.into_any(),
                        Err(err) => {
                            view! {
                                <main class="auth-shell">
                                    <div class="auth-card">
                                        <p class="auth-card__brand">"fin" <b>"All"</b></p>
                                        <h1>"Something went wrong"</h1>
                                        <p class="form-error">{server_error_message(&err)}</p>
                                        <p class="auth-alt">
                                            "Reload the page to try again."
                                        </p>
                                    </div>
                                </main>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
