use leptos::prelude::*;

/// A labelled text input for use inside a form.
///
/// `name` is both the input's `name` (so `ActionForm` maps it to a server
/// function argument) and its `id` (so the `<label>` points at it). One field
/// per `name` per form.
#[component]
pub fn TextField(
    /// Visible label text.
    label: &'static str,
    /// The `name` attribute; must match the server function argument.
    name: &'static str,
    /// The `type` attribute. Defaults to `"text"`.
    #[prop(default = "text")]
    input_type: &'static str,
    /// The `autocomplete` attribute, when the browser should be hinted.
    #[prop(optional)]
    autocomplete: Option<&'static str>,
    /// Whether the field is required. Defaults to true.
    #[prop(default = true)]
    required: bool,
) -> impl IntoView {
    view! {
        <div class="field">
            <label for=name>{label}</label>
            <input
                id=name
                name=name
                type=input_type
                autocomplete=autocomplete
                required=required
            />
        </div>
    }
}
