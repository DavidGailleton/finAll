use leptos::prelude::*;

/// A button for forms and actions.
///
/// While `pending` is true the button is disabled, so a submit cannot be
/// double-fired while a request is in flight.
#[component]
pub fn Button(
    /// Button content (its label).
    children: Children,
    /// The `type` attribute. Defaults to `"submit"`.
    #[prop(default = "submit")]
    kind: &'static str,
    /// Disables the button while true.
    #[prop(optional, into)]
    pending: Signal<bool>,
) -> impl IntoView {
    view! {
        <button type=kind class="btn" disabled=move || pending.get()>
            {children()}
        </button>
    }
}
