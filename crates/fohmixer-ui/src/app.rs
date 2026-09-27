//! The root component: the version label and the connection status line.
//! The hub connection and the mixer surfaces arrive in S3/S4.

use leptos::prelude::*;

/// The status line until the hub connection exists (S3).
pub const STATUS_TEXT: &str = "fohmixer — čaká na pripojenie k Abletonu";

/// The version label the header shows: `v` + the workspace version, the same
/// string the hub reports at `/api/version` (the E2E version test compares
/// them).
pub fn version_text() -> String {
    fohmixer_proto::version_label()
}

/// The app: `main[data-testid=app]` with a header holding the version label.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <main class="app" data-testid="app">
            <header class="app-header">
                <span class="app-title">"fohmixer"</span>
                <span class="app-version" data-testid="version">{version_text()}</span>
            </header>
            <p class="app-status" data-testid="status">{STATUS_TEXT}</p>
        </main>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_is_the_shared_version_label() {
        assert_eq!(version_text(), fohmixer_proto::version_label());
        assert_eq!(version_text(), format!("v{}", fohmixer_proto::VERSION));
    }
}
