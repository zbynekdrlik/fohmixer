//! The pure parts of the hub's link to Presenter (#90).

use serde_json::json;

use super::*;

#[test]
fn the_hubs_socket_is_presenters_live_socket_as_a_preview() {
    assert_eq!(
        socket_url("http://10.0.0.9:8080"),
        "ws://10.0.0.9:8080/live/ws?surface=fohmixer&preview=1"
    );
    assert_eq!(
        socket_url("http://[fd00::9]"),
        "ws://[fd00::9]/live/ws?surface=fohmixer&preview=1"
    );
}

#[test]
fn only_a_stage_layout_event_names_a_layout() {
    assert_eq!(
        stage_layout_event(r#"{"type":"stage_layout","code":"timer"}"#).as_deref(),
        Some("timer")
    );
    for text in [
        r#"{"type":"broadcast_live","enabled":true}"#,
        r#"{"type":"stage_layout"}"#,
        r#"{"code":"timer"}"#,
        r#"{"type":"stage_layout","code":7}"#,
        "not json",
        "",
    ] {
        assert_eq!(stage_layout_event(text), None, "{text}");
    }
}

#[test]
fn the_layouts_and_the_current_one_are_read_from_200_answers_only() {
    let body = json!([{"code": "timer", "name": "TIMER", "description": "Countdown"},
                      {"code": "api", "name": "API"}])
    .to_string();
    let read = listed(200, body.as_bytes()).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].code, "timer");
    assert_eq!(read[1].description, "");
    assert_eq!(
        listed(500, body.as_bytes()).unwrap_err(),
        "GET /stage-displays answers 500"
    );
    assert!(
        listed(200, b"{}")
            .unwrap_err()
            .starts_with("GET /stage-displays: ")
    );
    let layout = json!({"code": "timer", "layout": {"code": "timer", "name": "TIMER"}}).to_string();
    assert_eq!(
        layout_code(200, layout.as_bytes(), "GET /stage/layout").unwrap(),
        "timer"
    );
    assert_eq!(
        layout_code(503, layout.as_bytes(), "GET /stage/layout").unwrap_err(),
        "GET /stage/layout answers 503"
    );
    assert!(
        layout_code(200, b"[]", "GET /stage/layout")
            .unwrap_err()
            .starts_with("GET /stage/layout: ")
    );
}

#[test]
fn a_switch_answer_is_the_layout_on_stage_or_why_not() {
    let ok = json!({"code": "preach", "layout": {"code": "preach"}}).to_string();
    assert_eq!(
        switch_answer(200, ok.as_bytes(), "preach").unwrap(),
        "preach"
    );
    assert_eq!(
        switch_answer(400, b"code cannot be empty", "").unwrap_err(),
        "Presenter refused the layout \"\""
    );
    assert_eq!(
        switch_answer(404, b"unknown", "nope").unwrap_err(),
        "Presenter does not list \"nope\""
    );
    assert_eq!(
        switch_answer(500, b"", "timer").unwrap_err(),
        "POST /stage/layout answers 500"
    );
    assert_eq!(
        switch_answer(201, ok.as_bytes(), "preach").unwrap_err(),
        "POST /stage/layout answers 201"
    );
}

#[test]
fn the_link_is_down_after_two_failures_in_a_row_and_up_at_a_start() {
    let mut state = LinkState::default();
    assert_eq!(state.note(News::Failed("refused".into())), None);
    assert_eq!(
        state.note(News::Failed("refused again".into())),
        Some(Change::Down {
            failures: 2,
            error: "refused again".into()
        })
    );
    assert_eq!(
        state.note(News::Failed("still".into())),
        None,
        "already down"
    );
    assert_eq!(state.error.as_deref(), Some("still"));
    assert_eq!(state.note(News::Up), Some(Change::Up { failures: 3 }));
    assert_eq!(state.error, None);
    assert_eq!(state.link.online, Some(true));
    assert_eq!(state.note(News::Up), None);
    // A session's end alone keeps it up; the next try's failure takes it down.
    assert_eq!(state.note(News::Failed("closed".into())), None);
    assert_eq!(state.link.online, Some(true));
    assert_eq!(state.note(News::Up), None, "back before it went down");
}

#[test]
fn a_change_is_its_record_and_says_whether_presenter_is_up() {
    let up = Change::Up { failures: 1 };
    assert!(up.up());
    assert_eq!(up.fields(), json!({"up": true, "failures": 1}));
    let down = Change::Down {
        failures: 2,
        error: "refused".into(),
    };
    assert!(!down.up());
    assert_eq!(
        down.fields(),
        json!({"up": false, "failures": 2, "error": "refused"})
    );
}

#[test]
fn only_the_end_of_a_session_that_was_up_is_news() {
    assert_eq!(ended_news(Some(true)), Some("the session ended"));
    assert_eq!(ended_news(Some(false)), None);
    assert_eq!(ended_news(None), None);
}

#[test]
fn the_links_bounds_are_as_documented() {
    assert_eq!(RETRY, Duration::from_secs(2));
    assert_eq!(READ_TIMEOUT, Duration::from_secs(5));
    assert_eq!(PING_EVERY, Duration::from_secs(5));
    assert_eq!(SILENCE, Duration::from_secs(15));
    assert_eq!(MAX_BODY, 1_048_576);
    assert_eq!(MAX_CODE, 64);
}

#[tokio::test]
async fn a_switch_to_a_presenter_that_does_not_answer_says_so_with_its_request() {
    // Port 9 on loopback: nothing listens there on the runners.
    let api = Api::new("http://127.0.0.1:9");
    let why = api.switch("timer").await.unwrap_err();
    assert!(why.starts_with("POST /stage/layout: "), "{why}");
    let why = api.layouts().await.unwrap_err();
    assert!(why.starts_with("GET /stage-displays: "), "{why}");
    let why = api.current().await.unwrap_err();
    assert!(why.starts_with("GET /stage/layout: "), "{why}");
}

#[tokio::test]
async fn every_switch_is_carried_on_its_own_and_its_outcome_handed_back() {
    let api = Api::new("http://127.0.0.1:9");
    let (tx, rx) = mpsc::unbounded_channel();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(switches(api, rx, move |switched| {
        let _ = done_tx.send(switched);
    }));
    for seq in [1, 2] {
        tx.send(SwitchRequest {
            client: 4,
            seq,
            code: "timer".into(),
            from: None,
        })
        .unwrap();
    }
    let mut seqs = Vec::new();
    for _ in 0..2 {
        let (request, result, _) = tokio::time::timeout(Duration::from_secs(10), done_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.client, 4);
        assert!(result.is_err());
        seqs.push(request.seq);
    }
    seqs.sort_unstable();
    assert_eq!(seqs, [1, 2]);
    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("it ends with its channel")
        .unwrap();
}

/// A Presenter that answers the session's two reads and opens its live
/// socket, then neither reads nor writes on it again (gone without a close:
/// a power cut, a Wi-Fi drop), or (`answers`) only answers its pings: its
/// address.
async fn quiet_presenter(answers: bool) -> std::net::SocketAddr {
    use axum::extract::ws::WebSocketUpgrade;
    use axum::routing::get;
    let app = axum::Router::new()
        .route(
            "/stage-displays",
            get(|| async { axum::Json(json!([{"code": "timer", "name": "TIMER"}])) }),
        )
        .route(
            "/stage/layout",
            get(|| async { axum::Json(json!({"code": "timer", "layout": {}})) }),
        )
        .route(
            "/live/ws",
            get(move |ws: WebSocketUpgrade| async move {
                ws.on_upgrade(move |mut socket| async move {
                    if answers {
                        // Read: axum answers each ping with a pong.
                        while let Some(Ok(_)) = socket.recv().await {}
                    } else {
                        // Held, never read: no pong ever answers a ping.
                        std::future::pending::<()>().await;
                    }
                })
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

#[tokio::test(start_paused = true)]
async fn a_live_socket_silent_for_15_s_ends_its_session_though_the_pings_still_go_out() {
    let addr = quiet_presenter(false).await;
    let base = format!("http://{addr}");
    let (events, _) = crate::events::EventLog::channel(16);
    let site = Presenter::new(&base, events);
    let api = Api::new(&base);
    let told = std::sync::Mutex::new(Vec::new());
    let started = tokio::time::Instant::now();
    let tell = |event| told.lock().unwrap().push(event);
    let session = session(&site, &api, &tell);
    let why = tokio::time::timeout(Duration::from_secs(120), session)
        .await
        .expect("the session ends once the socket stays silent");
    let after = started.elapsed();
    assert_eq!(why, "the live socket was silent for 15000 ms");
    assert!(
        after >= SILENCE && after < SILENCE + PING_EVERY,
        "ended after {after:?}"
    );
    let told = told.into_inner().unwrap();
    assert!(
        matches!(told.first(), Some(LinkEvent::Listed { current, .. }) if current == "timer"),
        "{told:?}"
    );
    assert_eq!(told.get(1), Some(&LinkEvent::Online(true)));
}

#[tokio::test(start_paused = true)]
async fn a_live_socket_that_only_answers_the_pings_keeps_its_session() {
    let addr = quiet_presenter(true).await;
    let base = format!("http://{addr}");
    let (events, _) = crate::events::EventLog::channel(16);
    let site = Presenter::new(&base, events);
    let api = Api::new(&base);
    let tell = |_| {};
    let session = session(&site, &api, &tell);
    // Every pong is a frame heard: no silence however long nothing happens.
    let lived = tokio::time::timeout(SILENCE * 4, session).await;
    assert!(lived.is_err(), "the session ended: {lived:?}");
}
