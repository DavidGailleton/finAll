use leptos::prelude::*;

/// A form-level error message. The live region is always present so a
/// late-arriving message is announced; it renders nothing visible while
/// `message` is `None`.
#[component]
pub fn FormError(
    /// The message to show, or `None` to hide.
    #[prop(into)]
    message: Signal<Option<String>>,
) -> impl IntoView {
    view! {
        <div role="alert" aria-live="assertive">
            {move || {
                message
                    .get()
                    .map(|text| view! { <p class="form-error">{text}</p> })
            }}
        </div>
    }
}
