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
    /// Static helper text shown under the field.
    #[prop(optional)]
    hint: Option<&'static str>,
    /// Reactive validation message; when `Some`, the control is marked invalid.
    #[prop(optional, into)]
    error: Signal<Option<String>>,
    /// The `<option>` elements.
    children: Children,
) -> impl IntoView {
    let hint_id = hint.map(|_| format!("{name}-hint"));
    let error_id = format!("{name}-error");
    let described_by = {
        let mut ids: Vec<String> = Vec::new();
        if let Some(id) = &hint_id {
            ids.push(id.clone());
        }
        ids.push(error_id.clone());
        ids.join(" ")
    };

    view! {
        <div class="field">
            <label for=name>{label}</label>
            <select
                id=name
                name=name
                required=required
                aria-describedby=described_by
                aria-invalid=move || error.get().map(|_| "true")
            >
                {children()}
            </select>
            {hint
                .map(|hint| {
                    view! {
                        <p class="field__hint" id=hint_id>
                            {hint}
                        </p>
                    }
                })}
            <p class="field__error" id=error_id aria-live="polite">
                {move || error.get()}
            </p>
        </div>
    }
}
