use leptos::prelude::*;

use crate::components::Icon;

/// Light / dark / system colour-theme control.
///
/// All behaviour lives in the inline `THEME_SCRIPT` (see [`crate::app`]): the
/// buttons call the global `__setTheme`, which writes `localStorage` and the
/// `data-theme` attribute and updates `aria-pressed`. No hydration needed, so it
/// works before (and without) the WASM bundle.
#[component]
pub fn ThemeToggle() -> impl IntoView {
    view! {
        <div class="theme-toggle" role="group" aria-label="Colour theme" data-theme-toggle>
            <button
                type="button"
                data-theme-value="light"
                aria-pressed="false"
                aria-label="Light theme"
                onclick="__setTheme('light')"
            >
                <Icon name="sun" size="sm" />
            </button>
            <button
                type="button"
                data-theme-value="dark"
                aria-pressed="false"
                aria-label="Dark theme"
                onclick="__setTheme('dark')"
            >
                <Icon name="moon" size="sm" />
            </button>
            <button
                type="button"
                data-theme-value="system"
                aria-pressed="false"
                aria-label="Match system theme"
                onclick="__setTheme('system')"
            >
                <Icon name="monitor" size="sm" />
            </button>
        </div>
    }
}
