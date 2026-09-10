use leptos::prelude::*;

/// A card / panel — Sure's `bg-container rounded-xl shadow-border-xs` surface.
///
/// `span` is the number of 12-grid columns the panel occupies on a wide
/// viewport (it always drops to full width on narrow ones). `widget` switches to
/// Sure's dashboard-widget layout: a flush heading strip over a padded body.
#[component]
pub fn Panel(
    /// Optional panel heading (rendered as an `<h2>`).
    #[prop(optional, into)]
    title: Option<String>,
    /// Columns spanned on a wide viewport (1–12).
    #[prop(default = 12)]
    span: u8,
    /// Kept for call-site compatibility; no distinct styling (Sure has none).
    #[prop(optional)]
    primary: bool,
    /// Use Sure's dashboard-widget layout (flush header + padded body).
    #[prop(optional)]
    widget: bool,
    /// Muted note shown beside the heading.
    #[prop(optional, into)]
    note: Option<String>,
    children: Children,
) -> impl IntoView {
    let mut class = String::from("panel");
    if primary {
        class.push_str(" panel--primary");
    }
    if widget {
        class.push_str(" panel--widget");
    }

    let head = title.map(|title| {
        view! {
            <div class="panel__head">
                <h2 class="panel__title">{title}</h2>
                {note.map(|note| view! { <span class="panel__note">{note}</span> })}
            </div>
        }
    });

    view! {
        <section class=class style=format!("--span:{span}")>
            {head}
            {if widget {
                view! { <div class="panel__body">{children()}</div> }.into_any()
            } else {
                children().into_any()
            }}
        </section>
    }
}
