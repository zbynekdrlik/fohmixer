//! Tests of AbleSet's name behind the hub (#90): the pure header, framing
//! and failure decisions, and the routers (the dispatch by `Host` on both
//! listeners, AbleSet's own gate, the proxy) against a fake AbleSet.

use std::net::SocketAddr;
use std::sync::mpsc::Receiver;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderName, Uri};
use axum::routing::{any, get, post};
use std::time::Duration;

use super::*;
use crate::config::DEFAULT_ABLESET_UPSTREAM;
use crate::test_keys::{KeyServer, claims, jwks, key, sign};
use crate::ws::tests::logged;

const NAME: &str = "ableset.example.org";
const TLS_NAME: &str = "foh.example.org";
const ABLESET_AUD: &str = "aud-ableset-test";
/// Where nothing listens: a connection is refused at once.
const CLOSED: &str = "http://127.0.0.1:1";
const OURS: &str = "frame-ancestors 'self' https://foh.example.org https://foh.example.org:8443";

fn tls() -> TlsCfg {
    TlsCfg {
        name: TLS_NAME.into(),
        port: 8443,
        redirect_http: true,
    }
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn policies(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

async fn text_of(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn the_upstream_request_keeps_the_browsers_headers_only() {
    let mut map = headers(&[
        ("host", "ableset.example.org:8443"),
        // A client cannot name its Host, Origin or body length away.
        (
            "connection",
            "keep-alive, X-Hop, Host, origin, Content-Length",
        ),
        ("x-hop", "1"),
        ("keep-alive", "timeout=5"),
        ("proxy-connection", "keep-alive"),
        ("te", "trailers"),
        ("trailer", "x-sum"),
        ("transfer-encoding", "chunked"),
        ("upgrade", "websocket"),
        ("proxy-authenticate", "Basic"),
        ("proxy-authorization", "Basic x"),
        ("cf-connecting-ip", "203.0.113.7"),
        ("cf-ray", "r"),
        ("cf-visitor", "{}"),
        ("cf-ipcountry", "SK"),
        ("cf-access-jwt-assertion", "t"),
        ("cf-access-authenticated-user-email", "e@example.org"),
        ("x-forwarded-for", "203.0.113.7"),
        ("x-forwarded-host", "ableset.example.org"),
        ("x-forwarded-proto", "https"),
        ("forwarded", "for=203.0.113.7"),
        ("x-real-ip", "203.0.113.7"),
        ("true-client-ip", "203.0.113.7"),
        ("cdn-loop", "cloudflare"),
        ("via", "1.1 proxy"),
        ("x-forwarded-port", "443"),
        ("x-forwarded-server", "proxy"),
        ("cookie", "CF_Authorization=t; theme=dark"),
        ("cookie", "browserId=b"),
        ("origin", "https://ableset.example.org:8443"),
        ("user-agent", "ua"),
        ("accept", "*/*"),
        ("content-type", "application/json"),
        ("content-length", "2"),
        ("authorization", "Bearer ableset"),
    ]);
    strip_request(&mut map);
    let mut names: Vec<&str> = map.keys().map(HeaderName::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "accept",
            "authorization",
            "content-length",
            "content-type",
            "cookie",
            "host",
            "origin",
            "user-agent"
        ]
    );
    assert_eq!(map["host"], "ableset.example.org:8443");
    assert_eq!(map["cookie"], "theme=dark; browserId=b");
}

#[test]
fn only_the_access_cookie_is_taken_out() {
    assert_eq!(
        without_access_cookie(["CF_Authorization=t"].into_iter()),
        None
    );
    assert_eq!(
        without_access_cookie(["a=1;; CF_Authorization=t ; b=2"].into_iter()).as_deref(),
        Some("a=1; b=2")
    );
    assert_eq!(
        without_access_cookie(["CF_Authorization_x=1"].into_iter()).as_deref(),
        Some("CF_Authorization_x=1")
    );
    assert_eq!(without_access_cookie(std::iter::empty()), None);
}

#[test]
fn ablesets_answers_may_be_framed_by_fohmixers_page_only() {
    assert_eq!(
        strip_frame_ancestors("default-src 'self'; frame-ancestors 'self'").as_deref(),
        Some("default-src 'self'")
    );
    assert_eq!(
        strip_frame_ancestors("Frame-Ancestors 'none'; ; script-src 'self' ;frame-ancestors x")
            .as_deref(),
        Some("script-src 'self'")
    );
    assert_eq!(strip_frame_ancestors("frame-ancestors 'none'"), None);
    let mut map = headers(&[
        ("x-frame-options", "SAMEORIGIN"),
        (
            "content-security-policy",
            "default-src 'self'; frame-ancestors 'self'",
        ),
        ("content-security-policy", "frame-ancestors 'none'"),
        ("location", "http://ableset.example.org:8443/next?a=b"),
        ("content-type", "text/html"),
    ]);
    frame_policy(&mut map, &HeaderValue::from_static(OURS), NAME);
    assert!(map.get(header::X_FRAME_OPTIONS).is_none());
    assert_eq!(policies(&map), ["default-src 'self'", OURS]);
    assert_eq!(map["location"], "https://ableset.example.org:8443/next?a=b");
    assert_eq!(map["content-type"], "text/html");
    assert_eq!(
        https_origins(TLS_NAME, 443),
        "https://foh.example.org",
        "the tunnel's port is the default one"
    );
    assert_eq!(
        https_origins(TLS_NAME, 8443),
        "https://foh.example.org https://foh.example.org:8443"
    );
}

#[test]
fn only_an_http_location_of_ablesets_name_becomes_https() {
    for (location, https) in [
        (
            "http://ableset.example.org",
            Some("https://ableset.example.org"),
        ),
        (
            "http://AbleSet.example.org/p?q#f",
            Some("https://AbleSet.example.org/p?q#f"),
        ),
        (
            "http://ableset.example.org:8443?x",
            Some("https://ableset.example.org:8443?x"),
        ),
        ("https://ableset.example.org/", None),
        ("/relative", None),
        ("http://foh.example.org/", None),
        ("http://ableset.example.org.evil.example/", None),
        ("http://ableset.example.org:x/", None),
    ] {
        assert_eq!(
            https_location(location, NAME).as_deref(),
            https,
            "{location}"
        );
    }
}

#[test]
fn a_host_of_ablesets_name_goes_to_ableset_and_nothing_else_does() {
    for host in ["ableset.example.org", "AbleSet.Example.ORG:8443"] {
        assert!(for_ableset(Some(host), NAME), "{host}");
    }
    for host in [
        TLS_NAME,
        "ableset.example.org.evil.example",
        "ableset.example.org:x",
        "ableset.example.org:8443 x",
        "10.0.0.5",
        "",
    ] {
        assert!(!for_ableset(Some(host), NAME), "{host}");
    }
    assert!(!for_ableset(None, NAME));
}

#[test]
fn ablesets_gate_is_fohmixers_team_with_ablesets_audience() {
    let mut config = Config::defaults(std::path::Path::new("/data"));
    assert_eq!(access_cfg(&config), None);
    let access = AccessCfg {
        team_domain: "team.example.com".into(),
        aud: vec!["aud-foh".into()],
        jwks_url: Some("http://127.0.0.1:9/certs".into()),
    };
    config.access = Some(access.clone());
    config.ableset = Some(AbleSetCfg {
        name: NAME.into(),
        upstream: DEFAULT_ABLESET_UPSTREAM.into(),
        access_aud: Vec::new(),
    });
    assert_eq!(access_cfg(&config), None, "no audience: internet refused");
    config.ableset.as_mut().unwrap().access_aud = vec![ABLESET_AUD.into()];
    let gate = access_cfg(&config).unwrap();
    assert_eq!(gate.aud, vec![ABLESET_AUD.to_string()]);
    assert_eq!(
        (gate.team_domain, gate.jwks_url),
        (access.team_domain, access.jwks_url)
    );
    config.access = None;
    assert_eq!(access_cfg(&config), None);
}

#[test]
fn a_body_is_refused_by_its_announced_size_only_over_the_cap() {
    let of = |length: &str| too_large(&headers(&[("content-length", length)]), 16);
    assert_eq!((of("16"), of("17"), of("x")), (None, Some(17), None));
    assert_eq!(too_large(&HeaderMap::new(), 16), None);
    assert_eq!(proxy::MAX_BODY, 64 * 1024 * 1024);
}

#[tokio::test]
async fn a_navigation_gets_a_page_that_tries_again_anything_else_json() {
    for dest in ["document", "iframe"] {
        let page = down_answer(Some(dest), false);
        assert_eq!(page.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
        let text = text_of(page).await;
        assert!(
            text.contains("AbleSet na PC neodpovedá. Skúšam znova o 5 s."),
            "{text}"
        );
        assert!(
            text.contains("http-equiv=\"refresh\" content=\"5\""),
            "{text}"
        );
    }
    assert_eq!(
        down_answer(Some("iframe"), true).status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    for (dest, timed_out, status, code) in [
        (None, false, 502, "ABLESET_DOWN"),
        (Some("empty"), false, 502, "ABLESET_DOWN"),
        (Some("script"), true, 504, "ABLESET_TIMEOUT"),
    ] {
        let answer = down_answer(dest, timed_out);
        assert_eq!(answer.status().as_u16(), status);
        let body: Value = serde_json::from_str(&text_of(answer).await).unwrap();
        assert_eq!(body["code"], code);
    }
}

/// AbleSet behind a proxy with these bounds, no gate, its records.
fn site_with(upstream: &str, head: Duration, cap: u64) -> (Arc<AbleSet>, Receiver<Value>) {
    let (events, records) = EventLog::channel(64);
    let proxy = Proxy::new(upstream, head, cap);
    let site = AbleSet::with_proxy(NAME, &tls(), None, events, proxy);
    (Arc::new(site), records)
}

#[tokio::test]
async fn warn_lines_are_rationed_and_only_ablesets_own_fault_is_a_warn_class_record() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let down = || Failure::Down("AbleSet does not answer: refused".into());
    let log = logged(|| {
        site.failed(&Method::GET, "/a", None, down());
        assert!(site.failing.load(Ordering::Relaxed), "an outage started");
        site.failed(&Method::GET, "/b", None, down());
        site.failed(&Method::POST, "/c", None, Failure::TooLarge("big".into()));
        site.failed(&Method::GET, "/d", None, Failure::Timeout("late".into()));
        site.reached(&Method::GET, "/e", StatusCode::OK, Instant::now());
        assert!(!site.failing.load(Ordering::Relaxed), "the outage ended");
        site.failed(&Method::GET, "/f", None, down());
    });
    // One warn line per kind within its gap: AbleSet's outage (/a; the
    // late answer and the next outage come too soon after it), and a
    // client's fault (/c), which can never hide AbleSet's.
    let warned: Vec<&str> = log.lines().filter(|l| l.contains(" WARN ")).collect();
    assert_eq!(warned.len(), 2, "{log}");
    for (line, path) in warned.iter().zip(["/a", "/c"]) {
        assert!(line.contains(&format!("path=\"{path}\"")), "{line}");
    }
    let records: Vec<Value> = records.try_iter().collect();
    let first = &records[0];
    assert_eq!(
        (&first["ev"], &first["method"]),
        (&json!("ableset_proxy"), &json!("GET"))
    );
    // Every one is recorded; only the warned fault of AbleSet itself under
    // `error` (warn class, kept past the event log's daily cap), the rest
    // under `why`: no client fills the PC's disk.
    let keys: Vec<&str> = records
        .iter()
        .map(|r| {
            if r.get("error").is_some() {
                "error"
            } else {
                "why"
            }
        })
        .collect();
    assert_eq!(keys, ["error", "why", "why", "why", "why"]);
    let rows: Vec<(&str, u64, &str)> = records
        .iter()
        .map(|r| {
            let why = r.get("error").or_else(|| r.get("why")).unwrap();
            let path = r["path"].as_str().unwrap();
            (path, r["status"].as_u64().unwrap(), why.as_str().unwrap())
        })
        .collect();
    let refused = "AbleSet does not answer: refused";
    assert_eq!(
        rows,
        [
            ("/a", 502, refused),
            ("/b", 502, refused),
            ("/c", 413, "big"),
            ("/d", 504, "late"),
            ("/f", 502, refused)
        ]
    );
}

/// A fake AbleSet on 127.0.0.1: `/echo` answers the headers it got as JSON
/// (a header's values `, `-joined), `/framed` with AbleSet-like framing
/// headers, `/moved` an http redirect of AbleSet's name, `/cookies`,
/// `/cors` and `/hop` with such headers, `/slow` never, `POST /press` the
/// length of its body, anything else `fake <path and query>`.
async fn fake() -> String {
    let echo = |seen: HeaderMap| async move {
        let seen: serde_json::Map<String, Value> = seen
            .keys()
            .map(|k| {
                let values: Vec<&str> = seen
                    .get_all(k)
                    .iter()
                    .map(|v| v.to_str().unwrap_or(""))
                    .collect();
                (k.as_str().to_string(), json!(values.join(", ")))
            })
            .collect();
        Json(Value::Object(seen))
    };
    let framed = || async {
        let framing = [
            ("x-frame-options", "DENY"),
            ("content-security-policy", "frame-ancestors 'self'"),
        ];
        (framing, "framed")
    };
    let moved = || async {
        let to = [("location", "http://ableset.example.org/next")];
        (StatusCode::FOUND, to)
    };
    let cookies = || async {
        let set = [
            (
                "set-cookie",
                "CF_Authorization=evil; Domain=example.org; Path=/",
            ),
            ("set-cookie", "cf_clearance=c; Path=/"),
            (
                "set-cookie",
                "session=s; Domain=example.org; Path=/; HttpOnly",
            ),
            ("set-cookie", "theme=dark"),
        ];
        (axum::response::AppendHeaders(set), "cookies")
    };
    let cors = || async {
        let grant = [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-credentials", "true"),
            ("access-control-expose-headers", "x-secret"),
        ];
        (grant, "cors")
    };
    let hop = || async {
        let hop = [
            ("connection", "x-gone"),
            ("x-gone", "1"),
            ("keep-alive", "timeout=5"),
            ("proxy-authenticate", "Basic"),
        ];
        (hop, "hop")
    };
    let app = Router::new()
        .route("/echo", any(echo))
        .route("/framed", get(framed))
        .route("/moved", get(moved))
        .route("/cookies", get(cookies))
        .route("/cors", get(cors))
        .route("/hop", get(hop))
        .route("/slow", get(std::future::pending::<()>))
        .route(
            "/press",
            post(|body: Bytes| async move { body.len().to_string() }),
        )
        .fallback(|uri: Uri| async move { format!("fake {uri}") });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A hub with `[tls]`, `[ableset]` at `upstream` with the audiences `aud`,
/// and `[access]` from `keys`.
fn hub_with(dir: &std::path::Path, upstream: &str, keys: Option<&KeyServer>, aud: &[&str]) -> Hub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.tls = Some(tls());
    config.access = keys.map(KeyServer::config);
    config.ableset = Some(AbleSetCfg {
        name: NAME.into(),
        upstream: upstream.into(),
        access_aud: aud.iter().map(|a| a.to_string()).collect(),
    });
    Hub::start(config).unwrap()
}

/// A request for `host` from the peer `from` (the listener's `ConnectInfo`).
fn request(
    method: Method,
    host: &str,
    path: &str,
    from: &str,
    pairs: &[(&str, &str)],
    body: Body,
) -> axum::http::Request<Body> {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .body(body)
        .unwrap();
    request.headers_mut().extend(headers(pairs));
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(from.parse().unwrap(), 50000)));
    request
}

fn get_from(
    host: &str,
    path: &str,
    from: &str,
    pairs: &[(&str, &str)],
) -> axum::http::Request<Body> {
    request(Method::GET, host, path, from, pairs, Body::empty())
}

/// Through the plain-HTTP listener's application.
async fn plain(hub: &Hub, request: axum::http::Request<Body>) -> Response {
    crate::app_router(hub.clone())
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn ablesets_name_reaches_ableset_on_both_listeners_and_any_other_host_fohmixer() {
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    for host in [NAME, "AbleSet.Example.org:8480"] {
        let response = plain(&hub, get_from(host, "/api/version?x=1", "10.0.0.5", &[])).await;
        assert_eq!(response.status(), StatusCode::OK, "{host}");
        assert!(response.headers().get(header::X_FRAME_OPTIONS).is_none());
        assert_eq!(policies(response.headers()), [OURS]);
        assert_eq!(text_of(response).await, "fake /api/version?x=1", "{host}");
    }
    let https = crate::https_router(hub.clone())
        .oneshot(get_from(NAME, "/p", "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(text_of(https).await, "fake /p");
    // fohmixer's own name and an address: fohmixer, its headers, a policy
    // that lets its page frame AbleSet.
    for host in [TLS_NAME, "10.0.0.5:8480"] {
        let response = plain(&hub, get_from(host, "/api/version", "10.0.0.5", &[])).await;
        assert_eq!(
            response.headers()[header::X_FRAME_OPTIONS],
            "DENY",
            "{host}"
        );
        let csp = policies(response.headers()).remove(0);
        assert!(
            csp.ends_with(
                "; frame-src https://ableset.example.org https://ableset.example.org:8443"
            ),
            "{csp}"
        );
        let body: Value = serde_json::from_str(&text_of(response).await).unwrap();
        assert_eq!(body["version"], fohmixer_proto::VERSION, "{host}");
    }
    // A Host the one parser refuses is no AbleSet request: fohmixer's 421.
    let refused = plain(
        &hub,
        get_from("ableset.example.org:x", "/", "10.0.0.5", &[]),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::MISDIRECTED_REQUEST);
    // AbleSet's own framing gives way to fohmixer's page; its other
    // headers stay, the Access cookie does not reach it (Cloudflare's
    // headers: the internet test, a LAN request carries none).
    let framed = plain(&hub, get_from(NAME, "/framed", "10.0.0.5", &[])).await;
    assert!(framed.headers().get(header::X_FRAME_OPTIONS).is_none());
    assert_eq!(policies(framed.headers()), [OURS]);
    let pairs = [
        ("cookie", "CF_Authorization=t; theme=dark"),
        ("user-agent", "ua"),
    ];
    let seen = echo(plain(&hub, get_from(NAME, "/echo", "10.0.0.5", &pairs)).await).await;
    assert_eq!(
        (&seen["host"], &seen["cookie"], &seen["user-agent"]),
        (&json!(NAME), &json!("theme=dark"), &json!("ua"))
    );
    hub.stop();
}

/// What the fake's `/echo` saw.
async fn echo(response: Response) -> Value {
    serde_json::from_str(&text_of(response).await).unwrap()
}

/// A request for `target` (`method`, no `Host` when `host` is none) from
/// a LAN peer.
fn odd(method: Method, target: &str, host: Option<&str>) -> axum::http::Request<Body> {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(target)
        .body(Body::empty())
        .unwrap();
    if let Some(host) = host {
        request
            .headers_mut()
            .insert(header::HOST, HeaderValue::from_str(host).unwrap());
    }
    let lan = SocketAddr::new("10.0.0.5".parse().unwrap(), 50000);
    request.extensions_mut().insert(ConnectInfo(lan));
    request
}

#[tokio::test]
async fn ableset_gets_one_host_the_one_matched_and_only_a_path_never_an_outage() {
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    // A second Host, or a Connection naming Host or Origin away, changes
    // nothing: AbleSet gets exactly the Host the hub matched (#90 review 7).
    let mut twice = get_from(NAME, "/echo", "10.0.0.5", &[]);
    let evil = HeaderValue::from_static("evil.example");
    twice.headers_mut().append(header::HOST, evil);
    assert_eq!(echo(plain(&hub, twice).await).await["host"], NAME);
    for connection in ["host", "Host, origin"] {
        let pairs = [("connection", connection), ("origin", "https://x.example")];
        let seen = echo(plain(&hub, get_from(NAME, "/echo", "10.0.0.5", &pairs)).await).await;
        assert_eq!(
            (&seen["host"], &seen["origin"]),
            (&json!(NAME), &json!("https://x.example")),
            "{connection}"
        );
    }
    // Without a Host (HTTP/2 sends none) the target's authority is the
    // Host: AbleSet's name reaches AbleSet, a foreign one gets 421 (#90
    // review 9); a Host with a trailing dot is neither name: 421.
    let no_host = odd(Method::GET, "http://ableset.example.org/echo", None);
    assert_eq!(echo(plain(&hub, no_host).await).await["host"], NAME);
    let foreign = odd(Method::GET, "http://evil.example/api/version", None);
    let dotted = get_from("ableset.example.org.", "/", "10.0.0.5", &[]);
    for refused in [foreign, dotted] {
        let answer = plain(&hub, refused).await;
        assert_eq!(answer.status(), StatusCode::MISDIRECTED_REQUEST);
    }
    // CONNECT and TRACE are never forwarded, nor a target that is no path;
    // none of them makes an AbleSet outage (#90 review 5).
    for (method, target, status) in [
        (Method::CONNECT, "ableset.example.org:443", 405),
        (Method::TRACE, "/", 405),
        (Method::OPTIONS, "*", 400),
    ] {
        let answer = plain(&hub, odd(method.clone(), target, Some(NAME))).await;
        assert_eq!(answer.status().as_u16(), status, "{method} {target}");
        assert_eq!(policies(answer.headers()), [OURS]);
    }
    let site = hub.ableset.as_ref().unwrap();
    assert!(!site.failing.load(Ordering::Relaxed), "no outage");
    // The plain listener's LAN answer keeps AbleSet's http Location (https
    // would point TLS at the plain port, #90 review 8); over HTTPS it is
    // made https.
    let moved = plain(&hub, get_from(NAME, "/moved", "10.0.0.5", &[])).await;
    let location = "http://ableset.example.org/next";
    assert_eq!(moved.headers()[header::LOCATION], location);
    let moved = crate::https_router(hub.clone())
        .oneshot(get_from(NAME, "/moved", "10.0.0.5", &[]))
        .await
        .unwrap();
    let location = "https://ableset.example.org/next";
    assert_eq!(moved.headers()[header::LOCATION], location);
    hub.stop();
}

#[tokio::test]
async fn ablesets_answers_serve_its_own_pages_only() {
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    let fetch = |site: &str, mode: &str, dest: &str| {
        let pairs = [
            ("sec-fetch-site", site),
            ("sec-fetch-mode", mode),
            ("sec-fetch-dest", dest),
        ];
        get_from(NAME, "/p", "10.0.0.5", &pairs)
    };
    // Another site's image, script or fetch in a LAN browser (which opens
    // AbleSet without its password) is refused before AbleSet (#90 review
    // 2); a navigation from anywhere and the same site's requests pass.
    for (mode, dest) in [
        ("no-cors", "image"),
        ("no-cors", "script"),
        ("cors", "empty"),
    ] {
        let refused = plain(&hub, fetch("cross-site", mode, dest)).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{mode} {dest}");
        assert_eq!(policies(refused.headers()), [OURS]);
        assert_eq!(refused.headers()["x-content-type-options"], "nosniff");
    }
    for (site, mode, dest) in [
        ("cross-site", "navigate", "document"),
        ("none", "navigate", "document"),
        ("same-origin", "cors", "empty"),
        ("same-site", "no-cors", "image"),
    ] {
        let passed = plain(&hub, fetch(site, mode, dest)).await;
        assert_eq!(text_of(passed).await, "fake /p", "{site} {mode} {dest}");
    }
    // No CORS grant of AbleSet's leaves the hub; nosniff and fohmixer's
    // Referrer-Policy on its answers (#90 review 10).
    let cors = plain(&hub, get_from(NAME, "/cors", "10.0.0.5", &[])).await;
    let names: Vec<&str> = cors.headers().keys().map(HeaderName::as_str).collect();
    assert!(
        names.iter().all(|n| !n.starts_with("access-control-")),
        "{names:?}"
    );
    assert_eq!(cors.headers()["x-content-type-options"], "nosniff");
    let referrer = "strict-origin-when-cross-origin";
    assert_eq!(cors.headers()["referrer-policy"], referrer);
    // No Cloudflare cookie from AbleSet (it would shadow fohmixer's Access
    // cookie), and the others host-only (#90 review 6).
    let cookies = plain(&hub, get_from(NAME, "/cookies", "10.0.0.5", &[])).await;
    let set: Vec<&str> = cookies
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(set, ["session=s; Path=/; HttpOnly", "theme=dark"]);
    // AbleSet's hop-by-hop headers end at the hop.
    let hop = plain(&hub, get_from(NAME, "/hop", "10.0.0.5", &[])).await;
    for gone in ["connection", "x-gone", "keep-alive", "proxy-authenticate"] {
        assert!(hop.headers().get(gone).is_none(), "{gone}");
    }
    assert_eq!(text_of(hop).await, "hop");
    hub.stop();
}

#[tokio::test]
async fn from_the_internet_only_the_ableset_applications_assertion_opens_ableset() {
    let upstream = fake().await;
    let keys = KeyServer::start(jwks(&[key()])).await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, Some(&keys), &[ABLESET_AUD]);
    let mut ablesets = claims();
    ablesets["aud"] = json!([ABLESET_AUD]);
    let ablesets = sign(key(), &ablesets);
    let fohmixers = sign(key(), &claims());
    let tunnel = |host: &str, path: &str, token: Option<&str>| {
        let mut pairs = vec![
            ("cf-connecting-ip", "203.0.113.7"),
            ("x-forwarded-for", "203.0.113.7"),
            ("forwarded", "for=203.0.113.7"),
            ("cookie", "theme=dark"),
        ];
        pairs.extend(token.map(|t| ("cf-access-jwt-assertion", t)));
        get_from(host, path, "127.0.0.1", &pairs)
    };
    for token in [None, Some(fohmixers.as_str())] {
        let refused = plain(&hub, tunnel(NAME, "/echo", token)).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{token:?}");
        assert_eq!(policies(refused.headers()), [OURS]);
    }
    let allowed = plain(&hub, tunnel(NAME, "/echo", Some(ablesets.as_str()))).await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let seen: Value = serde_json::from_str(&text_of(allowed).await).unwrap();
    for gone in [
        "cf-connecting-ip",
        "x-forwarded-for",
        "forwarded",
        "cf-access-jwt-assertion",
    ] {
        assert!(seen.get(gone).is_none(), "{gone}: {seen}");
    }
    assert_eq!(seen["cookie"], "theme=dark");
    // The assertion in Access's cookie: fohmixer's refused, AbleSet's let
    // through, the cookie kept from AbleSet.
    let in_cookie = |token: &str| {
        let cookie = format!("CF_Authorization={token}; theme=dark");
        let pairs = [
            ("cf-connecting-ip", "203.0.113.7"),
            ("cookie", cookie.as_str()),
        ];
        get_from(NAME, "/echo", "127.0.0.1", &pairs)
    };
    let refused = plain(&hub, in_cookie(&fohmixers)).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let seen = echo(plain(&hub, in_cookie(&ablesets)).await).await;
    assert_eq!(seen["cookie"], "theme=dark");
    // A Cloudflare header alone is the internet: a tunnel request a rule
    // left without its forwarding headers still has Cf-Ray (#90 review 1).
    // So is a public peer with no proxy header (a port-forward), and a
    // request without its peer address is refused (fail closed).
    let ray = get_from(NAME, "/echo", "127.0.0.1", &[("cf-ray", "r")]);
    let public = get_from(NAME, "/echo", "203.0.113.7", &[]);
    for request in [ray, public] {
        assert_eq!(plain(&hub, request).await.status(), StatusCode::FORBIDDEN);
    }
    let mut no_peer = odd(Method::GET, "/echo", Some(NAME));
    no_peer.extensions_mut().remove::<ConnectInfo<SocketAddr>>();
    let body = echo(plain(&hub, no_peer).await).await;
    let message = body["message"].as_str().unwrap();
    assert!(message.starts_with("Refused (no_peer_address)"), "{body}");
    // Through the tunnel the browser is on https (Cloudflare's edge):
    // AbleSet's http Location becomes https there too (#90 review 8).
    let moved = plain(&hub, tunnel(NAME, "/moved", Some(ablesets.as_str()))).await;
    let location = "https://ableset.example.org/next";
    assert_eq!(moved.headers()[header::LOCATION], location);
    // AbleSet's assertion never opens fohmixer.
    let fohmixer = plain(
        &hub,
        tunnel(TLS_NAME, "/api/version", Some(ablesets.as_str())),
    )
    .await;
    assert_eq!(fohmixer.status(), StatusCode::FORBIDDEN);
    // The Origin guard: another site's POST is refused, AbleSet's own passes.
    let press = |origin: &str| {
        let pairs = [("origin", origin)];
        request(
            Method::POST,
            NAME,
            "/press",
            "10.0.0.5",
            &pairs,
            Body::from("hello"),
        )
    };
    assert_eq!(
        plain(&hub, press("https://evil.example")).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        text_of(plain(&hub, press("https://ableset.example.org")).await).await,
        "5"
    );
    hub.stop();
    // Without an AbleSet audience the internet is refused, the LAN served.
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, Some(&keys), &[]);
    let refused = plain(&hub, tunnel(NAME, "/", Some(fohmixers.as_str()))).await;
    let body: Value = serde_json::from_str(&text_of(refused).await).unwrap();
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("Refused (no_access)"),
        "{body}"
    );
    let lan = plain(&hub, get_from(NAME, "/", "10.0.0.5", &[])).await;
    assert_eq!(text_of(lan).await, "fake /");
    hub.stop();
}

#[tokio::test]
async fn when_ableset_does_not_answer_the_hub_does() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), CLOSED, None, &[]);
    let pairs = [("sec-fetch-dest", "iframe")];
    let page = plain(&hub, get_from(NAME, "/", "10.0.0.5", &pairs)).await;
    assert_eq!(page.status(), StatusCode::BAD_GATEWAY);
    let page_policy = "default-src 'none'; style-src 'unsafe-inline'";
    assert_eq!(policies(page.headers()), [page_policy, OURS]);
    assert!(text_of(page).await.contains("AbleSet na PC neodpovedá"));
    let poll = plain(&hub, get_from(NAME, "/socket.io/?EIO=4", "10.0.0.5", &[])).await;
    let body: Value = serde_json::from_str(&text_of(poll).await).unwrap();
    assert_eq!(body["code"], "ABLESET_DOWN");
    hub.stop();
}

#[tokio::test]
async fn a_late_answer_is_504_and_a_body_over_the_cap_413() {
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_millis(300), 16);
    let through =
        |request: axum::http::Request<Body>| router(Arc::clone(&site), None).oneshot(request);
    let late = through(get_from(NAME, "/slow", "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(late.status(), StatusCode::GATEWAY_TIMEOUT);
    let press = |body: &'static str, length: Option<&str>| {
        let pairs: Vec<_> = length.map(|l| ("content-length", l)).into_iter().collect();
        request(
            Method::POST,
            NAME,
            "/press",
            "10.0.0.5",
            &pairs,
            Body::from(body),
        )
    };
    let at_cap = through(press("sixteen bytes ok", Some("16")))
        .await
        .unwrap();
    assert_eq!(text_of(at_cap).await, "16");
    let streamed = through(press("a body longer than the sixteen bytes", None))
        .await
        .unwrap();
    assert_eq!(streamed.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let errors: Vec<Value> = records.try_iter().map(|r| r["error"].clone()).collect();
    assert_eq!(
        errors,
        [
            "AbleSet did not answer within 300 ms",
            "a request body went over the 16 bytes AbleSet is sent"
        ]
    );
    // An announced size over the cap is refused before AbleSet is asked
    // (here nothing listens: asking would be a 502); a refused connection
    // is AbleSet down, named with its causes.
    let (site, records) = site_with(CLOSED, Duration::from_millis(300), 16);
    let through =
        |request: axum::http::Request<Body>| router(Arc::clone(&site), None).oneshot(request);
    let announced = through(press("seventeen bytes!!", Some("17")))
        .await
        .unwrap();
    assert_eq!(announced.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(policies(announced.headers()), [OURS]);
    let down = through(get_from(NAME, "/", "10.0.0.5", &[])).await.unwrap();
    assert_eq!(down.status(), StatusCode::BAD_GATEWAY);
    let errors: Vec<String> = records
        .try_iter()
        .map(|r| r["error"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        errors[0],
        "a request body of 17 bytes is over the 16 bytes AbleSet is sent"
    );
    let prefix = "AbleSet does not answer: client error (Connect): tcp connect error: ";
    assert!(
        errors[1].starts_with(prefix) && errors[1].len() > prefix.len(),
        "{errors:?}"
    );
}

#[tokio::test]
async fn the_plain_listener_redirects_ablesets_name_once_its_certificate_serves() {
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    let https =
        Arc::new(crate::https::Https::bind("127.0.0.1:0".parse().unwrap(), Router::new()).unwrap());
    let _ = hub.https.set(Arc::clone(&https));
    let ca = crate::tls::test_certs::TestCa::new();
    let now = crate::auth::now_secs() as i64;
    let serve = |slot: Slot, name: &str| {
        let (chain, key) = ca.leaf(&[name], now - 3600, now + 86_400);
        let pem = crate::tls::checked(chain, key, name).unwrap();
        https.set_cert(slot, name, &pem).unwrap();
    };
    let host = format!("{NAME}:8480");
    let ask = || get_from(&host, "/x?y=1", "10.0.0.5", &[]);
    // Not while AbleSet's certificate is not served: no certificate, then
    // only the [tls] name's.
    assert_eq!(text_of(plain(&hub, ask()).await).await, "fake /x?y=1");
    serve(Slot::Primary, TLS_NAME);
    assert_eq!(text_of(plain(&hub, ask()).await).await, "fake /x?y=1");
    serve(Slot::AbleSet, NAME);
    let redirected = plain(&hub, ask()).await;
    assert_eq!(redirected.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = format!("https://{NAME}:{}/x?y=1", https.addr().port());
    assert_eq!(redirected.headers()[header::LOCATION], location.as_str());
    assert_eq!(policies(redirected.headers()), [OURS]);
    // Never a tunnel request (refused here: no AbleSet audience), never on
    // the HTTPS listener.
    let pairs = [("cf-connecting-ip", "203.0.113.7")];
    let tunnel = plain(&hub, get_from(&host, "/x", "127.0.0.1", &pairs)).await;
    assert_eq!(tunnel.status(), StatusCode::FORBIDDEN);
    let over_https = crate::https_router(hub.clone())
        .oneshot(ask())
        .await
        .unwrap();
    assert_eq!(text_of(over_https).await, "fake /x?y=1");
    hub.stop();
}
