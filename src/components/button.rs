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
    /// Visual variant: `"primary"` (default, ink-filled), `"secondary"`,
    /// `"outline"`, `"ghost"`, or `"danger"`.
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
    match variant {
        "secondary" => class.push_str(" btn--secondary"),
        "outline" => class.push_str(" btn--outline"),
        "ghost" => class.push_str(" btn--ghost"),
        "danger" => class.push_str(" btn--danger"),
        _ => {}
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
