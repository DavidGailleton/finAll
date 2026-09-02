//! Routed pages. Shared between SSR and hydration.

mod guard;
mod login;
mod signup;

pub use login::LoginPage;
pub use signup::SignupPage;

use leptos::prelude::ServerFnError;

/// Turn a server-function error into a message safe to show the user.
///
/// The auth server functions already return user-facing strings in the
/// `ServerError` variant (see `crate::server::error::AuthError`); anything else
/// is a transport or framework failure and gets a generic message.
pub(crate) fn server_error_message(err: &ServerFnError) -> String {
    match err {
        ServerFnError::ServerError(message) => message.clone(),
        _ => "Something went wrong. Please try again.".to_owned(),
    }
}
