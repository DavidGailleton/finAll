pub mod accounts;
pub mod app;
pub mod assets;
pub mod auth;
pub mod balances;
pub mod categories;
pub mod components;
pub mod income_expense;
pub mod merchants;
pub mod net_worth;
pub mod pages;
pub mod transactions;
pub mod transfers;

#[cfg(feature = "ssr")]
pub mod server;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
