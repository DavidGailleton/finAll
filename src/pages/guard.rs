use leptos::prelude::*;
use leptos_router::components::Redirect;

use crate::auth::api::current_user;

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
