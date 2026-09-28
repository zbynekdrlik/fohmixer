//! Remote access through the real listeners (#17): the HTTPS listener of
//! the public name serves the stored certificate next to the plain-HTTP
//! one (the client WebSocket over TLS too), the plain-HTTP listener
//! redirects the name and stays the emergency path by IP, `/api/status`
//! reports it, and the Access check is live on the real listener (the peer
//! address comes from the listener: a tunnel request from 127.0.0.1 is
//! still refused without an Access assertion).

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fohmixer_hub::config::{Config, TlsCfg};
use futures_util::StreamExt;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::Value;
use support::TestHub;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

const NAME: &str = "foh.example.org";

/// A test CA (DER) and a leaf for NAME it signs (chain PEM, key PEM).
fn certificate() -> (CertificateDer<'static>, String, String) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec![NAME.to_string()])
        .unwrap()
        .signed_by(&key, &issuer)
        .unwrap();
    (ca.der().clone(), leaf.pem(), key.serialize_pem())
}

/// A hub with `[tls]` NAME on an ephemeral port, and a stored certificate
/// for it when `ca` is given.
async fn hub(dir: &std::path::Path, stored: Option<(&str, &str)>) -> TestHub {
    if let Some((chain, key)) = stored {
        std::fs::create_dir_all(dir.join("tls")).unwrap();
        std::fs::write(dir.join("tls").join("cert.pem"), chain).unwrap();
        std::fs::write(dir.join("tls").join("key.pem"), key).unwrap();
    }
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.tls = Some(TlsCfg {
        name: NAME.into(),
        port: 0,
        redirect_http: true,
    });
    TestHub::start_config(config).await
}

/// A TLS connection to 127.0.0.1:`port` for NAME, trusting `ca` only.
async fn tls(
    port: u16,
    ca: &CertificateDer<'static>,
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
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(NAME).unwrap(), tcp)
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
    let hub = hub(dir.path(), Some((&chain, &key))).await;
    let status = hub.status().await;
    let https = status.remote.https.clone().expect("the HTTPS listener");
    assert!(https.serving, "{status:?}");
    assert_ne!(https.port, 0);
    assert_ne!(https.port, hub.addr.port());
    assert_eq!(https.cert_names, vec![NAME.to_string()]);
    assert_eq!(status.remote.name.as_deref(), Some(NAME));
    assert!(!status.remote.access);

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
    let url = format!("wss://{NAME}:{}/ws?token={}&proto=1", https.port, hub.token);
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
    let hub = hub(dir.path(), Some((&chain, &key))).await;
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
async fn without_a_certificate_https_waits_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), None).await;
    let https = hub.status().await.remote.https.unwrap();
    assert!(!https.serving);
    assert!(!https.acme);
    let why = https.cert_error.unwrap();
    assert!(
        why.starts_with("no certificate for foh.example.org yet"),
        "{why}"
    );
    // Plain HTTP by IP serves meanwhile.
    let (code, _, _) = get(
        TcpStream::connect(hub.addr).await.unwrap(),
        &hub.addr.to_string(),
        "/api/version",
        &[],
    )
    .await;
    assert_eq!(code, 200);
    hub.stop().await;
}

#[tokio::test]
async fn a_taken_https_port_stops_the_start() {
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
    let (ready, _) = tokio::sync::oneshot::channel();
    let error = fohmixer_hub::serve_until(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        config,
        ready,
        std::future::pending(),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").starts_with("binding HTTPS 127.0.0.1:"),
        "{error:#}"
    );
    drop(taken);
}
