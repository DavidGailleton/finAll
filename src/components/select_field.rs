use leptos::prelude::*;

/// A labelled `<select>` for use inside a form.
///
/// `name` is both the control's `name` (so `ActionForm` maps it to a server
/// function argument) and its `id` (so the `<label>` points at it). The caller
/// supplies the `<option>` elements as children and marks the selected one.
#[component]
pub fn SelectField(
    /// Visible label text.
    label: &'static str,
    /// The `name` attribute; must match the server function argument.
    name: &'static str,
    /// Whether a value must be chosen. Defaults to true; pair it with a
    /// disabled, selected placeholder `<option value="">` to force a choice.
    #[prop(default = true)]
    required: bool,
    /// The `<option>` elements.
    children: Children,
) -> impl IntoView {
    view! {
        <div class="field">
            <label for=name>{label}</label>
            <select id=name name=name required=required>
                {children()}
            </select>
        </div>
    }
}
