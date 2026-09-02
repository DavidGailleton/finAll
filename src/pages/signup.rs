use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::auth::api::Signup;
use crate::components::{AuthCard, Button, FormError, TextField};
use crate::pages::guard::GuestOnly;
use crate::pages::server_error_message;

/// `/signup`
#[component]
pub fn SignupPage() -> impl IntoView {
    view! {
        <GuestOnly>
            <SignupForm />
        </GuestOnly>
    }
}

#[component]
fn SignupForm() -> impl IntoView {
    let action = ServerAction::<Signup>::new();
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
        <AuthCard title="Create your account">
            <ActionForm action=action>
                <TextField label="Email" name="email" input_type="email" autocomplete="email" />
                <TextField
                    label="Display name (optional)"
                    name="display_name"
                    autocomplete="name"
                    required=false
                />
                <TextField
                    label="Password"
                    name="password"
                    input_type="password"
                    autocomplete="new-password"
                />
                <FormError message=error />
                <Button pending=action.pending()>"Sign up"</Button>
            </ActionForm>
            <p class="auth-alt">"Already have an account? " <A href="/login">"Sign in"</A></p>
        </AuthCard>
    }
}
