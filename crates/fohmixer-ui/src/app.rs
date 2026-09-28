//! The root component: the PIN login until the device holds a token, then
//! the mixer surface (S4 design note §6). A refused token brings the login
//! back once, with no reload.

use leptos::prelude::*;

use crate::auth;
use crate::pages::login::Login;
use crate::pages::surface::Surface;

/// The version label the app shows: `v` + the workspace version, the same
/// string the hub reports at `/api/version` (the E2E version test compares
/// them).
pub fn version_text() -> String {
    fohmixer_proto::version_label()
}

/// The address the page shows: the surface at `/`, the login at `/login`.
pub fn page_path(logged_in: bool) -> &'static str {
    if logged_in { "/" } else { "/login" }
}

/// Puts `path` in the address bar (no navigation, no reload).
fn show_path(path: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let current = window.location().pathname().unwrap_or_default();
    if current != path
        && let Ok(history) = window.history()
    {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path));
    }
}

/// The app.
#[component]
pub fn App() -> impl IntoView {
    let session = RwSignal::new(auth::stored_token());
    Effect::new(move |_| show_path(page_path(session.with(Option::is_some))));
    let page = move || match session.get() {
        Some(token) => view! { <Surface token=token session=session /> }.into_any(),
        None => view! { <Login session=session /> }.into_any(),
    };
    view! { <main class="app" data-testid="app">{page}</main> }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_is_the_shared_version_label() {
        assert_eq!(version_text(), fohmixer_proto::version_label());
        assert_eq!(version_text(), format!("v{}", fohmixer_proto::VERSION));
    }

    #[test]
    fn the_login_lives_at_its_own_address() {
        assert_eq!(page_path(false), "/login");
        assert_eq!(page_path(true), "/");
    }
}
