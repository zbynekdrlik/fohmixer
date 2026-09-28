//! The hub connection's rules (S4 design note §4, §6): the reconnect
//! schedule (under 2 s, never giving up), the protocol handshake (iemmixer's
//! `handshake.rs` @ 22372bc: a bounded reload on a protocol mismatch), the
//! WebSocket URL, and what an HTTP answer of the hub means. The browser
//! calls (`fetch_text`, `reload`, `last_reload`) are thin wrappers.

use fohmixer_proto::client::UI_PROTO;
use fohmixer_proto::layout::LayoutResponse;
use wasm_bindgen::JsCast;

/// The first reconnect waits this long, doubling up to `RECONNECT_MAX_MS`.
pub const RECONNECT_FIRST_MS: f64 = 500.0;
pub const RECONNECT_MAX_MS: f64 = 2000.0;
/// No hello this long after the socket opened: one reload.
pub const HELLO_TIMEOUT_MS: f64 = 3000.0;
/// At most one handshake reload per this interval.
pub const RELOAD_GAP_MS: f64 = 60_000.0;
/// Where the last handshake reload time is kept (per browser).
const RELOAD_KEY: &str = "fohmixer_proto_reload_at";

/// The wait before reconnect attempt `attempt` (0 for the first).
pub fn reconnect_delay(attempt: u32) -> f64 {
    let doubled = RECONNECT_FIRST_MS * 2f64.powi(attempt.min(16) as i32);
    doubled.min(RECONNECT_MAX_MS)
}

/// The client WebSocket URL of a page served from `host` over `scheme`
/// (`http:`/`https:`), with the engineer's token.
pub fn ws_url(page_scheme: &str, host: &str, token: &str) -> String {
    let scheme = if page_scheme == "https:" { "wss" } else { "ws" };
    format!("{scheme}://{host}/ws?token={token}&proto={UI_PROTO}")
}

/// What the page does about the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Reload,
}

/// Whether a reload is allowed at `now` after the last one at `last`.
fn may_reload(now: f64, last: Option<f64>) -> bool {
    last.is_none_or(|t| now - t >= RELOAD_GAP_MS || now < t)
}

/// The decision on the hub's hello: reload (at most once a minute) when
/// the hub does not serve this page's protocol.
pub fn on_hello(server_proto: u32, min_client_proto: u32, now: f64, last: Option<f64>) -> Decision {
    let served = (min_client_proto..=server_proto).contains(&UI_PROTO);
    if served || !may_reload(now, last) {
        Decision::Keep
    } else {
        Decision::Reload
    }
}

/// The decision when the hub never said hello, or closed the socket with
/// the reload code.
pub fn on_missing_hello(now: f64, last: Option<f64>) -> Decision {
    if may_reload(now, last) {
        Decision::Reload
    } else {
        Decision::Keep
    }
}

/// What `GET /api/layout` answered.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutFetch {
    /// The served layout.
    Layout(Box<LayoutResponse>),
    /// The token is not (or no longer) valid: back to the login.
    Unauthorized,
    /// The hub serves no layout yet (it says why).
    NoLayout(String),
    /// Anything else: try again later.
    Failed(String),
}

/// Classifies an answer of `GET /api/layout`.
pub fn layout_fetch(status: u16, body: &str) -> LayoutFetch {
    match status {
        200 => match serde_json::from_str::<LayoutResponse>(body) {
            Ok(layout) => LayoutFetch::Layout(Box::new(layout)),
            Err(e) => LayoutFetch::Failed(format!("unreadable layout: {e}")),
        },
        401 => LayoutFetch::Unauthorized,
        503 => LayoutFetch::NoLayout(api_message(body)),
        other => LayoutFetch::Failed(format!("HTTP {other}: {}", api_message(body))),
    }
}

/// The `message` of an API error body, or the body itself.
pub fn api_message(body: &str) -> String {
    serde_json::from_str::<fohmixer_proto::client::ApiError>(body)
        .map_or_else(|_| body.trim().to_string(), |e| e.message)
}

/// Runs an HTTP request against the hub: `(status, body)`, or the error
/// of a request that never got an answer.
pub async fn fetch_text(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let window = web_sys::window().ok_or("no window")?;
    let opts = web_sys::RequestInit::new();
    opts.set_method(method);
    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    if let Some(body) = body {
        opts.set_body(&wasm_bindgen::JsValue::from_str(body));
        headers
            .set("content-type", "application/json")
            .map_err(|e| format!("{e:?}"))?;
    }
    if let Some(token) = token {
        headers
            .set("authorization", &format!("Bearer {token}"))
            .map_err(|e| format!("{e:?}"))?;
    }
    opts.set_headers(&headers);
    let request =
        web_sys::Request::new_with_str_and_init(url, &opts).map_err(|e| format!("{e:?}"))?;
    let answer = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let response: web_sys::Response = answer.dyn_into().map_err(|e| format!("{e:?}"))?;
    let status = response.status();
    let text = response.text().map_err(|e| format!("{e:?}"))?;
    let text = wasm_bindgen_futures::JsFuture::from(text)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok((status, text.as_string().unwrap_or_default()))
}

/// The last handshake reload time (local storage).
pub fn last_reload() -> Option<f64> {
    crate::dom::storage_get(RELOAD_KEY)?.parse().ok()
}

/// Records the reload time and reloads the page.
pub fn reload(now: f64, why: &str) {
    crate::dom::log(&format!("protocol handshake: {why}; reloading"));
    crate::dom::storage_set(RELOAD_KEY, &now.to_string());
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reconnects_start_at_half_a_second_and_stay_under_two() {
        assert_eq!(reconnect_delay(0), 500.0);
        assert_eq!(reconnect_delay(1), 1000.0);
        assert_eq!(reconnect_delay(2), 2000.0);
        assert_eq!(reconnect_delay(3), 2000.0);
        assert_eq!(reconnect_delay(u32::MAX), 2000.0);
    }

    #[test]
    fn the_socket_url_carries_the_token_and_the_protocol() {
        assert_eq!(
            ws_url("http:", "10.0.0.5:8480", "t.k.n"),
            "ws://10.0.0.5:8480/ws?token=t.k.n&proto=1"
        );
        assert_eq!(
            ws_url("https:", "foh.local", "x"),
            "wss://foh.local/ws?token=x&proto=1"
        );
    }

    #[test]
    fn a_served_protocol_keeps_the_page() {
        assert_eq!(on_hello(1, 1, 0.0, None), Decision::Keep);
        assert_eq!(on_hello(2, 1, 0.0, None), Decision::Keep);
        // The hub no longer serves this page, or is older than it.
        assert_eq!(on_hello(3, 2, 1e6, None), Decision::Reload);
        assert_eq!(on_hello(0, 0, 1e6, None), Decision::Reload);
    }

    #[test]
    fn reloads_are_at_most_one_per_minute() {
        let t = 1_000_000.0;
        assert_eq!(on_hello(3, 2, t, Some(t - 59_999.0)), Decision::Keep);
        assert_eq!(on_hello(3, 2, t, Some(t - 60_000.0)), Decision::Reload);
        assert_eq!(on_missing_hello(t, Some(t - 1_000.0)), Decision::Keep);
        assert_eq!(
            on_missing_hello(t, Some(t - RELOAD_GAP_MS)),
            Decision::Reload
        );
        assert_eq!(on_missing_hello(t, None), Decision::Reload);
        assert_eq!(
            on_missing_hello(t, Some(t)),
            Decision::Keep,
            "not twice at once"
        );
        // A clock that went backwards does not block reloads for ever.
        assert_eq!(on_missing_hello(t, Some(t + 5_000.0)), Decision::Reload);
    }

    #[test]
    fn layout_answers_are_classified() {
        let body = json!({"rev": 3, "layout": {
            "schema": 1, "canvas": {"w": 100, "h": 100},
            "pages": [{"id": "p", "title": "P"}]}})
        .to_string();
        let LayoutFetch::Layout(layout) = layout_fetch(200, &body) else {
            panic!("a layout")
        };
        assert_eq!(layout.rev, 3);
        assert_eq!(layout.layout.pages[0].id, "p");
        assert!(
            matches!(layout_fetch(200, "{"), LayoutFetch::Failed(e) if e.starts_with("unreadable layout: "))
        );
        assert_eq!(layout_fetch(401, ""), LayoutFetch::Unauthorized);
        assert_eq!(
            layout_fetch(
                503,
                r#"{"code":"NO_LAYOUT","message":"layout.json: schema 2"}"#
            ),
            LayoutFetch::NoLayout("layout.json: schema 2".into())
        );
        assert_eq!(
            layout_fetch(500, " oops \n"),
            LayoutFetch::Failed("HTTP 500: oops".into())
        );
        assert_eq!(
            layout_fetch(421, r#"{"code":"UNKNOWN_HOST","message":"no"}"#),
            LayoutFetch::Failed("HTTP 421: no".into())
        );
    }
}
