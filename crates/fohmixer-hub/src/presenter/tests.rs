//! The pure parts of Presenter's stage page behind the hub (#90).

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use serde_json::json;

use super::*;
use crate::ableset::Ended;

#[test]
fn presenters_paths_are_its_stage_pages_own_and_nothing_else() {
    for path in [
        "/stage",
        "/stage-displays",
        "/live/ws",
        "/healthz",
        "/group-colors",
        "/stage/layout",
        "/stage/snapshot",
        "/stage/broadcast-live",
        "/ui-pkg/presenter-ui-0123456789abcdef.js",
        "/bible/active-slide",
        "/ndi/time",
        "/whep/resource/1",
        "/presentations/42",
        "/libraries/7",
    ] {
        assert!(is_presenter_path(path), "{path}");
    }
    for path in [
        "/",
        "/api/status",
        "/ws",
        "/stages",
        "/stage-display",
        "/live",
        "/live/ws/x",
        "/ui-pkg",
        "/bible",
        "/ui/operator",
        "/healthz/x",
        "/group-colors/x",
        "/presentations",
        "/libraries",
        "/fohmixer-ui-0123456789ab.js",
        "/sw.js",
        "",
    ] {
        assert!(!is_presenter_path(path), "{path}");
    }
    assert_eq!(SOCKET, "/live/ws");
}

#[test]
fn a_dot_segment_is_found_plain_or_percent_encoded() {
    for path in [
        "/stage/..",
        "/stage/../api",
        "/ui-pkg/./x",
        "/ui-pkg/%2e%2e/x",
        "/ui-pkg/%2E%2E/x",
        "/ui-pkg/.%2e/x",
        "/ui-pkg/%2e",
    ] {
        assert!(dot_segment(path), "{path}");
    }
    for path in [
        "/stage",
        "/ui-pkg/a.js",
        "/ui-pkg/..a/x",
        "/ui-pkg/a../x",
        "/ui-pkg/...",
        "/",
    ] {
        assert!(!dot_segment(path), "{path}");
    }
}

#[test]
fn only_reads_and_presenters_socket_pass() {
    assert_eq!(pass(&Method::GET, "/stage", false), Pass::Page);
    assert_eq!(pass(&Method::HEAD, "/ui-pkg/a.js", false), Pass::Page);
    assert_eq!(pass(&Method::GET, "/live/ws", true), Pass::Socket);
    assert_eq!(pass(&Method::GET, "/live/ws", false), Pass::Page);
    assert_eq!(pass(&Method::GET, "/stage", true), Pass::Upgrade);
    for method in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
        Method::CONNECT,
        Method::TRACE,
    ] {
        assert_eq!(
            pass(&method, "/stage/layout", false),
            Pass::Method,
            "{method}"
        );
    }
    assert_eq!(pass(&Method::GET, "/stage/../api/auth", false), Pass::Path);
    assert_eq!(pass(&Method::POST, "/stage/../x", false), Pass::Path);
    assert_eq!(pass(&Method::GET, "/stage/../live/ws", true), Pass::Path);
    // Not one of Presenter's paths at all: fohmixer's own router.
    assert_eq!(pass(&Method::GET, "/live/../live/ws", true), Pass::No);
    assert_eq!(pass(&Method::GET, "/api/status", false), Pass::No);
    assert_eq!(pass(&Method::POST, "/api/auth", false), Pass::No);
    assert_eq!(pass(&Method::GET, "/ws", true), Pass::No);
}

#[test]
fn every_failure_has_its_words_kind_side_and_answer() {
    let cases = [
        (
            Failure::Method("m".into()),
            "method",
            false,
            StatusCode::METHOD_NOT_ALLOWED,
            "PRESENTER_METHOD",
        ),
        (
            Failure::Upgrade("u".into()),
            "upgrade",
            false,
            StatusCode::BAD_REQUEST,
            "PRESENTER_UPGRADE",
        ),
        (
            Failure::Target("t".into()),
            "target",
            false,
            StatusCode::BAD_REQUEST,
            "PRESENTER_TARGET",
        ),
        (
            Failure::Busy("b".into()),
            "busy",
            false,
            StatusCode::SERVICE_UNAVAILABLE,
            "PRESENTER_BUSY",
        ),
        (
            Failure::Down("d".into()),
            "down",
            true,
            StatusCode::BAD_GATEWAY,
            "PRESENTER_DOWN",
        ),
        (
            Failure::Timeout("o".into()),
            "timeout",
            true,
            StatusCode::GATEWAY_TIMEOUT,
            "PRESENTER_TIMEOUT",
        ),
    ];
    for (failure, kind, presenter, status, code) in cases {
        assert_eq!(failure.kind(), kind);
        assert_eq!(failure.is_presenter(), presenter, "{kind}");
        assert_eq!(failure.why().len(), 1, "{kind}");
        let answer = failure.answer(None);
        assert_eq!(answer.status(), status, "{kind}");
        let body = body_text(answer);
        assert!(body.contains(code), "{kind}: {body}");
    }
    let answer = Failure::Method("m".into()).answer(None);
    assert_eq!(answer.headers()[header::ALLOW], "GET, HEAD");
}

fn body_text(answer: Response) -> String {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let bytes = runtime
        .block_on(axum::body::to_bytes(answer.into_body(), 1 << 16))
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn presenter_down_is_a_page_for_a_navigation_and_json_for_anything_else() {
    for dest in ["document", "iframe"] {
        for (timed_out, status) in [
            (false, StatusCode::BAD_GATEWAY),
            (true, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let answer = down_answer(Some(dest), timed_out);
            assert_eq!(answer.status(), status);
            assert_eq!(answer.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                answer.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            let body = body_text(answer);
            assert!(body.contains("Presenter can't be reached"), "{body}");
            assert!(
                body.contains("http-equiv=\"refresh\" content=\"5\""),
                "{body}"
            );
        }
    }
    for dest in [None, Some("script"), Some("empty")] {
        let answer = down_answer(dest, false);
        assert_eq!(answer.status(), StatusCode::BAD_GATEWAY);
        let body = body_text(answer);
        assert!(body.contains("PRESENTER_DOWN"), "{body}");
        assert!(body.contains("Presenter does not answer"), "{body}");
    }
    let body = body_text(down_answer(Some("script"), true));
    assert!(body.contains("PRESENTER_TIMEOUT"), "{body}");
}

#[test]
fn every_answer_is_framed_by_fohmixers_own_page_only() {
    for path in ["/stage", "/stage/layout", "/ui-pkg/a.js", "/libraries/1"] {
        let mut headers = HeaderMap::new();
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        for name in ORIGIN_WIDE {
            headers.insert(name, HeaderValue::from_static("x"));
        }
        frame_headers(path, &mut headers);
        assert_eq!(headers[header::X_FRAME_OPTIONS], "SAMEORIGIN", "{path}");
        let policies: Vec<&str> = headers
            .get_all(header::CONTENT_SECURITY_POLICY)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        let mut wanted = vec!["default-src 'self'", "frame-ancestors 'self'"];
        if !runs_as_page(path) {
            wanted.push("sandbox; default-src 'none'");
        }
        assert_eq!(policies, wanted, "{path}");
        assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        for name in ORIGIN_WIDE {
            assert!(!headers.contains_key(name), "{path}: {name}");
        }
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }
    assert_eq!(FRAME_ANCESTORS, "frame-ancestors 'self'");
    assert_eq!(SANDBOXED, "sandbox; default-src 'none'");
    assert_eq!(
        ORIGIN_WIDE,
        ["clear-site-data", "service-worker-allowed", "refresh"]
    );
}

#[test]
fn only_the_stage_page_and_its_assets_run_as_a_page() {
    for path in ["/stage", "/ui-pkg/presenter-ui-0123.js", "/ui-pkg/a.wasm"] {
        assert!(runs_as_page(path), "{path}");
    }
    for path in [
        "/stage/layout",
        "/stages",
        "/stage-displays",
        "/ui-pkg",
        "/libraries/1/file.svg",
        "/presentations/2",
        "/bible/active-slide",
        "/live/ws",
    ] {
        assert!(!runs_as_page(path), "{path}");
    }
}

#[test]
fn a_proxy_record_says_error_for_presenters_fault_and_why_for_a_clients() {
    let down = Failure::Down("Presenter does not answer: refused".into());
    assert_eq!(
        proxy_fields("GET", "/stage", 502, &down, 3),
        json!({"method": "GET", "path": "/stage", "status": 502, "kind": "down",
               "skipped": 3, "error": "Presenter does not answer: refused"})
    );
    let method = Failure::Method("POST is never passed to Presenter".into());
    assert_eq!(
        proxy_fields("POST", "/stage/state", 405, &method, 0),
        json!({"method": "POST", "path": "/stage/state", "status": 405, "kind": "method",
               "skipped": 0, "why": "POST is never passed to Presenter"})
    );
}

#[test]
fn a_socket_record_names_its_page_at_the_open_and_its_life_at_the_close() {
    let opener = Opener {
        peer: "10.0.0.5".into(),
        forwarded: "-".into(),
        source: "lan",
        origin: "foh.example.org".into(),
    };
    assert_eq!(
        ws_fields(4, &opener, None, 2),
        json!({"what": "open", "client": 4, "peer": "10.0.0.5", "forwarded": "-",
               "source": "lan", "skipped": 2})
    );
    let ended = Ended {
        up: 10,
        down: 20,
        why: "closed".into(),
    };
    assert_eq!(
        ws_fields(4, &opener, Some((1500, &ended)), 0),
        json!({"what": "close", "client": 4, "ms": 1500, "up": 10, "down": 20,
               "why": "closed", "skipped": 0})
    );
}

#[tokio::test]
async fn the_link_is_noted_once_per_change_and_shown_in_the_status() {
    let (events, records) = EventLog::channel(16);
    let site = Presenter::new("http://10.0.0.9:8080", events);
    assert_eq!(site.status().online, None);
    assert_eq!(site.noted(link::News::Up), Some(true));
    assert_eq!(site.noted(link::News::Up), None, "no change");
    site.seen(Some(9), Some("timer"));
    site.seen(None, Some("preach"));
    let status = site.status();
    assert_eq!(status.url, "http://10.0.0.9:8080");
    assert_eq!(status.online, Some(true));
    assert_eq!(status.layouts, 9);
    assert_eq!(status.current.as_deref(), Some("preach"));
    assert_eq!(status.sockets, 0);
    assert_eq!(
        site.noted(link::News::Failed("gone".into())),
        None,
        "one failure"
    );
    assert_eq!(site.status().last_error.as_deref(), Some("gone"));
    assert_eq!(
        site.noted(link::News::Failed("refused".into())),
        Some(false)
    );
    let first = records.try_recv().unwrap();
    assert_eq!(first["ev"], "presenter_link");
    assert_eq!(first["up"], true);
    let second = records.try_recv().unwrap();
    assert_eq!(second["up"], false);
    assert_eq!(second["error"], "refused");
    assert_eq!(second["failures"], 2);
    assert!(records.try_recv().is_err());
}

#[tokio::test]
async fn a_stopped_presenter_has_every_socket_slot_back() {
    let (events, _) = EventLog::channel(16);
    let site = Presenter::new("http://127.0.0.1:9", events);
    site.stop();
    assert!(site.closed(Duration::from_millis(100)).await);
    let held = Arc::clone(&site.sockets).try_acquire_owned().unwrap();
    assert_eq!(site.status().sockets, 1);
    assert!(!site.closed(Duration::from_millis(50)).await);
    drop(held);
}

#[tokio::test]
async fn a_request_goes_to_presenter_with_its_own_host_and_no_proxy_header() {
    let (events, _) = EventLog::channel(16);
    let site = Presenter::new("http://10.0.0.9:8080", events);
    let request = Request::builder()
        .uri("/stage?preview=1")
        .header(header::HOST, "foh.example.org")
        .header("cf-connecting-ip", "198.51.100.7")
        .header("x-forwarded-for", "198.51.100.7")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::empty())
        .unwrap();
    let sent = site.upstream(request).unwrap();
    assert_eq!(sent.uri(), "http://10.0.0.9:8080/stage?preview=1");
    assert_eq!(sent.headers()[header::HOST], "10.0.0.9:8080");
    assert!(!sent.headers().contains_key("cf-connecting-ip"));
    assert!(!sent.headers().contains_key("x-forwarded-for"));
    assert!(!sent.headers().contains_key(header::CONNECTION));
    assert_eq!(sent.version(), Version::HTTP_11);
}

#[tokio::test]
async fn a_failed_request_is_recorded_within_its_ration_and_answered() {
    let (events, records) = EventLog::channel(64);
    let site = Presenter::new("http://10.0.0.9:8080", events);
    for _ in 0..FAULT_RECORDS + 2 {
        let answer = site.failed(
            &Method::POST,
            "/stage/state",
            None,
            Failure::Method("POST is never passed to Presenter".into()),
        );
        assert_eq!(answer.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
    let written: Vec<_> = records.try_iter().collect();
    assert_eq!(written.len(), FAULT_RECORDS as usize, "{written:?}");
    assert_eq!(written[0]["ev"], "presenter_proxy");
    assert_eq!(written[0]["kind"], "method");
    assert_eq!(FAULT_RECORDS, 4);
    assert_eq!(WS_RECORDS, 8);
    assert!(site.warn_due("other"), "a kind of its own");
    assert!(!site.warn_due("other"), "once per gap");
}
