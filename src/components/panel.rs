use leptos::prelude::*;

/// A bento-grid panel: the box every screen is built from.
///
/// `span` is the number of 12-grid columns the panel occupies on a wide
/// viewport (it always drops to full width on narrow ones). `primary` marks the
/// one panel that carries the screen's headline figure — it gets the brass
/// left edge and a soft shadow.
#[component]
pub fn Panel(
    /// Optional panel heading (rendered as an `<h2>`).
    #[prop(optional, into)]
    title: Option<String>,
    /// Columns spanned on a wide viewport (1–12).
    #[prop(default = 12)]
    span: u8,
    /// Mark this as the screen's primary panel.
    #[prop(optional)]
    primary: bool,
    /// Muted note shown beside the heading.
    #[prop(optional, into)]
    note: Option<String>,
    children: Children,
) -> impl IntoView {
    let class = if primary {
        "panel panel--primary"
    } else {
        "panel"
    };

    view! {
        <section class=class style=format!("--span:{span}")>
            {title
                .map(|title| {
                    view! {
                        <div class="panel__head">
                            <h2 class="panel__title">{title}</h2>
                            {note.map(|note| view! { <span class="panel__note">{note}</span> })}
                        </div>
                    }
                })}
            {children()}
        </section>
    }
}
