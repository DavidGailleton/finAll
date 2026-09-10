use leptos::prelude::*;

/// A Lucide icon from `/public/icons/<name>.svg`, tinted with `currentColor`
/// via a CSS mask (so `color` on the parent styles it).
///
/// Decorative by default (`aria-hidden`). Pass `label` to make it a labelled
/// `role="img"` when the icon is the only content of a control.
#[component]
pub fn Icon(
    /// Lucide slug, e.g. `"wallet"`. Resolves to `/icons/<name>.svg`.
    #[prop(into)]
    name: String,
    /// `"xs" | "sm" | "md" (default) | "lg" | "xl"`.
    #[prop(default = "md")]
    size: &'static str,
    /// Accessible name; when set the icon is exposed as `role="img"`.
    #[prop(optional)]
    label: Option<&'static str>,
    /// Extra class(es) appended after `icon icon--<size>`.
    #[prop(optional, into)]
    class: Option<String>,
) -> impl IntoView {
    let mut classes = String::from("icon");
    if size != "md" {
        classes.push_str(" icon--");
        classes.push_str(size);
    }
    if let Some(extra) = class {
        classes.push(' ');
        classes.push_str(&extra);
    }

    let url = format!("/icons/{name}.svg");
    let style = format!("-webkit-mask-image:url({url});mask-image:url({url})");

    view! {
        <span
            class=classes
            style=style
            role=label.map(|_| "img")
            aria-label=label
            aria-hidden=label.is_none().then_some("true")
        ></span>
    }
}
