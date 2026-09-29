//! Tests of `client_report.rs`: the cleaning, the budget, the ring, and the
//! route in front of the real router (public, the kept fields, the status,
//! the body limit, the budget).

use std::net::Ipv4Addr;

use axum::body::Body;
use axum::http::{Request, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

/// A report's key: a documentation address.
fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

/// Every field set to `value`.
fn fields(value: &str) -> ReportFields {
    let v = || Some(value.to_string());
    ReportFields {
        kind: v(),
        display: v(),
        ua: v(),
        build: v(),
        host: v(),
        screen: v(),
        sw: v(),
        wake_lock: v(),
        visibility: v(),
        reconnects: v(),
        error: v(),
        fps: v(),
        long_frame_ms: v(),
        touches_max: v(),
        pointer: v(),
    }
}

fn report(kind: &str) -> ClientReport {
    let mut fields = fields("x");
    fields.kind = Some(kind.to_string());
    ClientReport {
        at: 1,
        peer: "192.0.2.1".into(),
        client: None,
        source: "lan".into(),
        fields,
    }
}

#[test]
fn the_limits_are_these() {
    assert_eq!(TEXT_MAX_CHARS, 300);
    assert_eq!(WORD_MAX_CHARS, 64);
    assert_eq!(RING, 50);
    assert_eq!(RATE_WINDOW, Duration::from_secs(10));
    assert_eq!(RATE_MAX, 60);
    assert_eq!(MAX_PEERS, 256);
}

#[test]
fn a_value_loses_its_control_characters() {
    assert_eq!(clean("load", 64), "load");
    assert_eq!(
        clean("a\nb\r\tc\u{1b}[31md\u{7f}e\u{85}f", 64),
        "abc[31mdef"
    );
    // A page cannot forge a second log line.
    let forged = clean("iPad\nINFO fohmixer_hub::auth: engineer logged in", 300);
    assert!(!forged.contains('\n'), "{forged}");
    assert_eq!(clean("", 64), "");
}

#[test]
fn a_long_value_is_cut_with_an_ellipsis() {
    let exact = "é".repeat(10);
    assert_eq!(clean(&exact, 10), exact);
    let long = format!("{exact}z");
    let cut = clean(&long, 10);
    assert_eq!(cut, format!("{exact}…"));
    assert_eq!(cut.chars().count(), 11);
    // Control characters do not count: they are gone before the cut.
    let with_breaks = format!("{}\n\n", "x".repeat(10));
    assert_eq!(clean(&with_breaks, 10), "x".repeat(10));
}

#[test]
fn every_field_is_cleaned_the_free_texts_are_longer() {
    assert_eq!(clean_fields(fields("a\nb")), fields("ab"));
    let long = "y".repeat(TEXT_MAX_CHARS + 5);
    let word = format!("{}…", "y".repeat(WORD_MAX_CHARS));
    let text = format!("{}…", "y".repeat(TEXT_MAX_CHARS));
    let mut expected = fields(&word);
    expected.ua = Some(text.clone());
    expected.error = Some(text);
    assert_eq!(clean_fields(fields(&long)), expected);
    // Exactly at the limits: kept whole.
    let mut exact = fields(&"w".repeat(WORD_MAX_CHARS));
    exact.ua = Some("u".repeat(TEXT_MAX_CHARS));
    exact.error = Some("e".repeat(TEXT_MAX_CHARS));
    assert_eq!(clean_fields(exact.clone()), exact);
    assert_eq!(
        clean_fields(ReportFields::default()),
        ReportFields::default()
    );
}

#[test]
fn the_source_is_the_access_class() {
    assert_eq!(source(Origin::Local), "lan");
    assert_eq!(source(Origin::Internet), "internet");
}

#[test]
fn an_internet_report_keeps_the_client_cloudflare_names() {
    let mut through_tunnel = HeaderMap::new();
    through_tunnel.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
    assert_eq!(
        forwarded_client(Origin::Internet, &through_tunnel),
        Some("203.0.113.7".to_string())
    );
    // On the LAN the peer is the client: nothing forwarded is taken.
    assert_eq!(forwarded_client(Origin::Local, &through_tunnel), None);
    assert_eq!(forwarded_client(Origin::Internet, &HeaderMap::new()), None);
    // Cleaned like a field.
    let mut long = HeaderMap::new();
    long.insert("cf-connecting-ip", "9".repeat(80).parse().unwrap());
    let kept = forwarded_client(Origin::Internet, &long).unwrap();
    assert_eq!(kept, format!("{}…", "9".repeat(WORD_MAX_CHARS)));
}

#[test]
fn a_missing_field_shows_as_a_dash() {
    assert_eq!(shown(None), "-");
    assert_eq!(shown(Some("held")), "held");
}

#[test]
fn a_peer_gets_rate_max_reports_per_window() {
    let mut budget = Budget::default();
    let t0 = Instant::now();
    for n in 1..=RATE_MAX {
        assert_eq!(budget.admit(ip(1), t0), Admit::Keep, "report {n}");
    }
    // Over: the first drop is the one logged, the rest are silent.
    assert_eq!(budget.admit(ip(1), t0), Admit::DropFirst);
    assert_eq!(budget.admit(ip(1), t0), Admit::Drop);
    // Another peer has its own budget.
    assert_eq!(budget.admit(ip(2), t0), Admit::Keep);
    // Until the window ends, still over; then a new window.
    let just_before = t0 + RATE_WINDOW - Duration::from_nanos(1);
    assert_eq!(budget.admit(ip(1), just_before), Admit::Drop);
    assert_eq!(budget.admit(ip(1), t0 + RATE_WINDOW), Admit::Keep);
    for n in 2..=RATE_MAX {
        assert_eq!(budget.admit(ip(1), t0 + RATE_WINDOW), Admit::Keep, "{n}");
    }
    assert_eq!(budget.admit(ip(1), t0 + RATE_WINDOW), Admit::DropFirst);
}

#[test]
fn the_budget_tracks_a_bounded_number_of_peers() {
    let mut budget = Budget::default();
    let t0 = Instant::now();
    let peer = |n: usize| IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + n as u32));
    let at = |ms: usize| t0 + Duration::from_millis(ms as u64);
    // Peer 0 opens the oldest window and uses it up.
    for _ in 0..=RATE_MAX {
        budget.admit(peer(0), at(0));
    }
    assert_eq!(budget.admit(peer(0), at(0)), Admit::Drop);
    for n in 1..MAX_PEERS {
        assert_eq!(budget.admit(peer(n), at(n)), Admit::Keep);
        assert_eq!(budget.peers(), n + 1);
    }
    // Full and every window live: a new peer takes the oldest one's place.
    assert_eq!(budget.admit(peer(MAX_PEERS), at(MAX_PEERS)), Admit::Keep);
    assert_eq!(budget.peers(), MAX_PEERS);
    // Peer 0 was forgotten: it starts a new window.
    assert_eq!(budget.admit(peer(0), at(MAX_PEERS)), Admit::Keep);
    assert_eq!(budget.peers(), MAX_PEERS);
    // Once every window ended, a new peer clears them all.
    let later = t0 + RATE_WINDOW + Duration::from_secs(1);
    assert_eq!(budget.admit(peer(MAX_PEERS + 1), later), Admit::Keep);
    assert_eq!(budget.peers(), 1);
}

#[test]
fn a_new_peer_clears_the_windows_that_ended() {
    let mut budget = Budget::default();
    let t0 = Instant::now();
    for last in 1..=3 {
        budget.admit(ip(last), t0);
    }
    budget.admit(ip(4), t0 + Duration::from_secs(5));
    assert_eq!(budget.peers(), 4);
    // At 10 s the first three ended; the fourth is live.
    budget.admit(ip(5), t0 + RATE_WINDOW);
    assert_eq!(budget.peers(), 2);
    // A known peer clears nothing.
    budget.admit(ip(4), t0 + Duration::from_secs(20));
    assert_eq!(budget.peers(), 2);
}

#[test]
fn the_newest_reports_are_kept() {
    let reports = Reports::default();
    let t0 = Instant::now();
    // Distinct peers, so none is over its budget.
    for n in 0..=RING {
        let kept = reports.record(ip(n as u8), t0, report(&format!("k{n}")));
        assert_eq!(kept, Admit::Keep);
    }
    let kept = reports.list();
    assert_eq!(kept.len(), RING);
    assert_eq!(kept[0].fields.kind.as_deref(), Some("k1"));
    assert_eq!(kept[RING - 1].fields.kind.as_deref(), Some("k50"));
}

#[test]
fn a_report_over_the_budget_is_not_kept() {
    let reports = Reports::default();
    let t0 = Instant::now();
    for n in 1..=RATE_MAX {
        let kept = reports.record(ip(1), t0, report(&format!("k{n}")));
        assert_eq!(kept, Admit::Keep, "{n}");
    }
    assert_eq!(reports.record(ip(1), t0, report("over")), Admit::DropFirst);
    assert_eq!(reports.record(ip(1), t0, report("over")), Admit::Drop);
    let kept = reports.list();
    assert_eq!(kept.len(), RING);
    let last = kept[RING - 1].fields.kind.clone();
    assert_eq!(last, Some(format!("k{RATE_MAX}")));
    assert!(
        kept.iter()
            .all(|r| r.fields.kind.as_deref() != Some("over"))
    );
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

/// The LAN client's address the listener would insert (see `routes.rs`
/// `lan`).
const LAN_PEER: ([u8; 4], u16) = ([10, 0, 0, 5], 40000);

fn lan(router: axum::Router) -> axum::Router {
    router.layer(axum::Extension(ConnectInfo(SocketAddr::from(LAN_PEER))))
}

async fn post(hub: &Hub, body: String) -> StatusCode {
    lan(crate::app_router(hub.clone()))
        .oneshot(
            Request::post("/api/client-report")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn status_json(hub: &Hub) -> Value {
    let token = hub.auth.issue().unwrap();
    let response = lan(crate::app_router(hub.clone()))
        .oneshot(
            Request::get("/api/status")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn a_report_needs_no_token_and_is_listed_in_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let hub = crate::test_hub(dir.path());
    let body = json!({
        "kind": "load",
        "display": "standalone",
        "ua": "Mozilla/5.0 (iPad)\r\nINFO forged line",
        "build": "0.1.0",
        "host": "foh.example.org",
        "screen": "1194x834@2",
        "sw": "registered",
        "wake_lock": "held",
        "visibility": "visible",
        "reconnects": "0",
        "cookie": "not kept",
    });
    assert_eq!(post(&hub, body.to_string()).await, StatusCode::NO_CONTENT);
    let status = status_json(&hub).await;
    let kept = status["client_reports"].as_array().unwrap();
    assert_eq!(kept.len(), 1);
    let at = kept[0]["at"].as_u64().unwrap();
    assert!(at.abs_diff(crate::auth::now_secs()) <= 5, "{at}");
    assert_eq!(
        kept[0],
        json!({
            "at": at,
            "peer": "10.0.0.5",
            "client": null,
            "source": "lan",
            "kind": "load",
            "display": "standalone",
            "ua": "Mozilla/5.0 (iPad)INFO forged line",
            "build": "0.1.0",
            "host": "foh.example.org",
            "screen": "1194x834@2",
            "sw": "registered",
            "wake_lock": "held",
            "visibility": "visible",
            "reconnects": "0",
            "error": null,
            "fps": null,
            "long_frame_ms": null,
            "touches_max": null,
            "pointer": null,
        })
    );
    // The connected-socket count keeps its meaning next to the reports.
    assert_eq!(status["clients"], 0);
    hub.stop();
}

#[tokio::test]
async fn a_perf_report_keeps_its_numbers_as_short_words() {
    // The page's frame rate and simultaneous touches (#5, K4): kept like
    // every other field, cut to a word.
    let dir = tempfile::tempdir().unwrap();
    let hub = crate::test_hub(dir.path());
    let body = json!({
        "kind": "perf",
        "screen": "1194x834@2",
        "fps": "59.9",
        "long_frame_ms": "34",
        "touches_max": "4",
        "pointer": format!("touch\n{}", "p".repeat(80)),
    });
    assert_eq!(post(&hub, body.to_string()).await, StatusCode::NO_CONTENT);
    let kept = hub.status().await.client_reports;
    assert_eq!(kept.len(), 1);
    let f = &kept[0].fields;
    assert_eq!(f.kind.as_deref(), Some("perf"));
    assert_eq!(f.fps.as_deref(), Some("59.9"));
    assert_eq!(f.long_frame_ms.as_deref(), Some("34"));
    assert_eq!(f.touches_max.as_deref(), Some("4"));
    let pointer = f.pointer.clone().unwrap();
    assert_eq!(pointer, format!("touch{}…", "p".repeat(WORD_MAX_CHARS - 5)));
    // In the status as text next to the page's other fields.
    let status = status_json(&hub).await;
    let listed = &status["client_reports"][0];
    assert_eq!(listed["fps"], "59.9");
    assert_eq!(listed["long_frame_ms"], "34");
    assert_eq!(listed["touches_max"], "4");
    assert_eq!(listed["ua"], Value::Null);
    hub.stop();
}

#[tokio::test]
async fn a_report_that_is_not_text_fields_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let hub = crate::test_hub(dir.path());
    assert_eq!(
        post(&hub, json!({"kind": 5}).to_string()).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // The page sends its numbers as text, never as JSON numbers.
    assert_eq!(
        post(&hub, json!({"kind": "perf", "fps": 59.9}).to_string()).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(&hub, "not json".to_string()).await,
        StatusCode::BAD_REQUEST
    );
    assert!(hub.status().await.client_reports.is_empty());
    hub.stop();
}

#[tokio::test]
async fn a_report_over_10_kib_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let hub = crate::test_hub(dir.path());
    // 10_240 bytes is the limit, as for /api/client-error.
    let under = format!(r#"{{"error":"{}"}}"#, "x".repeat(10_200));
    assert!(under.len() <= 10_240);
    assert_eq!(post(&hub, under).await, StatusCode::NO_CONTENT);
    let over = format!(r#"{{"error":"{}"}}"#, "x".repeat(10_240));
    assert!(over.len() > 10_240);
    assert_eq!(post(&hub, over).await, StatusCode::PAYLOAD_TOO_LARGE);
    let kept = hub.status().await.client_reports;
    assert_eq!(kept.len(), 1);
    // The accepted one was cut to the field limit.
    let error = kept[0].fields.error.as_deref().unwrap();
    assert_eq!(error.chars().count(), TEXT_MAX_CHARS + 1);
    hub.stop();
}

#[tokio::test]
async fn a_looping_page_fills_neither_the_log_nor_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let hub = crate::test_hub(dir.path());
    // Every answer is 204 (a 429 would be a console error on the page), but
    // only the budget's reports are kept.
    for n in 0..=RATE_MAX {
        let body = json!({"kind": "error", "error": format!("loop {n}")}).to_string();
        assert_eq!(post(&hub, body).await, StatusCode::NO_CONTENT, "{n}");
    }
    // The ring holds the newest: the last kept is the budget's last, the
    // report over the budget is not there.
    let kept = hub.status().await.client_reports;
    assert_eq!(kept.len(), RING);
    let last = kept[RING - 1].fields.error.clone();
    assert_eq!(last, Some(format!("loop {}", RATE_MAX - 1)));
    hub.stop();
}
