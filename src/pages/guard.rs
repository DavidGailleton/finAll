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
/// redirecting an authenticated visitor away; otherwise the children render.
#[component]
pub fn RequireAuth(children: ChildrenFn) -> impl IntoView {
    let session = Resource::new(|| (), |_| async move { current_user().await });

    view! {
        <Suspense fallback=|| ()>
            {move || {
                session
                    .get()
                    .map(|result| match result {
                        Ok(Some(_)) => children(),
                        Ok(None) => view! { <Redirect path="/login" /> }.into_any(),
                        Err(err) => {
                            view! {
                                <p class="form-error" role="alert">
                                    {server_error_message(&err)}
                                </p>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
