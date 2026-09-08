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
    /// The `<option>` elements.
    children: Children,
) -> impl IntoView {
    view! {
        <div class="field">
            <label for=name>{label}</label>
            <select id=name name=name>
                {children()}
            </select>
        </div>
    }
}
