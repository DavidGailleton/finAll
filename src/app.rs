use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Meta, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    ParamSegment, StaticSegment,
};

use crate::pages::{
    AccountDetailPage, AccountsPage, CategoriesPage, HomePage, LoginPage, MerchantsPage,
    SignupPage, TransactionsPage,
};

/// Applies the saved colour theme before first paint (anti-FOUC) and exposes
/// `window.__setTheme(light|dark|system)` for the [`ThemeToggle`] buttons.
/// `data-theme` absent = follow the OS (`prefers-color-scheme`).
const THEME_SCRIPT: &str = r#"(function(){
  var d=document.documentElement;
  function saved(){try{return localStorage.theme||'system'}catch(e){return 'system'}}
  function apply(t){
    if(t==='dark')d.setAttribute('data-theme','dark');
    else if(t==='light')d.setAttribute('data-theme','light');
    else d.removeAttribute('data-theme');
  }
  function marks(t){
    var els=document.querySelectorAll('[data-theme-toggle] [data-theme-value]');
    for(var i=0;i<els.length;i++){
      els[i].setAttribute('aria-pressed',els[i].getAttribute('data-theme-value')===t?'true':'false');
    }
  }
  apply(saved());
  window.__setTheme=function(t){
    try{if(t==='system')localStorage.removeItem('theme');else localStorage.theme=t}catch(e){}
    apply(t);marks(t);
  };
  document.addEventListener('DOMContentLoaded',function(){marks(saved())});
})();"#;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <meta name="color-scheme" content="light dark"/>
                <script inner_html=THEME_SCRIPT />
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        // injects a stylesheet into the document <head>
        // id=leptos means cargo-leptos will hot-reload this stylesheet
        <Stylesheet id="leptos" href="/pkg/fin-all.css"/>

        // default document title; each page sets its own more specific one
        <Title text="finAll"/>
        <Meta name="color-scheme" content="light dark"/>

        <Router>
            <Routes fallback=|| view! { <NotFound/> }>
                <Route path=StaticSegment("") view=HomePage/>
                <Route path=StaticSegment("login") view=LoginPage/>
                <Route path=StaticSegment("signup") view=SignupPage/>
                <Route path=StaticSegment("accounts") view=AccountsPage/>
                <Route
                    path=(StaticSegment("accounts"), ParamSegment("id"))
                    view=AccountDetailPage
                />
                <Route path=StaticSegment("transactions") view=TransactionsPage/>
                <Route path=StaticSegment("categories") view=CategoriesPage/>
                <Route path=StaticSegment("merchants") view=MerchantsPage/>
            </Routes>
        </Router>
    }
}

/// Router fallback for an unknown path.
#[component]
fn NotFound() -> impl IntoView {
    view! {
        <Title text="Page not found · finAll"/>
        <main class="not-found">
            <h1>"Page not found"</h1>
            <p class="empty-state">"That page doesn't exist or has moved."</p>
            <a href="/" class="btn btn--secondary">"Back to the dashboard"</a>
        </main>
    }
}
