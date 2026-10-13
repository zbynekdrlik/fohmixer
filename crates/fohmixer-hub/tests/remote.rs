//! Remote access through the real listeners (#17): the HTTPS listener of
//! the public name serves the stored certificate next to the plain-HTTP
//! one (the client WebSocket over TLS too), the plain-HTTP listener
//! redirects the name and stays the emergency path by IP, `/api/status`
//! reports it, and the Access check is live on the real listener (the peer
//! address comes from the listener: a tunnel request from 127.0.0.1 is
//! still refused without an Access assertion). AbleSet's name (#90) gets its
//! own certificate on the same listener, by SNI.

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fohmixer_hub::ableset::{CLOSE_BOUND, STOPPED};
use fohmixer_hub::config::{AbleSetCfg, Config, TlsCfg};
use fohmixer_proto::client::ServerMsg;
use futures_util::{SinkExt, StreamExt};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::Value;
use support::TestHub;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;

const NAME: &str = "foh.example.org";
const ABLESET: &str = "ableset.example.org";

/// A test CA (DER) and a leaf for `name` it signs (chain PEM, key PEM).
fn certificate_for(name: &str) -> (CertificateDer<'static>, String, String) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec![name.to_string()])
        .unwrap()
        .signed_by(&key, &issuer)
        .unwrap();
    (ca.der().clone(), leaf.pem(), key.serialize_pem())
}

/// A test CA (DER) and a leaf for NAME it signs (chain PEM, key PEM).
fn certificate() -> (CertificateDer<'static>, String, String) {
    certificate_for(NAME)
}

/// A hub with `[tls]` NAME on an ephemeral port, and the certificate
/// `stored` (chain, key) in its store when given; with `ableset`, also
/// `[ableset]` ABLESET and that certificate in its store (#90).
async fn hub(
    dir: &std::path::Path,
    stored: Option<(&str, &str)>,
    ableset: Option<(&str, &str)>,
) -> TestHub {
    let upstream = fohmixer_hub::config::DEFAULT_ABLESET_UPSTREAM;
    hub_to(dir, stored, ableset, upstream).await
}

/// [`hub`] with AbleSet at `upstream`.
async fn hub_to(
    dir: &std::path::Path,
    stored: Option<(&str, &str)>,
    ableset: Option<(&str, &str)>,
    upstream: &str,
) -> TestHub {
    let tls_dir = dir.join("tls");
    for (folder, pair) in [(tls_dir.join("ableset"), ableset), (tls_dir, stored)] {
        if let Some((chain, key)) = pair {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("cert.pem"), chain).unwrap();
            std::fs::write(folder.join("key.pem"), key).unwrap();
        }
    }
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.tls = Some(TlsCfg {
        name: NAME.into(),
        port: 0,
        redirect_http: true,
    });
    config.ableset = ableset.map(|_| AbleSetCfg {
        name: ABLESET.into(),
        upstream: upstream.into(),
        access_aud: Vec::new(),
        title: fohmixer_hub::config::DEFAULT_ABLESET_TITLE.into(),
    });
    TestHub::start_config(config).await
}

/// A TLS connection to 127.0.0.1:`port` for NAME, trusting `ca` only.
async fn tls(
    port: u16,
    ca: &CertificateDer<'static>,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    tls_for(port, ca, NAME).await
}

/// A TLS connection to 127.0.0.1:`port` for `name` (SNI, and the name the
/// certificate must cover), trusting `ca` only.
async fn tls_for(
    port: u16,
    ca: &CertificateDer<'static>,
    name: &'static str,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tls_over(tcp, ca, name).await
}

/// A TLS connection over `tcp` for `name`, trusting `ca` only.
async fn tls_over(
    tcp: TcpStream,
    ca: &CertificateDer<'static>,
    name: &'static str,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.clone()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(name).unwrap(), tcp)
        .await
        .expect("a TLS handshake for the name")
}

/// One HTTP/1.1 GET on `stream` with `headers`: the status, the headers
/// block and the JSON body (`null` when not JSON).
async fn get<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, String, Value) {
    let mut request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    // A TLS peer may end without close_notify after `Connection: close`.
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut bytes))
        .await
        .expect("an answer within 10 s");
    let text = String::from_utf8_lossy(&bytes).to_string();
    let code = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("not an HTTP answer: {text}"));
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (
        code,
        head.to_ascii_lowercase(),
        serde_json::from_str(body).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_public_name_is_served_over_https_next_to_plain_http() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, chain, key) = certificate();
    let hub = hub(dir.path(), Some((&chain, &key)), None).await;
    let status = hub.status().await;
    let https = status.remote.https.clone().expect("the HTTPS listener");
    assert!(https.serving, "{status:?}");
    assert_ne!(https.port, 0);
    assert_ne!(https.port, hub.addr.port());
    assert_eq!(https.cert_names, vec![NAME.to_string()]);
    assert_eq!(status.remote.name.as_deref(), Some(NAME));
    assert!(!status.remote.access);
    assert_eq!(status.ableset, None, "no [ableset]: nothing about AbleSet");

    // HTTPS: the version, with the security headers.
    let (code, head, body) = get(tls(https.port, &ca).await, NAME, "/api/version", &[]).await;
    assert_eq!(code, 200, "{head}");
    assert_eq!(body["version"], fohmixer_proto::VERSION);
    assert!(head.contains("x-frame-options: deny"), "{head}");
    assert!(
        !head.contains("strict-transport-security"),
        "no HSTS: {head}"
    );

    // The client WebSocket over TLS (wss://).
    let url = format!("wss://{NAME}:{}/ws?token={}&proto=2", https.port, hub.token);
    let (mut ws, _) = tokio_tungstenite::client_async(url, tls(https.port, &ca).await)
        .await
        .expect("a wss upgrade");
    let hello = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("a hello within 10 s")
        .unwrap()
        .unwrap();
    assert!(hello.to_text().unwrap().contains("\"hello\""), "{hello:?}");

    // Plain HTTP: the name is redirected to HTTPS, an IP address is served.
    let plain = || TcpStream::connect(hub.addr);
    let host = format!("{NAME}:{}", hub.addr.port());
    let (code, head, _) = get(plain().await.unwrap(), &host, "/api/version?x=1", &[]).await;
    assert_eq!(code, 307, "{head}");
    assert!(
        head.contains(&format!(
            "location: https://{NAME}:{}/api/version?x=1",
            https.port
        )),
        "{head}"
    );
    let (code, _, body) = get(
        plain().await.unwrap(),
        &hub.addr.to_string(),
        "/api/version",
        &[],
    )
    .await;
    assert_eq!(
        (code, body["version"].as_str()),
        (200, Some(fohmixer_proto::VERSION))
    );
    hub.stop().await;
}

#[tokio::test]
async fn the_access_check_is_live_on_the_real_listeners() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, chain, key) = certificate();
    let hub = hub(dir.path(), Some((&chain, &key)), None).await;
    let port = hub.status().await.remote.https.unwrap().port;
    let tunnel = [("cf-connecting-ip", "203.0.113.7")];
    // The tunnel's request comes from 127.0.0.1: the header makes it an
    // internet request, and without [access] it is refused.
    let host = hub.addr.to_string();
    let (code, _, body) = get(
        TcpStream::connect(hub.addr).await.unwrap(),
        &host,
        "/",
        &tunnel,
    )
    .await;
    assert_eq!((code, body["code"].as_str()), (403, Some("ACCESS_DENIED")));
    let (code, _, _) = get(tls(port, &ca).await, NAME, "/api/version", &tunnel).await;
    assert_eq!(code, 403);
    // Without the header it is the LAN.
    let (code, _, _) = get(tls(port, &ca).await, NAME, "/api/version", &[]).await;
    assert_eq!(code, 200);
    hub.stop().await;
}

#[tokio::test]
async fn ablesets_name_gets_its_own_certificate_on_the_same_listener() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, chain, key) = certificate();
    let (ableset_ca, ableset_chain, ableset_key) = certificate_for(ABLESET);
    let ableset = Some((ableset_chain.as_str(), ableset_key.as_str()));
    let hub = hub(dir.path(), Some((&chain, &key)), ableset).await;
    let status = hub.status().await;
    let https = status.remote.https.clone().unwrap();
    assert_eq!(https.cert_names, vec![NAME.to_string()]);
    let ableset = status.ableset.clone().expect("AbleSet's status");
    assert_eq!(ableset.name, ABLESET);
    assert!(ableset.cert.serving, "{status:?}");
    assert_eq!(ableset.cert.cert_names, vec![ABLESET.to_string()]);
    // Each handshake trusts only its own name's CA, so it passes only with
    // that name's certificate: AbleSet's by its SNI, and the [tls] name's
    // still serves fohmixer.
    drop(tls_for(https.port, &ableset_ca, ABLESET).await);
    let (code, _, body) = get(tls(https.port, &ca).await, NAME, "/api/version", &[]).await;
    assert_eq!(
        (code, body["version"].as_str()),
        (200, Some(fohmixer_proto::VERSION))
    );
    hub.stop().await;
}

/// A fake AbleSet that serves its index: its URL.
async fn indexed_ableset() -> String {
    let index = axum::routing::get(|| async { "<!doctype html><title>AbleSet</title>" });
    let app = axum::Router::new().route("/", index);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn the_served_hub_reads_ablesets_link_and_tells_every_page() {
    // #90 B3: the first read of AbleSet's index is at once; its change goes
    // to /api/status, the event log and every page.
    let dir = tempfile::tempdir().unwrap();
    let (_, chain, key) = certificate_for(ABLESET);
    let upstream = indexed_ableset().await;
    let ableset = Some((chain.as_str(), key.as_str()));
    let hub = hub_to(dir.path(), None, ableset, &upstream).await;
    let up = |s: &fohmixer_proto::client::HubStatus| {
        s.ableset.as_ref().is_some_and(|a| a.online == Some(true))
    };
    let status = hub.status_until(Duration::from_secs(5), up).await;
    let ableset = status.ableset.unwrap();
    assert_eq!(
        (ableset.upstream.as_str(), ableset.last_error),
        (upstream.as_str(), None)
    );
    assert!(ableset.checked.is_some());
    let mut page = hub.client().await;
    let port: u16 = upstream.rsplit_once(':').unwrap().1.parse().unwrap();
    let told = page
        .wait(Duration::from_secs(5), |msg| {
            matches!(
                msg,
                ServerMsg::Ableset {
                    online: Some(true),
                    ..
                }
            )
            .then(|| msg.clone())
        })
        .await;
    let wanted = ServerMsg::Ableset {
        title: "AbleSet".into(),
        name: ABLESET.into(),
        parent: NAME.into(),
        online: Some(true),
        lan_port: port,
    };
    assert_eq!(told, wanted);
    let linked = |r: &[Value]| r.iter().any(|r| r["ev"] == "ableset_link");
    let records = hub.events_until(Duration::from_secs(5), linked).await;
    let link = records.iter().find(|r| r["ev"] == "ableset_link").unwrap();
    assert_eq!(
        (&link["up"], &link["failures"]),
        (&Value::Bool(true), &Value::from(0))
    );
    page.close().await;
    hub.stop().await;
}

#[tokio::test]
async fn a_served_hub_without_ableset_tells_no_page_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), None, None).await;
    let mut page = hub.client().await;
    // The attach's messages come at once.
    let mut told = Vec::new();
    while let Some(msg) = page.next_message(Duration::from_millis(500)).await {
        told.push(msg);
    }
    let ableset = told.iter().any(|m| matches!(m, ServerMsg::Ableset { .. }));
    assert!(!ableset, "{told:?}");
    assert_eq!(hub.status().await.ableset, None);
    page.close().await;
    hub.stop().await;
}

#[tokio::test]
async fn without_a_certificate_https_waits_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), None, None).await;
    let https = hub.status().await.remote.https.unwrap();
    assert!(!https.serving);
    assert!(!https.acme);
    let why = https.cert_error.unwrap();
    assert!(
        why.starts_with("no certificate for foh.example.org yet"),
        "{why}"
    );
    // Plain HTTP serves meanwhile, by IP and by the name (no redirect to an
    // HTTPS listener that cannot answer yet).
    for host in [hub.addr.to_string(), format!("{NAME}:{}", hub.addr.port())] {
        let (code, head, _) = get(
            TcpStream::connect(hub.addr).await.unwrap(),
            &host,
            "/api/version",
            &[],
        )
        .await;
        assert_eq!(code, 200, "{host}: {head}");
    }
    hub.stop().await;
}

#[tokio::test]
async fn a_taken_https_port_leaves_plain_http_serving_and_says_why() {
    // The emergency path never depends on :443: another program on the
    // HTTPS port costs HTTPS only, and the status names the reason.
    let dir = tempfile::tempdir().unwrap();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut config = Config::defaults(dir.path());
    config.instances.clear();
    config.tls = Some(TlsCfg {
        name: NAME.into(),
        port,
        redirect_http: true,
    });
    let hub = TestHub::start_config(config).await;
    let https = hub.status().await.remote.https.unwrap();
    assert!(!https.bound && !https.serving, "{https:?}");
    assert_eq!(https.port, port);
    let why = https.bind_error.unwrap();
    assert!(
        why.starts_with(&format!("binding HTTPS 127.0.0.1:{port}: ")),
        "{why}"
    );
    let (code, _, body) = get(
        TcpStream::connect(hub.addr).await.unwrap(),
        &hub.addr.to_string(),
        "/api/version",
        &[],
    )
    .await;
    assert_eq!(
        (code, body["version"].as_str()),
        (200, Some(fohmixer_proto::VERSION))
    );
    hub.stop().await;
    drop(taken);
}

/// A fake AbleSet for the stop's test: on 127.0.0.1, it sends every
/// WebSocket it accepts 64 KiB binary messages until the connection takes
/// no more; its URL and the bytes it got out so far.
async fn pushing_ableset() -> (String, Arc<AtomicU64>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let pushed = Arc::new(AtomicU64::new(0));
    let count = Arc::clone(&pushed);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let count = Arc::clone(&count);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let block = vec![7_u8; 64 * 1024];
                while ws.send(Message::binary(block.clone())).await.is_ok() {
                    count.fetch_add(block.len() as u64, Ordering::Relaxed);
                }
            });
        }
    });
    (format!("http://{addr}"), pushed)
}

#[tokio::test]
async fn the_stop_waits_for_an_ableset_tunnel_to_close_and_record_it() {
    // #90 review 3: a tunnel's connection left the listener at its upgrade,
    // so neither drain waits for it; serve_until must, or the binary's
    // runtime shutdown cuts the close (AbleSet's end never gets its FIN)
    // and its record.
    let dir = tempfile::tempdir().unwrap();
    let (_, chain, key) = certificate();
    let (ableset_ca, ableset_chain, ableset_key) = certificate_for(ABLESET);
    let (upstream, pushed) = pushing_ableset().await;
    let ableset = Some((ableset_chain.as_str(), ableset_key.as_str()));
    let hub = hub_to(dir.path(), Some((&chain, &key)), ableset, &upstream).await;
    let port = hub.status().await.remote.https.unwrap().port;
    // A page that never reads, on a 4 KiB receive buffer.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let tcp = socket
        .connect(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .unwrap();
    let tls = tls_over(tcp, &ableset_ca, ABLESET).await;
    let url = format!("wss://{ABLESET}:{port}/socket.io/?EIO=4");
    let (page, _) = tokio_tungstenite::client_async(url, tls)
        .await
        .expect("a wss tunnel to AbleSet");
    // AbleSet fills the way to the page until its pushes stall: every
    // buffer on the way is full, so the stop's close_notify to the page
    // cannot go out and the tunnel's close takes its whole CLOSE_BOUND.
    let (mut last, mut still) = (0, 0);
    for _ in 0..400 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let now = pushed.load(Ordering::Relaxed);
        if now > 0 && now == last {
            still += 1;
            if still == 12 {
                break;
            }
        } else {
            (last, still) = (now, 0);
        }
    }
    assert_eq!(still, 12, "AbleSet's pushes never stalled: {last} bytes");
    let stopping = Instant::now();
    hub.stop().await;
    let waited = stopping.elapsed();
    assert!(
        waited >= CLOSE_BOUND,
        "serve_until returned before the tunnel closed: {waited:?}"
    );
    // Its close was recorded before that (a tunnel gives its slot back only
    // after its record), with the bytes it carried.
    let deadline = Instant::now() + Duration::from_secs(5);
    let close = loop {
        let records = support::events_in(dir.path());
        let close = records
            .into_iter()
            .find(|r| r["ev"] == "ableset_ws" && r["what"] == "close");
        if let Some(close) = close {
            break close;
        }
        assert!(Instant::now() < deadline, "no close record");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(close["why"], STOPPED);
    assert!(
        close["down"].as_u64().is_some_and(|down| down > 0),
        "{close}"
    );
    assert!(close["up"].is_u64(), "{close}");
    drop(page);
}
