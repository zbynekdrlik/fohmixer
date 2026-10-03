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

/// What `f` logs, formatted as the hub's log formats it (without the time
/// and the colours).
fn logged(f: impl FnOnce()) -> String {
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
