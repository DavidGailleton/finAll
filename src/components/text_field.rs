use leptos::prelude::*;

/// A labelled text input for use inside a form.
///
/// `name` is both the input's `name` (so `ActionForm` maps it to a server
/// function argument) and its `id` (so the `<label>` points at it). One field
/// per `name` per form. An optional `hint` and a reactive `error` are wired to
/// the input with `aria-describedby` / `aria-invalid`.
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
    /// Initial value of the input, for edit forms.
    #[prop(optional, into)]
    value: Option<String>,
    /// Static helper text shown under the field.
    #[prop(optional)]
    hint: Option<&'static str>,
    /// Reactive validation message; when `Some`, the field is marked invalid.
    #[prop(optional, into)]
    error: Signal<Option<String>>,
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
            <input
                id=name
                name=name
                type=input_type
                autocomplete=autocomplete
                required=required
                value=value
                aria-describedby=described_by
                aria-invalid=move || error.get().map(|_| "true")
            />
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
