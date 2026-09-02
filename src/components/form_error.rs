use leptos::prelude::*;

/// A form-level error message. Renders nothing when `message` is `None`.
#[component]
pub fn FormError(
    /// The message to show, or `None` to hide.
    #[prop(into)]
    message: Signal<Option<String>>,
) -> impl IntoView {
    view! {
        {move || {
            message
                .get()
                .map(|text| view! { <p class="form-error" role="alert">{text}</p> })
        }}
    }
}
