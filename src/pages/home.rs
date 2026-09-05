use leptos::prelude::*;

use crate::components::Layout;
use crate::pages::guard::RequireAuth;

/// `/`
#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <RequireAuth>
            <Layout>
                <h1>"Home"</h1>
            </Layout>
        </RequireAuth>
    }
}
