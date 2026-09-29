//! The panic hook: a reload overlay and a report to `/api/client-error`
//! (copied from iemmixer's `iem-ui/src/lifecycle.rs` @ 22372bc, the
//! WebSocket watchdog and the reconnect banner left for S4).
//!
//! The pure helpers (`build_overlay_html`, `format_panic_payload`,
//! `truncate_for_display`, `html_escape`) are unit-tested here. The DOM- and
//! network-touching wrappers (`render_panic_overlay`, `post_panic_report`)
//! are thin web-sys calls over them.

use wasm_bindgen::JsCast;

/// Maximum number of characters of a panic message shown in the overlay;
/// longer messages are cut with an ellipsis so the overlay stays legible.
pub const PANIC_MESSAGE_MAX_DISPLAY_CHARS: usize = 200;

/// Install a panic hook that:
/// 1. logs the panic to the browser console (`console_error_panic_hook`);
/// 2. renders a red reload overlay into `document.body`;
/// 3. POSTs a report to `/api/client-error` (fire-and-forget).
///
/// Must be called BEFORE `leptos::mount::mount_to_body`, so a panic during
/// mount is captured.
pub fn install_panic_hook() {
    // Keep console_error_panic_hook's console formatting.
    console_error_panic_hook::set_once();

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The previous hook first, so the console still gets the structured
        // message. If that panics, it is swallowed.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prev(info);
        }));

        // Best effort: the overlay and the report, each in catch_unwind so a
        // failure in one does not block the other and nothing re-panics
        // inside the hook.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_panic_overlay(info);
        }));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            post_panic_report(info);
        }));
    }));
}

/// The red reload overlay for a panic summary and a version. Pure: the
/// `summary` and `version` are HTML-escaped, so a panic payload like
/// `<script>alert(1)</script>` shows as text instead of running.
fn build_overlay_html(summary: &str, version: &str) -> String {
    format!(
        r#"<div id="fohmixer-panic-overlay" style="
            position:fixed;inset:0;z-index:2147483647;
            background:rgba(20,0,0,0.95);color:#fff;
            font-family:system-ui,-apple-system,sans-serif;
            display:flex;flex-direction:column;
            align-items:center;justify-content:center;
            padding:24px;text-align:center;">
            <h1 style="color:#ff4444;margin:0 0 16px 0;font-size:22px;">
                fohmixer encountered an error
            </h1>
            <p style="opacity:0.85;max-width:480px;margin:0 0 24px 0;
                font-family:monospace;font-size:13px;word-break:break-word;">
                {}
            </p>
            <button id="fohmixer-panic-reload" style="
                padding:14px 32px;font-size:16px;
                background:#ff4444;color:#fff;border:0;border-radius:8px;
                cursor:pointer;">
                Reload
            </button>
            <p style="opacity:0.5;font-size:11px;margin-top:16px;">{}</p>
        </div>"#,
        html_escape(summary),
        html_escape(version),
    )
}

fn render_panic_overlay(info: &std::panic::PanicHookInfo<'_>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else { return };

    let message = format_panic_message(info);
    let summary = truncate_for_display(&message, PANIC_MESSAGE_MAX_DISPLAY_CHARS);
    let version = fohmixer_proto::version_label();
    body.set_inner_html(&build_overlay_html(&summary, &version));

    // Wire the reload button.
    if let Some(btn) = document.get_element_by_id("fohmixer-panic-reload") {
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            if let Some(w) = web_sys::window() {
                let _ = w.location().reload();
            }
        }) as Box<dyn FnMut()>);
        if let Some(el) = btn.dyn_ref::<web_sys::HtmlElement>() {
            el.set_onclick(Some(closure.as_ref().unchecked_ref()));
        }
        // Leak the closure: the overlay is a terminal state, the page reloads.
        closure.forget();
    }
}

fn post_panic_report(info: &std::panic::PanicHookInfo<'_>) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let url = window
        .location()
        .pathname()
        .unwrap_or_else(|_| String::from("/"));
    let user_agent = window.navigator().user_agent().unwrap_or_default();
    let message = format_panic_message(info);
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_default();

    // No `backtrace`: a WASM build strips symbols, so a backtrace captured in
    // the browser is only instruction offsets. The console keeps the native
    // one (console_error_panic_hook).
    let body = serde_json::json!({
        "panic_message": message,
        "version": fohmixer_proto::VERSION,
        "git_hash": fohmixer_proto::git_hash(),
        "url": url,
        "user_agent": user_agent,
        "location": location,
    });
    let body_string = body.to_string();

    // Fire-and-forget fetch: not awaited, the page is about to reload.
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body_string));
    let Ok(headers) = web_sys::Headers::new() else {
        return;
    };
    let _ = headers.set("content-type", "application/json");
    opts.set_headers(&headers);
    if let Ok(request) = web_sys::Request::new_with_str_and_init("/api/client-error", &opts) {
        let _ = window.fetch_with_request(&request);
    }
}

/// A human-readable string from a panic payload. `PanicHookInfo` cannot be
/// built by user code, so this takes the `&dyn Any` payload (what
/// `PanicHookInfo::payload()` returns), which tests can build.
fn format_panic_payload(payload: &dyn std::any::Any) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::from("(unknown panic payload)")
    }
}

fn format_panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    format_panic_payload(info.payload())
}

pub(crate) fn truncate_for_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_for_display_below_limit_is_unchanged() {
        assert_eq!(truncate_for_display("short", 200), "short");
    }

    #[test]
    fn truncate_for_display_at_the_limit_is_unchanged() {
        let exact = "é".repeat(200);
        assert_eq!(truncate_for_display(&exact, 200), exact);
    }

    #[test]
    fn truncate_for_display_above_limit_is_truncated_with_ellipsis() {
        let long = "a".repeat(250);
        let result = truncate_for_display(&long, 200);
        assert_eq!(result.chars().count(), 201); // 200 + the ellipsis
        assert!(result.ends_with('…'));
        assert!(result.starts_with(&"a".repeat(200)));
    }

    #[test]
    fn html_escape_replaces_all_special_chars() {
        assert_eq!(
            html_escape(r#"<script>alert("&")</script>"#),
            "&lt;script&gt;alert(&quot;&amp;&quot;)&lt;/script&gt;"
        );
        assert_eq!(html_escape("plain"), "plain");
    }

    #[test]
    fn build_overlay_html_contains_title_and_reload_button() {
        let html = build_overlay_html("some panic", "v0.1.0");
        assert!(html.contains("fohmixer encountered an error"), "{html}");
        assert!(html.contains(r#"id="fohmixer-panic-reload""#), "{html}");
        assert!(html.contains(r#"id="fohmixer-panic-overlay""#), "{html}");
        assert!(html.contains("Reload"), "{html}");
    }

    #[test]
    fn build_overlay_html_interpolates_summary_and_version() {
        let html = build_overlay_html("my custom panic msg", "v9.9.9");
        assert!(html.contains("my custom panic msg"));
        assert!(html.contains("v9.9.9"));
    }

    #[test]
    fn build_overlay_html_escapes_hostile_input() {
        let hostile = r#"<script>alert("xss")</script>"#;
        let html = build_overlay_html(hostile, hostile);
        assert!(!html.contains("<script>"), "{html}");
        assert_eq!(html.matches("&lt;script&gt;").count(), 2, "{html}");
    }

    #[test]
    fn build_overlay_html_covers_the_viewport_on_top() {
        // The overlay must cover the whole viewport above whatever Leptos
        // rendered before the panic.
        let html = build_overlay_html("x", "v1.0");
        assert!(html.contains("position:fixed"));
        assert!(html.contains("inset:0"));
        assert!(html.contains("z-index:2147483647"));
    }

    #[test]
    fn format_panic_payload_extracts_str_slice() {
        let payload: &dyn std::any::Any = &"boom";
        assert_eq!(format_panic_payload(payload), "boom");
    }

    #[test]
    fn format_panic_payload_extracts_owned_string() {
        let payload = String::from("owned boom");
        assert_eq!(format_panic_payload(&payload), "owned boom");
    }

    #[test]
    fn format_panic_payload_handles_unknown_type_gracefully() {
        let payload: u32 = 42;
        assert_eq!(format_panic_payload(&payload), "(unknown panic payload)");
    }

    #[test]
    fn panic_message_display_cap_fits_a_tablet() {
        const { assert!(PANIC_MESSAGE_MAX_DISPLAY_CHARS <= 500) }
        const { assert!(PANIC_MESSAGE_MAX_DISPLAY_CHARS > 0) }
    }
}
