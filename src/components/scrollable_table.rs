use leptos::prelude::*;

/// A data table that scrolls horizontally inside its own focusable, labelled
/// region (so keyboard users can reach the overflow, per WCAG SC 1.4.10 / 2.1.1)
/// and carries a screen-reader `<caption>`.
///
/// The caller supplies `<thead>` and `<tbody>` as children.
#[component]
pub fn ScrollableTable(
    /// Describes the table's contents; used for both the visually-hidden
    /// `<caption>` and the scroll region's label.
    #[prop(into)]
    caption: String,
    children: Children,
) -> impl IntoView {
    let label = caption.clone();
    view! {
        <div class="table-scroll" role="region" aria-label=label tabindex="0">
            <table class="data-table">
                <caption class="sr-only">{caption}</caption>
                {children()}
            </table>
        </div>
    }
}
