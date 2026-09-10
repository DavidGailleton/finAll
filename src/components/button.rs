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
    /// Visual variant: `"primary"` (default, ink-filled), `"secondary"`
    /// (outline), or `"danger"` (used for the confirm step of a destructive
    /// action).
    #[prop(default = "primary")]
    variant: &'static str,
    /// Render at the smaller inline size (row actions).
    #[prop(optional)]
    small: bool,
    /// Disables the button while true.
    #[prop(optional, into)]
    pending: Signal<bool>,
) -> impl IntoView {
    let mut class = String::from("btn");
    if variant == "secondary" {
        class.push_str(" btn--secondary");
    } else if variant == "danger" {
        class.push_str(" btn--danger");
    }
    if small {
        class.push_str(" btn--small");
    }

    view! {
        <button type=kind class=class disabled=move || pending.get()>
            {children()}
        </button>
    }
}
