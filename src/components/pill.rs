use leptos::prelude::*;

/// A small rounded status pill (a delta / summary chip).
///
/// `tone` selects the colour: `"up"` (brand green), `"down"` (loss red), or
/// anything else for the neutral grey.
#[component]
pub fn Pill(#[prop(default = "")] tone: &'static str, children: Children) -> impl IntoView {
    let class = match tone {
        "up" => "pill pill--up",
        "down" => "pill pill--down",
        _ => "pill",
    };
    view! { <span class=class>{children()}</span> }
}
