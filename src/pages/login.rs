use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::auth::api::Login;
use crate::components::{AuthCard, Button, FormError, TextField};
use crate::pages::guard::GuestOnly;
use crate::pages::server_error_message;

/// `/login`
#[component]
pub fn LoginPage() -> impl IntoView {
    view! {
        <GuestOnly>
            <LoginForm />
        </GuestOnly>
    }
}

#[component]
fn LoginForm() -> impl IntoView {
    let action = ServerAction::<Login>::new();
    let navigate = use_navigate();

    Effect::new(move |_| {
        if matches!(action.value().get(), Some(Ok(_))) {
            navigate("/", Default::default());
        }
    });

    let error = Signal::derive(move || match action.value().get() {
        Some(Err(err)) => Some(server_error_message(&err)),
        _ => None,
    });

    view! {
        <AuthCard title="Sign in">
            <ActionForm action=action>
                <TextField label="Email" name="email" input_type="email" autocomplete="email" />
                <TextField
                    label="Password"
                    name="password"
                    input_type="password"
                    autocomplete="current-password"
                />
                <FormError message=error />
                <Button pending=action.pending()>"Sign in"</Button>
            </ActionForm>
            <p class="auth-alt">"Don't have an account? " <A href="/signup">"Sign up"</A></p>
        </AuthCard>
    }
}
