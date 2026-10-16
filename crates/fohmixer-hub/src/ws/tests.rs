//! Tests of `ws.rs`: who opened a socket, as its connect and disconnect
//! lines name it (#9): the peer, the address Cloudflare forwarded, the
//! Access check's source (`lan` / `internet`) and the host of the page's
//! `Origin`.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::http::{HeaderMap, HeaderName, HeaderValue};

use super::*;
use crate::access::PROXY_HEADERS;
use crate::client_report::WORD_MAX_CHARS;

/// The headers of an upgrade request.
fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

/// A peer address (its port is not on the line).
fn peer(ip: &str) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), 50000)
}

fn named(peer: &str, forwarded: &str, source: &'static str, origin: &str) -> Opener {
    Opener {
        peer: peer.to_string(),
        forwarded: forwarded.to_string(),
        source,
        origin: origin.to_string(),
    }
}

#[test]
fn a_lan_socket_names_its_peer_the_lan_and_its_page_host() {
    let upgrade = headers(&[("origin", "http://mixer.example:8080")]);
    // On the LAN the peer is the client: nothing forwarded.
    assert_eq!(
        opener(peer("10.0.0.5"), &upgrade),
        named("10.0.0.5", "-", "lan", "mixer.example:8080")
    );
    // A browser on the PC itself is the LAN too.
    assert_eq!(
        opener(peer("127.0.0.1"), &upgrade),
        named("127.0.0.1", "-", "lan", "mixer.example:8080")
    );
}

#[test]
fn a_socket_through_a_proxy_is_from_the_internet() {
    // Through the tunnel the peer is cloudflared on the PC; Cloudflare's
    // header says the internet and names the client.
    let tunnel = headers(&[
        ("origin", "https://foh.example.org"),
        ("cf-connecting-ip", "203.0.113.7"),
    ]);
    assert_eq!(
        opener(peer("127.0.0.1"), &tunnel),
        named("127.0.0.1", "203.0.113.7", "internet", "foh.example.org")
    );
    // Every proxy header the Access check knows, as the Access check itself
    // classifies it (one classifier).
    for name in PROXY_HEADERS {
        let proxied = headers(&[(name, "192.0.2.7")]);
        assert_eq!(
            opener(peer("10.0.0.5"), &proxied).source,
            "internet",
            "{name}"
        );
    }
    // Only Cloudflare's header names the client (`forwarded_client`).
    let other_proxy = headers(&[("x-forwarded-for", "192.0.2.7")]);
    assert_eq!(opener(peer("127.0.0.1"), &other_proxy).forwarded, "-");
    // A public peer without the tunnel (a port-forward): the internet too.
    assert_eq!(
        opener(peer("198.51.100.7"), &HeaderMap::new()),
        named("198.51.100.7", "-", "internet", "-")
    );
}

#[test]
fn a_socket_is_the_lans_exactly_when_its_source_is() {
    // #90 PR C review M2: the router tells AbleSet's page this, so a page
    // on the LAN never shows AbleSet's internet sign-in.
    let lan = opener(peer("10.0.0.5"), &HeaderMap::new());
    assert!(lan.is_lan(), "{lan:?}");
    assert!(opener(peer("127.0.0.1"), &HeaderMap::new()).is_lan());
    let tunnel = headers(&[("cf-connecting-ip", "203.0.113.7")]);
    let internet = opener(peer("127.0.0.1"), &tunnel);
    assert!(!internet.is_lan(), "{internet:?}");
    assert!(!opener(peer("198.51.100.7"), &HeaderMap::new()).is_lan());
    assert!(!named("10.0.0.5", "-", "internet", "-").is_lan());
    assert!(named("10.0.0.5", "-", "lan", "-").is_lan());
}

#[test]
fn the_page_host_is_cleaned_and_no_origin_is_a_dash() {
    let host = |origin: Option<&str>| {
        let upgrade = origin.map_or_else(HeaderMap::new, |o| headers(&[("origin", o)]));
        opener(peer("10.0.0.5"), &upgrade).origin
    };
    // Not a browser (curl, a test client): no Origin.
    assert_eq!(host(None), "-");
    assert_eq!(host(Some("https://foh.example.org")), "foh.example.org");
    // Not http(s): kept as it came.
    assert_eq!(host(Some("null")), "null");
    // A control character cannot split the line; a long host is cut.
    assert_eq!(host(Some("https://foh.exa\tmple.org")), "foh.example.org");
    let long = format!("https://{}", "a".repeat(WORD_MAX_CHARS + 10));
    assert_eq!(
        host(Some(long.as_str())),
        format!("{}…", "a".repeat(WORD_MAX_CHARS))
    );
}

/// A log sink a test reads back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The tests' global subscriber: it writes nothing and is interested in every
/// callsite (`sometimes`: each event then asks the thread's own subscriber).
/// Without it, while `logged`'s scoped subscriber is the only one alive,
/// tracing-core asks the registering thread's own default about a new
/// callsite; another test's thread has none and caches the callsite as
/// never, for `logged` too (dev run 38919475834 lost ableset's lock warn so).
struct Quiet;

impl tracing::Subscriber for Quiet {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// What `f` logs, formatted as the hub's log formats it (without the time
/// and the colours); also `ableset`'s tests (#90).
pub(crate) fn logged(f: impl FnOnce()) -> String {
    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(|| {
        tracing::subscriber::set_global_default(Quiet)
            .expect("the hub's unit tests set no other global subscriber");
    });
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8(bytes).unwrap()
}

/// A warn line whose callsite a test reaches on two threads.
fn reached_on_two_threads() {
    tracing::warn!("logged: a callsite another thread reached first");
}

#[test]
fn logged_keeps_a_line_whose_callsite_another_thread_reached_first() {
    // The `test` job of dev run 38919475834: ableset's lock warn went missing
    // from `logged` although its record was written. With `logged`'s scoped
    // subscriber the only one alive, tracing-core asks the registering
    // thread's own default about a new callsite; another test's thread has
    // none, so the callsite was cached as never, for `logged` too.
    let log = logged(|| {
        std::thread::spawn(reached_on_two_threads).join().unwrap();
        reached_on_two_threads();
    });
    assert!(
        log.contains("logged: a callsite another thread reached first"),
        "{log:?}"
    );
    // A thread with no subscriber of its own has the tests' quiet one.
    assert!(tracing::dispatcher::has_been_set());
}

#[test]
fn the_connect_and_disconnect_lines_name_the_peer_the_forwarded_client_the_source_and_the_page_host()
 {
    let lan = opener(
        peer("10.0.0.5"),
        &headers(&[("origin", "http://mixer.example:8080")]),
    );
    // Tunnel-shaped: cloudflared on the PC, the client in Cloudflare's header.
    let tunnel = opener(
        peer("127.0.0.1"),
        &headers(&[
            ("origin", "https://foh.example.org"),
            ("cf-connecting-ip", "203.0.113.7"),
        ]),
    );
    let log = logged(|| {
        log_socket("client connected", 1, &lan);
        log_socket("client disconnected", 2, &tunnel);
    });
    // The page host and the forwarded address are the client's own text:
    // quoted, like a client report's fields. The source is quoted as a
    // client report's is, so one grep finds both kinds of line. `client`
    // stays the socket number.
    let connected = r#"client connected client=1 peer=10.0.0.5 forwarded="-" source="lan" origin="mixer.example:8080""#;
    assert!(log.contains(&format!("{connected}\n")), "{log}");
    let disconnected = r#"client disconnected client=2 peer=127.0.0.1 forwarded="203.0.113.7" source="internet" origin="foh.example.org""#;
    assert!(log.contains(&format!("{disconnected}\n")), "{log}");
}

#[test]
fn a_page_host_cannot_add_fields_to_its_line() {
    // Spaces and a quote in the Origin stay inside the quoted value: the
    // line keeps its own peer and source.
    let forged = opener(
        peer("10.0.0.5"),
        &headers(&[("origin", r#"http://x source=internet peer=192.0.2.9 "y"#)]),
    );
    let log = logged(|| log_socket("client connected", 3, &forged));
    let line = r#"client connected client=3 peer=10.0.0.5 forwarded="-" source="lan" origin="x source=internet peer=192.0.2.9 \"y""#;
    assert!(log.contains(&format!("{line}\n")), "{log}");
}

#[test]
fn a_forwarded_address_cannot_add_fields_to_its_line() {
    // The forwarded address is a header too: quoted, it stays one value.
    let forged = opener(
        peer("127.0.0.1"),
        &headers(&[("cf-connecting-ip", "203.0.113.7 source=lan")]),
    );
    let log = logged(|| log_socket("client connected", 4, &forged));
    let line = r#"client connected client=4 peer=127.0.0.1 forwarded="203.0.113.7 source=lan" source="internet" origin="-""#;
    assert!(log.contains(&format!("{line}\n")), "{log}");
}

#[test]
fn a_page_time_reaches_the_pong_and_the_event_log_to_the_bit() {
    // #43: serde_json's `float_roundtrip`. A page time has 17 significant
    // digits; serde_json's default parser reads these three one bit off, so a
    // pong would not echo its ping's own time and the event log would not
    // hold the page's (the first E2E run of `link.spec.ts` caught it).
    for text in [
        "1791045199487.8555",
        "1791045199790.3975",
        "1791045199943.1965",
    ] {
        let msg: ClientMsg =
            serde_json::from_str(&format!(r#"{{"type":"ping","n":1,"t":{text}}}"#)).unwrap();
        let ClientMsg::Ping { t, .. } = msg else {
            panic!("a ping")
        };
        assert_eq!(t, text.parse::<f64>().unwrap(), "{text}");
        let pong = serde_json::to_string(&ServerMsg::Pong { n: 1, t, h: 0.0 }).unwrap();
        assert!(pong.contains(&format!("\"t\":{text}")), "{pong}");
    }
}

#[test]
fn a_clock_step_warning_is_logged_at_most_once_per_ten_seconds() {
    // #54: a page with a bad clock must not fill the log with one warn
    // line per ping; the ping record keeps its `clock_step_ms` anyway.
    assert!(step_warn_due(None, 5_000.0), "the first one is logged");
    assert!(
        !step_warn_due(Some(1_000.0), 10_999.0),
        "9 999 ms: held back"
    );
    assert!(step_warn_due(Some(1_000.0), 11_000.0), "10 000 ms: logged");
    assert!(step_warn_due(Some(1_000.0), 11_001.0));
    assert!(!step_warn_due(Some(1_000.0), 1_000.0), "the same instant");
    // The hub's own clock stepped back: never held back for longer than
    // the interval.
    assert!(step_warn_due(Some(20_000.0), 5_000.0));
}

#[test]
fn an_idle_record_names_the_meter_period_the_hub_keeps() {
    // #51: after the clamp; 0 paused; none awake.
    let every = |ms: u64| Pace::Every(Duration::from_millis(ms));
    assert_eq!(
        idle_fields(3, "10.0.0.5", true, every(250)),
        json!({"client": 3, "peer": "10.0.0.5", "on": true, "meter_ms": 250})
    );
    assert_eq!(
        idle_fields(3, "10.0.0.5", true, meter_pace(true, Some(5000))),
        json!({"client": 3, "peer": "10.0.0.5", "on": true, "meter_ms": 2000})
    );
    assert_eq!(
        idle_fields(3, "10.0.0.5", true, Pace::Paused),
        json!({"client": 3, "peer": "10.0.0.5", "on": true, "meter_ms": 0})
    );
    assert_eq!(
        idle_fields(4, "10.0.0.6", false, Pace::Full),
        json!({"client": 4, "peer": "10.0.0.6", "on": false, "meter_ms": null})
    );
}

#[test]
fn an_idle_line_says_how_the_pages_meters_go() {
    let log = logged(|| {
        log_idle(5, meter_pace(true, None));
        log_idle(5, meter_pace(true, Some(0)));
        log_idle(6, meter_pace(false, None));
        log_idle(7, meter_pace(true, Some(50)));
    });
    for line in [
        "a page went idle: its meters at most every 250 ms client=5",
        "a page went idle: its meters paused client=5",
        "a page woke client=6",
        "a page went idle: its meters at most every 100 ms client=7",
    ] {
        assert!(log.contains(&format!("ws: {line}\n")), "{log}");
    }
    assert_eq!(log.matches(" INFO ").count(), 4, "{log}");
}
