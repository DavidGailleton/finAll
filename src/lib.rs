pub mod accounts;
pub mod app;
pub mod assets;
pub mod auth;
pub mod balances;
pub mod categories;
pub mod components;
pub mod merchants;
pub mod pages;
pub mod transactions;

#[cfg(feature = "ssr")]
pub mod server;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
