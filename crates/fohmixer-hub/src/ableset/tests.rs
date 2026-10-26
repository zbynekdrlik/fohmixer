//! Tests of AbleSet's name behind the hub (#90): the pure header, framing
//! and failure decisions, and the routers (the dispatch by `Host` on both
//! listeners, AbleSet's own gate, the proxy) against a fake AbleSet.

use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Receiver;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::{HeaderName, Uri, Version};
use axum::routing::{any, get, post};
use futures_util::{SinkExt as _, StreamExt as _};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{self, Message};

use super::*;
use crate::config::{DEFAULT_ABLESET_TITLE, DEFAULT_ABLESET_UPSTREAM};
use crate::test_keys::{KeyServer, claims, jwks, key, sign};
use crate::tls::test_certs::{TestCa, connect};
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
fn ablesets_cookies_stay_on_its_name_and_none_is_cloudflares() {
    for (set, kept) in [
        (
            "a=1; Domain=example.org; Path=/; HttpOnly",
            Some("a=1; Path=/; HttpOnly"),
        ),
        ("a=1;DOMAIN=.example.org;", Some("a=1")),
        ("theme=dark", Some("theme=dark")),
        ("cfg=1; domain = x; Secure", Some("cfg=1; Secure")),
        ("Domain=x", Some("Domain=x")),
        ("CF_Authorization=t; Path=/", None),
        ("cf_clearance=c", None),
        ("Cf_AppSession=x", None),
    ] {
        assert_eq!(host_only_cookie(set).as_deref(), kept, "{set}");
    }
}

#[test]
fn only_a_paths_request_no_other_site_made_goes_on_to_ableset() {
    let get = |target: &str, pairs: &[(&str, &str)]| {
        refusal(&Method::GET, &target.parse().unwrap(), &headers(pairs))
    };
    assert_eq!(get("/a?b=1", &[]), None);
    assert_eq!(get("http://ableset.example.org/a", &[]), None);
    for target in ["*", "ableset.example.org:443"] {
        assert!(
            matches!(get(target, &[]), Some(Failure::Target(_))),
            "{target}"
        );
    }
    for method in [Method::CONNECT, Method::TRACE] {
        let refused = refusal(&method, &Uri::from_static("/"), &HeaderMap::new());
        assert!(matches!(refused, Some(Failure::Method(_))), "{method}");
    }
    let site =
        |site: &str, mode: &str| get("/", &[("sec-fetch-site", site), ("sec-fetch-mode", mode)]);
    assert_eq!(site("Cross-Site", "Navigate"), None);
    for mode in ["no-cors", "cors", "websocket", ""] {
        let refused = site("cross-site", mode);
        assert!(matches!(refused, Some(Failure::CrossSite(_))), "{mode}");
    }
    let named = site("CROSS-SITE", "no-cors").unwrap();
    assert_eq!(
        named.why(),
        "a cross-site request: mode \"no-cors\", dest \"\""
    );
    for other in ["same-origin", "same-site", "none", ""] {
        assert_eq!(site(other, "no-cors"), None, "{other}");
    }
}

#[test]
fn a_warn_line_is_for_news_and_at_most_once_per_gap() {
    assert_eq!(WARN_GAP, Duration::from_secs(10));
    assert!(warn_line(true, None));
    assert!(warn_line(true, Some(WARN_GAP)));
    assert!(!warn_line(true, Some(WARN_GAP - Duration::from_millis(1))));
    assert!(!warn_line(false, None));
    assert!(!warn_line(false, Some(WARN_GAP)));
    let ableset = [
        Failure::Down(String::new()),
        Failure::Timeout(String::new()),
    ];
    assert!(ableset.iter().all(Failure::is_ableset));
    assert!(!Failure::Busy(String::new()).is_ableset());
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
        title: DEFAULT_ABLESET_TITLE.into(),
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
            text.contains("AbleSet on the PC is not answering. Retrying in 5 s."),
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
    // Every one is recorded, and the outage's end (#90 B3h review 1); only
    // the warned fault of AbleSet itself under `error` (warn class, kept
    // past the event log's daily cap), the rest under `why`: no client
    // fills the PC's disk.
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
    assert_eq!(keys, ["error", "why", "why", "why", "why", "why"]);
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
            ("/e", 200, "AbleSet answers again"),
            ("/f", 502, refused)
        ]
    );
}

#[test]
fn a_clients_faults_are_recorded_once_per_gap_per_kind_and_ablesets_own_each_time() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let busy = "every WebSocket slot for AbleSet is taken";
    // A client in a loop (#90 B3h, review finding 6): one record per kind
    // in a gap, the repeats left out (counted on the next one); AbleSet's
    // own fault every time.
    for _ in 0..3 {
        site.failed(&Method::GET, "/a", None, Failure::Busy(busy.into()));
        site.failed(&Method::GET, "/b", None, Failure::CrossSite("x".into()));
        site.failed(&Method::GET, "/c", None, Failure::Down("refused".into()));
    }
    site.failed(&Method::GET, "/d", None, Failure::Target("t".into()));
    let records: Vec<Value> = records.try_iter().collect();
    let rows: Vec<(&str, &str, u64, bool)> = records
        .iter()
        .map(|r| {
            let (key, why) = why_of(r);
            let skipped = r["skipped"].as_u64().unwrap();
            (key, why, skipped, crate::events::is_warn(r))
        })
        .collect();
    // Only AbleSet's outage outlives the event log's daily cap (warn
    // class), and only its warned first record; a client's faults never.
    assert_eq!(
        rows,
        [
            ("why", busy, 0, false),
            ("why", "x", 0, false),
            ("error", "refused", 0, true),
            ("why", "refused", 0, false),
            ("why", "refused", 0, false),
            ("why", "t", 0, false),
        ]
    );
    // Every kind is its own ration.
    let every = [
        Failure::Method(String::new()),
        Failure::Target(String::new()),
        Failure::CrossSite(String::new()),
        Failure::UpgradeBody(String::new()),
        Failure::Closed(String::new()),
        Failure::Busy(String::new()),
        Failure::TooLarge(String::new()),
        Failure::Down(String::new()),
        Failure::Timeout(String::new()),
    ];
    let kinds: std::collections::HashSet<&str> = every.iter().map(Failure::kind).collect();
    assert_eq!(kinds.len(), every.len(), "{kinds:?}");
    assert_eq!(FAULT_RECORDS, 1);
}

#[test]
fn an_outage_is_recorded_at_its_start_then_rationed_and_at_its_end_with_the_count_left_out() {
    // #90 B3h review 1: a client looping on AbleSet's name while AbleSet is
    // closed. The outage's first record and its end go out at once; the
    // rest, 8 per kind per gap, the count left out on the end's record.
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let refused = "AbleSet does not answer: refused";
    for _ in 0..20 {
        site.failed(&Method::GET, "/a", None, Failure::Down(refused.into()));
    }
    site.reached(&Method::GET, "/b", StatusCode::OK, Instant::now());
    // A new outage's first record goes out at once though its kind's
    // ration is used up.
    site.failed(&Method::GET, "/c", None, Failure::Down(refused.into()));
    let records: Vec<Value> = records.try_iter().collect();
    let rows: Vec<(&str, &str, u64)> = records
        .iter()
        .map(|r| {
            let (key, why) = why_of(r);
            (key, why, r["skipped"].as_u64().unwrap())
        })
        .collect();
    let mut expected: Vec<(&str, &str, u64)> = vec![("error", refused, 0)];
    expected.extend([("why", refused, 0); 8]);
    expected.push(("why", "AbleSet answers again", 11));
    expected.push(("why", refused, 0));
    assert_eq!(rows, expected);
    assert_eq!(
        (&records[9]["path"], &records[9]["status"]),
        (&json!("/b"), &json!(200))
    );
    // A hung AbleSet: its warned late answer at once, the rest rationed.
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    for _ in 0..12 {
        site.failed(&Method::GET, "/d", None, Failure::Timeout("late".into()));
    }
    let keys: Vec<&str> = records.try_iter().map(|r| why_of(&r).0).collect();
    let mut expected = vec!["error"];
    expected.extend(["why"; 8]);
    assert_eq!(keys, expected);
}

#[test]
fn a_clients_long_text_is_clipped_in_the_record_and_the_warn_line() {
    // #90 B3h review 2: a path up to 64 KB, a Sec-Fetch-* value up to the
    // head's 400 KB, a target that is no path: cut to 256 characters with
    // a mark naming the whole size, in the record and in hub.out.log.
    let path = format!("/{}", "p".repeat(60 * 1024));
    let mode = "m".repeat(300 * 1024);
    let cross = headers(&[
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", mode.as_str()),
    ]);
    let failure = refusal(&Method::GET, &"/".parse().unwrap(), &cross).unwrap();
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let log = logged(|| {
        site.failed(&Method::GET, &path, None, failure);
    });
    let records: Vec<Value> = records.try_iter().collect();
    let clipped = format!("{}… ({} bytes)", &path[..256], path.len());
    assert_eq!(records[0]["path"], json!(clipped));
    let why = why_of(&records[0]).1;
    assert!(why.len() < 1024, "{} bytes", why.len());
    assert!(why.contains(&format!("… ({} bytes)", mode.len())), "{why}");
    assert!(log.len() < 2048, "{} bytes", log.len());
    assert!(log.contains(" WARN ") && log.contains(&clipped), "{log}");
    let target: Uri = format!("{}:1", "h".repeat(1000)).parse().unwrap();
    let Some(Failure::Target(why)) = refusal(&Method::GET, &target, &HeaderMap::new()) else {
        panic!("a target that is no path");
    };
    assert!(why.len() < 512 && why.contains("… (1002 bytes)"), "{why}");
}

/// A record's key for its text (`error`: warn class, or `why`) and the text.
fn why_of(record: &Value) -> (&'static str, &str) {
    let key = if record.get("error").is_some() {
        "error"
    } else {
        "why"
    };
    (key, record[key].as_str().unwrap())
}

/// A fake AbleSet on 127.0.0.1: `/echo` answers the headers it got as JSON
/// (a header's values `, `-joined), `/framed` with AbleSet-like framing
/// headers, `/moved` an http redirect of AbleSet's name, `/cookies`,
/// `/cors` and `/hop` with such headers, `/slow` never, `POST /press` the
/// length of its body, `/socket.io/` a WebSocket echo ([`ws_echo`]),
/// `/gone` a WebSocket it closes at once (its TCP connection, no close
/// frame), anything else `fake <path and query>`.
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
        .route("/stream", get(|| async { endless() }))
        .route("/socket.io/", get(ws_echo))
        .route("/gone", get(ws_gone))
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

/// A WebSocket echo (#90 B2) with the subprotocol `x-test`: text and
/// binary messages come back, `bye` is answered with a close.
async fn ws_echo(ws: WebSocketUpgrade) -> Response {
    use axum::extract::ws::Message as M;
    ws.protocols(["x-test"])
        .on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                let reply = match message {
                    M::Text(ref text) if text.as_str() == "bye" => M::Close(None),
                    M::Text(_) | M::Binary(_) => message,
                    _ => continue,
                };
                if socket.send(reply).await.is_err() {
                    break;
                }
            }
        })
}

/// A WebSocket (the subprotocol `x-test`) AbleSet closes at once: its TCP
/// connection ends, no close frame is sent.
async fn ws_gone(ws: WebSocketUpgrade) -> Response {
    ws.protocols(["x-test"])
        .on_upgrade(|socket| async move { drop(socket) })
}

/// An answer whose body sends one chunk and never ends.
fn endless() -> Body {
    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from("a")) });
    Body::from_stream(first.chain(futures_util::stream::pending()))
}

#[tokio::test]
async fn a_request_over_the_limit_is_503_at_once_its_slot_held_while_an_answer_streams() {
    let upstream = fake().await;
    let (events, records) = EventLog::channel(64);
    let proxy = Proxy::new(&upstream, Duration::from_secs(5), 16);
    let site = Arc::new(AbleSet::limited(
        NAME,
        &tls(),
        None,
        events,
        proxy,
        1,
        MAX_SOCKETS,
    ));
    let through =
        |request: axum::http::Request<Body>| router(Arc::clone(&site), None).oneshot(request);
    // The one slot is held by an answer still streaming, not only its head.
    let streaming = through(get_from(NAME, "/stream", "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(streaming.status(), StatusCode::OK);
    let busy = through(get_from(NAME, "/p", "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(policies(busy.headers()), [OURS]);
    let body: Value = serde_json::from_str(&text_of(busy).await).unwrap();
    assert_eq!(body["code"], "ABLESET_BUSY");
    let why: Vec<Value> = records.try_iter().map(|r| r["why"].clone()).collect();
    assert_eq!(why, ["every request slot for AbleSet is taken"]);
    // The answer gone (the client left), its slot is free again.
    drop(streaming);
    let next = through(get_from(NAME, "/p", "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(text_of(next).await, "fake /p");
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
        title: DEFAULT_ABLESET_TITLE.into(),
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
    assert!(
        text_of(page)
            .await
            .contains("AbleSet on the PC is not answering")
    );
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
    // AbleSet's late answer is a warn-class record, the client's body over
    // the cap is not.
    let records: Vec<Value> = records.try_iter().collect();
    let whys: Vec<(&str, &str)> = records.iter().map(why_of).collect();
    assert_eq!(
        whys,
        [
            ("error", "AbleSet did not answer within 300 ms"),
            (
                "why",
                "a request body went over the 16 bytes AbleSet is sent"
            )
        ]
    );
    // An announced size over the cap is refused before AbleSet is asked
    // (here nothing listens: asking would be a 502); a refused connection
    // is AbleSet down, named with its causes (under the real head bound:
    // Windows tries a refused connect again for about 1 s).
    let (site, records) = site_with(CLOSED, proxy::HEAD_TIMEOUT, 16);
    let through =
        |request: axum::http::Request<Body>| router(Arc::clone(&site), None).oneshot(request);
    let announced = through(press("seventeen bytes!!", Some("17")))
        .await
        .unwrap();
    assert_eq!(announced.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(policies(announced.headers()), [OURS]);
    let down = through(get_from(NAME, "/", "10.0.0.5", &[])).await.unwrap();
    assert_eq!(down.status(), StatusCode::BAD_GATEWAY);
    let records: Vec<Value> = records.try_iter().collect();
    let whys: Vec<(&str, &str)> = records.iter().map(why_of).collect();
    assert_eq!(
        whys[0],
        (
            "why",
            "a request body of 17 bytes is over the 16 bytes AbleSet is sent"
        )
    );
    assert_eq!(whys[1].0, "error");
    let errors: Vec<&str> = whys.iter().map(|(_, why)| *why).collect();
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

#[test]
fn a_websocket_is_a_get_naming_the_upgrade_and_websocket_whose_pair_goes_on() {
    let ask = |method: Method, pairs: &[(&str, &str)]| {
        is_upgrade(&method, Version::HTTP_11, &headers(pairs))
    };
    let socket = [("connection", "Upgrade"), ("upgrade", "websocket")];
    assert!(ask(Method::GET, &socket));
    // Only HTTP/1.1 switches (#90 review 5): hyper offers no upgrade on
    // HTTP/1.0, and HTTP/2 has none.
    for version in [Version::HTTP_10, Version::HTTP_2] {
        assert!(
            !is_upgrade(&Method::GET, version, &headers(&socket)),
            "{version:?}"
        );
    }
    let listed = [
        ("connection", "keep-alive, Upgrade"),
        ("upgrade", "h2c, WebSocket"),
    ];
    assert!(ask(Method::GET, &listed));
    let two = [("connection", "keep-alive"), ("connection", "upgrade")];
    assert!(ask(
        Method::GET,
        &[two[0], two[1], ("upgrade", "websocket")]
    ));
    assert!(!ask(Method::POST, &socket));
    for pairs in [
        &[("upgrade", "websocket")][..],
        &[("connection", "upgrade")][..],
        &[("connection", "upgrade"), ("upgrade", "h2c")][..],
        &[("connection", "upgraded"), ("upgrade", "websocket")][..],
        &[("connection", "upgrade"), ("upgrade", "websockets")][..],
    ] {
        assert!(!ask(Method::GET, pairs), "{pairs:?}");
    }
    // After the hop-by-hop headers went, the pair is put back; the
    // handshake's own headers pass as they are.
    let mut map = headers(&[
        ("connection", "keep-alive, Upgrade, x-hop"),
        ("upgrade", "WebSocket"),
        ("x-hop", "1"),
        ("sec-websocket-key", "k"),
        ("sec-websocket-protocol", "x-test"),
        ("sec-websocket-extensions", "permessage-deflate"),
    ]);
    strip_hop_by_hop(&mut map);
    upgrade_headers(&mut map);
    let mut names: Vec<&str> = map.keys().map(HeaderName::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "connection",
            "sec-websocket-extensions",
            "sec-websocket-key",
            "sec-websocket-protocol",
            "upgrade"
        ]
    );
    assert_eq!(map["connection"], "upgrade");
    assert_eq!(map["upgrade"], "websocket");
}

#[tokio::test]
async fn an_upgrade_that_announces_a_body_in_any_way_is_refused_400() {
    let refused = |pairs: &[(&str, &str)]| upgrade_body(&headers(pairs)).is_some();
    assert!(!refused(&[
        ("connection", "upgrade"),
        ("upgrade", "websocket")
    ]));
    assert!(!refused(&[("content-length", "0")]));
    assert!(!refused(&[("content-length", " 0 ")]));
    for pairs in [
        &[("content-length", "1")][..],
        &[("content-length", "1, 1")][..],
        &[("content-length", "0"), ("content-length", "5")][..],
        &[("transfer-encoding", "chunked")][..],
        &[("content-length", "0"), ("transfer-encoding", "gzip")][..],
    ] {
        assert!(refused(pairs), "{pairs:?}");
    }
    let (site, _records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let failure = upgrade_body(&headers(&[("content-length", "1")])).unwrap();
    let answer = site.failed(&Method::GET, "/socket.io/", None, failure);
    assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_str(&text_of(answer).await).unwrap();
    assert_eq!(body["code"], "ABLESET_UPGRADE_BODY");
}

#[test]
fn a_tunnels_records_name_it_its_life_its_bytes_and_why_each_kind_rationed() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let from = "10.0.0.5:50000".parse().unwrap();
    let opener = crate::ws::opener(from, &headers(&[("origin", "https://ableset.example.org")]));
    let closed = Ended {
        up: 12,
        down: 34,
        why: "closed".into(),
    };
    let failed = Ended {
        up: 5,
        down: 0,
        why: "the page to AbleSet: reset by the peer".into(),
    };
    // A loop of opens: WS_RECORDS of them in a gap, the rest left out; a
    // close has a ration of its own.
    for _ in 0..=WS_RECORDS {
        site.ws_note(7, &opener, &Ws::Open);
    }
    site.ws_note(
        7,
        &opener,
        &Ws::Close {
            ms: 5,
            ended: &closed,
        },
    );
    site.ws_note(
        7,
        &opener,
        &Ws::Close {
            ms: 6,
            ended: &failed,
        },
    );
    let records: Vec<Value> = records
        .try_iter()
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect();
    let opens = WS_RECORDS as usize;
    assert_eq!(records.len(), opens + 2);
    let open = json!({"ev": "ableset_ws", "what": "open", "client": 7, "peer": "10.0.0.5",
        "forwarded": "-", "source": "lan", "origin": "ableset.example.org", "skipped": 0});
    assert!(records[..opens].iter().all(|r| *r == open), "{records:?}");
    assert_eq!(
        records[opens..],
        [
            json!({"ev": "ableset_ws", "what": "close", "client": 7, "ms": 5, "up": 12,
                "down": 34, "why": "closed", "skipped": 0}),
            json!({"ev": "ableset_ws", "what": "close", "client": 7, "ms": 6, "up": 5,
                "down": 0, "why": "the page to AbleSet: reset by the peer", "skipped": 0})
        ]
    );
}

#[tokio::test]
async fn an_upgrade_ableset_does_not_switch_is_answered_as_it_came_its_handshake_sent_on() {
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_secs(5), 16);
    let pairs = [
        ("connection", "keep-alive, Upgrade"),
        ("upgrade", "websocket"),
        ("sec-websocket-key", "k"),
        ("sec-websocket-version", "13"),
        ("sec-websocket-protocol", "x-test"),
        ("sec-websocket-extensions", "permessage-deflate"),
    ];
    let answer = router(Arc::clone(&site), None)
        .oneshot(get_from(NAME, "/echo", "10.0.0.5", &pairs))
        .await
        .unwrap();
    assert_eq!(answer.status(), StatusCode::OK);
    let seen = echo(answer).await;
    assert_eq!(
        (&seen["connection"], &seen["upgrade"]),
        (&json!("upgrade"), &json!("websocket"))
    );
    for (name, value) in &pairs[2..] {
        assert_eq!(seen[*name], json!(value), "{name}");
    }
    // No tunnel was opened: every open is an `ableset_ws` record (the first
    // of its kind in a gap always is), and nothing was refused.
    let records: Vec<Value> = records.try_iter().collect();
    assert!(records.is_empty(), "{records:?}");
}

/// Serves `app` on 127.0.0.1 as the plain listener does (axum-server, the
/// peer's address, its HTTP/1 settings): its address.
async fn listen(app: Router) -> SocketAddr {
    listen_with(app, crate::HEADER_READ).await
}

/// [`listen`] with `header_read` as its request heads' bound.
async fn listen_with(app: Router, header_read: Duration) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = crate::accept::server(listener, header_read);
    tokio::spawn(server.serve(app.into_make_service_with_connect_info::<SocketAddr>()));
    addr
}

/// A WebSocket for `url` over `stream`, asking for the subprotocol
/// `x-test`, with `pairs` added: the socket and its 101's headers, or the
/// refusal.
async fn socket<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    url: &str,
    pairs: &[(&str, &str)],
) -> Result<(WebSocketStream<S>, HeaderMap), tungstenite::Error> {
    let mut request = url.into_client_request().unwrap();
    let protocol = HeaderValue::from_static("x-test");
    request
        .headers_mut()
        .insert("sec-websocket-protocol", protocol);
    request.headers_mut().extend(headers(pairs));
    let (socket, answer) = tokio_tungstenite::client_async(request, stream).await?;
    Ok((socket, answer.headers().clone()))
}

/// The status of an upgrade that was refused.
fn refused<S>(opened: Result<(WebSocketStream<S>, HeaderMap), tungstenite::Error>) -> u16 {
    match opened {
        Err(tungstenite::Error::Http(answer)) => answer.status().as_u16(),
        Err(other) => panic!("the upgrade failed otherwise: {other}"),
        Ok(_) => panic!("the upgrade was let through"),
    }
}

/// `message` sent, and the next message read, within 5 s.
async fn round_trip<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: Message,
) -> Message {
    let trip = async {
        socket.send(message).await.unwrap();
        socket.next().await.unwrap().unwrap()
    };
    tokio::time::timeout(Duration::from_secs(5), trip)
        .await
        .expect("an answer within 5 s")
}

/// Whether the socket ends (a close, or its connection's end) within 5 s.
async fn ends<S: AsyncRead + AsyncWrite + Unpin>(socket: &mut WebSocketStream<S>) -> bool {
    let end = async {
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), end)
        .await
        .is_ok()
}

/// Waits up to 5 s for `open` tunnels.
async fn tunnels_until(site: &AbleSet, open: usize) {
    tunnels_within(site, open, Duration::from_secs(5)).await;
}

/// Waits up to `limit` for `open` taken socket slots.
async fn tunnels_within(site: &AbleSet, open: usize, limit: Duration) {
    let deadline = Instant::now() + limit;
    while site.open_sockets() != open && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(site.open_sockets(), open, "after {limit:?}");
}

#[tokio::test]
async fn a_websocket_of_ablesets_name_is_a_byte_tunnel_on_both_listeners_until_an_end_or_the_stop()
{
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    let site = Arc::clone(hub.ableset.as_ref().unwrap());
    // The plain listener: text and a large binary message both ways, the
    // subprotocol agreed with AbleSet, the tunnel in /api/status.
    let plain = listen(crate::app_router(hub.clone())).await;
    let url = format!("ws://{NAME}:{}/socket.io/?EIO=4", plain.port());
    let lan = || async move { TcpStream::connect(plain).await.unwrap() };
    let (mut ws, answer) = socket(lan().await, &url, &[]).await.unwrap();
    assert_eq!(answer["sec-websocket-protocol"], "x-test");
    let hi = Message::text("hi");
    assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    let big = Message::binary(vec![7_u8; 1 << 20]);
    assert_eq!(round_trip(&mut ws, big.clone()).await, big);
    let status = hub.status().await.ableset.unwrap();
    assert_eq!(status.sockets, 1);
    // The page's close reaches AbleSet, whose close comes back.
    ws.close(None).await.unwrap();
    assert!(ends(&mut ws).await, "AbleSet's close came back");
    drop(ws);
    tunnels_until(&site, 0).await;
    assert_eq!(hub.status().await.ableset.unwrap().sockets, 0);
    // From the internet only with an assertion for AbleSet (none here).
    let tunnel = [("cf-connecting-ip", "203.0.113.7")];
    assert_eq!(refused(socket(lan().await, &url, &tunnel).await), 403);
    // HTTPS by AbleSet's name (SNI): AbleSet's close reaches the page.
    let ca = TestCa::new();
    let now = crate::auth::now_secs() as i64;
    let https = crate::https::Https::bind(
        "127.0.0.1:0".parse().unwrap(),
        crate::https_router(hub.clone()),
    )
    .unwrap();
    for (slot, name) in [(Slot::Primary, TLS_NAME), (Slot::AbleSet, NAME)] {
        let (chain, key) = ca.leaf(&[name], now - 3600, now + 86_400);
        let pem = crate::tls::checked(chain, key, name).unwrap();
        https.set_cert(slot, name, &pem).unwrap();
    }
    let tls = connect(https.addr(), NAME, &ca.pem, true).await.unwrap();
    let secure = format!("wss://{NAME}/socket.io/?EIO=4");
    let (mut ws, _) = socket(tls, &secure, &[]).await.unwrap();
    let over = Message::text("over tls");
    assert_eq!(round_trip(&mut ws, over.clone()).await, over);
    let bye = round_trip(&mut ws, Message::text("bye")).await;
    assert!(bye.is_close(), "{bye:?}");
    drop(ws);
    tunnels_until(&site, 0).await;
    // The hub's stop closes an open tunnel.
    let (mut ws, _) = socket(lan().await, &url, &[]).await.unwrap();
    assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    hub.stop();
    assert!(ends(&mut ws).await, "the stop closed the tunnel");
    tunnels_until(&site, 0).await;
    https.stop(Duration::from_millis(10));
}

#[tokio::test]
async fn a_tunnel_past_the_limit_is_503_at_once_and_its_slot_comes_back_when_it_ends() {
    let upstream = fake().await;
    let (events, records) = EventLog::channel(64);
    let proxy = Proxy::new(&upstream, Duration::from_secs(5), 16);
    let site = Arc::new(AbleSet::limited(NAME, &tls(), None, events, proxy, 4, 1));
    let at = listen(router(Arc::clone(&site), None)).await;
    let url = format!("ws://{NAME}/socket.io/");
    let lan = || async move { TcpStream::connect(at).await.unwrap() };
    let (mut first, _) = socket(lan().await, &url, &[]).await.unwrap();
    assert_eq!(site.open_sockets(), 1);
    assert_eq!(refused(socket(lan().await, &url, &[]).await), 503);
    let hi = Message::text("hi");
    assert_eq!(round_trip(&mut first, hi.clone()).await, hi);
    first.close(None).await.unwrap();
    assert!(ends(&mut first).await);
    drop(first);
    tunnels_until(&site, 0).await;
    let (mut second, _) = socket(lan().await, &url, &[]).await.unwrap();
    assert_eq!(round_trip(&mut second, hi.clone()).await, hi);
    // The first one's open and close, the refusal between them.
    let records: Vec<Value> = records.try_iter().collect();
    let what: Vec<(&str, &str)> = records
        .iter()
        .map(|r| {
            let what = r.get("what").or_else(|| r.get("why")).unwrap();
            (r["ev"].as_str().unwrap(), what.as_str().unwrap())
        })
        .collect();
    assert_eq!(
        what[..4],
        [
            ("ableset_ws", "open"),
            ("ableset_proxy", "every WebSocket slot for AbleSet is taken"),
            ("ableset_ws", "close"),
            ("ableset_ws", "open")
        ]
    );
    let close = &records[2];
    assert_eq!(
        (&close["client"], &close["why"]),
        (&json!(1), &json!("closed"))
    );
    let bytes = |key: &str| close[key].as_u64().unwrap();
    assert!(bytes("up") > 0 && bytes("down") > 0, "{close}");
    assert_eq!(records[3]["client"], 2);
}

/// The `ableset_ws` records of kind `what` (`open`, `close`) in `records`.
fn ws_of<'a>(records: &'a [Value], what: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "ableset_ws" && r["what"] == what)
        .collect()
}

/// A fake AbleSet that reads a request's head and closes the connection
/// without an answer (as engine.io does with an upgrade it does not
/// serve): its URL, and the count of connections it took.
async fn closer() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let taken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&taken);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0_u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
            });
        }
    });
    (format!("http://{addr}"), taken)
}

/// A fake AbleSet that reads a request's head, then resets the connection
/// (`SO_LINGER` 0 before the drop: a reset on every platform, as Windows can
/// answer a close of an upgrade it does not serve, #90).
async fn resetter() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0_u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                stream.set_zero_linger().unwrap();
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn an_upgrade_ableset_resets_after_it_read_is_no_outage() {
    // #90: AbleSet read the upgrade and reset the connection without an
    // answer (Windows' way of a close): the client's fault, never an outage.
    let pair = [("connection", "upgrade"), ("upgrade", "websocket")];
    let upstream = resetter().await;
    let (site, records) = site_with(&upstream, proxy::HEAD_TIMEOUT, 16);
    let reset = router(Arc::clone(&site), None)
        .oneshot(get_from(NAME, "/elsewhere", "10.0.0.5", &pair))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_str(&text_of(reset).await).unwrap();
    assert_eq!(body["code"], "ABLESET_CLOSED");
    assert!(!site.failing.load(Ordering::Relaxed), "no outage");
    let records: Vec<Value> = records.try_iter().collect();
    assert_eq!(records.len(), 1, "{records:?}");
    let (key, why) = why_of(&records[0]);
    let text = "AbleSet closed the WebSocket request without an answer: ";
    assert!(
        key == "why" && why.starts_with(text) && why.len() > text.len(),
        "{key}: {why}"
    );
}

/// The error of a GET through hyper's client to `url` (none answers).
async fn client_error(url: &str) -> hyper_util::client::legacy::Error {
    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http::<http_body_util::Empty<Bytes>>();
    client
        .get(url.parse().unwrap())
        .await
        .expect_err("no answer")
}

/// An error whose source is a client's error (a connect error under a
/// reset, as no real connect gives one).
#[derive(Debug)]
struct Under(hyper_util::client::legacy::Error);

impl std::fmt::Display for Under {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "under")
    }
}

impl std::error::Error for Under {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[tokio::test]
async fn a_close_or_a_reset_after_the_connect_is_closed_and_a_failed_connect_is_down() {
    // One error at a time: an I/O reset or abort ends unanswered, another
    // I/O error does not.
    assert!(proxy::ended_unanswered(&io::Error::from(
        io::ErrorKind::ConnectionReset
    )));
    assert!(proxy::ended_unanswered(&io::Error::from(
        io::ErrorKind::ConnectionAborted
    )));
    assert!(!proxy::ended_unanswered(&io::Error::from(
        io::ErrorKind::ConnectionRefused
    )));
    assert!(!proxy::ended_unanswered(&io::Error::other("reset")));
    // AbleSet read the request and closed (hyper's incomplete message).
    let (closing, _) = closer().await;
    let incomplete = client_error(&closing).await;
    assert!(!proxy::connect_failed(&incomplete));
    assert!(proxy::closed_unanswered(&incomplete), "{incomplete:?}");
    assert!(matches!(
        proxy::failure(&incomplete, true, 16),
        Failure::Closed(_)
    ));
    assert!(
        matches!(proxy::failure(&incomplete, false, 16), Failure::Down(_)),
        "not an upgrade"
    );
    // It read the request and reset the connection.
    let reset = client_error(&resetter().await).await;
    assert!(!proxy::connect_failed(&reset));
    assert!(proxy::closed_unanswered(&reset), "{reset:?}");
    assert!(matches!(
        proxy::failure(&reset, true, 16),
        Failure::Closed(_)
    ));
    // A refused connect: AbleSet down.
    let refused = client_error(CLOSED).await;
    assert!(proxy::connect_failed(&refused), "{refused:?}");
    assert!(!proxy::closed_unanswered(&refused));
    assert!(matches!(
        proxy::failure(&refused, true, 16),
        Failure::Down(_)
    ));
    // A reset at the connect (a connect error under a reset): still down.
    let at_connect = io::Error::new(io::ErrorKind::ConnectionReset, Under(refused));
    assert!(proxy::connect_failed(&at_connect));
    assert!(!proxy::closed_unanswered(&at_connect));
    assert!(matches!(
        proxy::failure(&at_connect, true, 16),
        Failure::Down(_)
    ));
    // A reset with no client error at all ends unanswered.
    assert!(!proxy::connect_failed(&io::Error::from(
        io::ErrorKind::ConnectionReset
    )));
    assert!(proxy::closed_unanswered(&io::Error::from(
        io::ErrorKind::ConnectionReset
    )));
}

#[tokio::test]
async fn a_tunnel_ableset_closed_ends_after_the_grace_though_the_page_stays_silent() {
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_secs(5), 16);
    let at = listen(router(Arc::clone(&site), None)).await;
    // AbleSet closes its end at once; the page never reads, writes or
    // closes (a suspended tab on a tablet that then leaves the Wi-Fi:
    // nothing in flight, so no TCP timer would ever end it, #90 review 1).
    let page = TcpStream::connect(at).await.unwrap();
    let (silent, _) = socket(page, &format!("ws://{NAME}/gone"), &[])
        .await
        .unwrap();
    let opened = Instant::now();
    tunnels_within(&site, 0, Duration::from_secs(8)).await;
    // The page had the grace (5 s) to close its end, then the slot came
    // back.
    let held = opened.elapsed();
    assert!(held >= Duration::from_millis(4_900), "{held:?}");
    let records: Vec<Value> = records.try_iter().collect();
    let close = ws_of(&records, "close");
    assert_eq!(close.len(), 1, "{records:?}");
    let why = "the page did not close within 5000 ms of AbleSet's close";
    assert_eq!(
        (&close[0]["why"], &close[0]["up"], &close[0]["down"]),
        (&json!(why), &json!(0), &json!(0))
    );
    drop(silent);
}

#[tokio::test]
async fn an_upgrade_that_announces_a_body_is_400_before_it_takes_a_slot_and_no_outage() {
    // A WebSocket handshake is a GET without a body (RFC 6455 4.1). One
    // that announces a body would keep its connection to AbleSet as long as
    // the page holds its own: hyper hands neither connection over before
    // the body is read whole (#90 re-review Low 2). It is refused 400
    // before it takes a socket slot (none is free here) and before AbleSet
    // is asked, as the client's fault.
    let (upstream, contacted) = closer().await;
    let (events, records) = EventLog::channel(64);
    let proxy = Proxy::new(&upstream, Duration::from_secs(5), 16);
    let site = Arc::new(AbleSet::limited(NAME, &tls(), None, events, proxy, 4, 0));
    let at = listen(router(Arc::clone(&site), None)).await;
    for body in ["Content-Length: 1", "Transfer-Encoding: chunked"] {
        let mut page = TcpStream::connect(at).await.unwrap();
        let ask = format!(
            "GET /socket.io/ HTTP/1.1\r\nHost: {NAME}\r\nConnection: Upgrade\r\n\
             Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n{body}\r\n\r\n"
        );
        let sent = tokio::io::AsyncWriteExt::write_all(&mut page, ask.as_bytes());
        sent.await.unwrap();
        let mut status = [0_u8; 12];
        tokio::time::timeout(Duration::from_secs(5), page.read_exact(&mut status))
            .await
            .expect("an answer within 5 s")
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&status), "HTTP/1.1 400", "{body}");
    }
    assert_eq!(contacted.load(Ordering::Relaxed), 0, "AbleSet was asked");
    assert!(!site.failing.load(Ordering::Relaxed), "no outage");
    let records: Vec<Value> = records.try_iter().collect();
    assert_eq!(
        (&records[0]["ev"], &records[0]["status"]),
        (&json!("ableset_proxy"), &json!(400))
    );
    let why = "a WebSocket upgrade announced a request body";
    assert_eq!(why_of(&records[0]), ("why", why));
    assert!(
        records.iter().all(|r| why_of(r) == ("why", why)),
        "{records:?}"
    );
}

#[tokio::test]
async fn a_request_body_the_page_breaks_is_its_fault_never_an_outage() {
    // #90 B3h review 3: a chunked POST whose body breaks off (a malformed
    // chunk) fails the request to AbleSet; that is the page's fault, never
    // an AbleSet outage.
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_secs(5), 1024);
    let at = listen(router(Arc::clone(&site), None)).await;
    let mut page = TcpStream::connect(at).await.unwrap();
    let ask =
        format!("POST /press HTTP/1.1\r\nHost: {NAME}\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n");
    let sent = tokio::io::AsyncWriteExt::write_all(&mut page, ask.as_bytes());
    sent.await.unwrap();
    let mut status = [0_u8; 12];
    tokio::time::timeout(Duration::from_secs(5), page.read_exact(&mut status))
        .await
        .expect("an answer within 5 s")
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status), "HTTP/1.1 400");
    assert!(!site.failing.load(Ordering::Relaxed), "no outage");
    let records: Vec<Value> = records.try_iter().collect();
    let (key, why) = why_of(&records[0]);
    let text = "the page's request body broke off: ";
    assert!(key == "why" && why.starts_with(text), "{key}: {why}");
}

#[test]
fn a_page_body_error_is_marked_as_the_pages_and_never_ablesets() {
    let marked = proxy::ClientBody(axum::Error::new(io::Error::other("cut")));
    assert_eq!(marked.to_string(), "the page's request body: cut");
    assert!(std::error::Error::source(&marked).is_some());
    assert!(proxy::broke_off(&marked));
    let failure = proxy::failure(&marked, false, 16);
    let text = "the page's request body broke off: the page's request body: cut";
    let Failure::BrokenBody(why) = &failure else {
        panic!("{failure:?}");
    };
    assert!(why.starts_with(text), "{why}");
    assert_eq!(failure.kind(), "broken_body");
    assert!(!failure.is_ableset());
    // An error of AbleSet's own is not the page's.
    assert!(!proxy::broke_off(&io::Error::other("reset")));
}

#[tokio::test]
async fn a_tunnel_whose_upgrades_never_complete_gives_its_slot_back_after_the_head_bound() {
    // #90 B3h review 4: carry gives the two upgrades the proxy's head bound
    // (300 ms here), so a page that never reads its 101 (a full receive
    // window: hyper hands its connection over only once the 101 is out)
    // holds no slot for good.
    let (site, records) = site_with(CLOSED, Duration::from_millis(300), 16);
    let opener = crate::ws::opener("10.0.0.5:50000".parse().unwrap(), &HeaderMap::new());
    let slot = Arc::clone(&site.sockets).try_acquire_owned().unwrap();
    let never = std::future::pending::<Result<Upgraded, hyper::Error>>;
    let carried = Arc::clone(&site).carry(1, opener, never(), never(), slot);
    tokio::time::timeout(Duration::from_secs(5), carried)
        .await
        .expect("the head bound ends it");
    assert_eq!(site.open_sockets(), 0);
    let records: Vec<Value> = records.try_iter().collect();
    let close = ws_of(&records, "close");
    assert_eq!(close.len(), 1, "{records:?}");
    assert_eq!(
        close[0]["why"],
        "the upgrade did not complete within 300 ms"
    );
}

#[tokio::test]
async fn an_upgrade_that_announces_an_empty_body_opens_its_tunnel() {
    // Content-Length: 0 is no body: never refused (#90 B3h review).
    let upstream = fake().await;
    let (site, _records) = site_with(&upstream, Duration::from_secs(5), 16);
    let at = listen(router(Arc::clone(&site), None)).await;
    let page = TcpStream::connect(at).await.unwrap();
    let url = format!("ws://{NAME}/socket.io/");
    let (mut ws, _) = socket(page, &url, &[("content-length", "0")])
        .await
        .unwrap();
    let hi = Message::text("hi");
    assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    assert_eq!(site.open_sockets(), 1);
}

#[tokio::test]
async fn an_upgrade_answered_without_a_switch_holds_its_socket_slot_while_its_body_streams() {
    let upstream = fake().await;
    let (events, records) = EventLog::channel(64);
    let proxy = Proxy::new(&upstream, Duration::from_secs(5), 16);
    let site = Arc::new(AbleSet::limited(NAME, &tls(), None, events, proxy, 4, 1));
    let upgrade = |path: &str| {
        let pair = [("connection", "upgrade"), ("upgrade", "websocket")];
        get_from(NAME, path, "10.0.0.5", &pair)
    };
    let through =
        |request: axum::http::Request<Body>| router(Arc::clone(&site), None).oneshot(request);
    // AbleSet answers 200 with a body that never ends: the one socket slot
    // is held while it streams (#90 review 2), so a second upgrade is
    // refused at once.
    let streaming = through(upgrade("/stream")).await.unwrap();
    assert_eq!(streaming.status(), StatusCode::OK);
    assert_eq!(site.open_sockets(), 1);
    let busy = through(upgrade("/stream")).await.unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = serde_json::from_str(&text_of(busy).await).unwrap();
    assert_eq!(body["code"], "ABLESET_BUSY");
    // The answer gone (the page left), its slot is free again.
    drop(streaming);
    assert_eq!(site.open_sockets(), 0);
    let seen = echo(through(upgrade("/echo")).await.unwrap()).await;
    assert_eq!(seen["upgrade"], "websocket");
    assert_eq!(site.open_sockets(), 0, "its body read, the slot is back");
    let records: Vec<Value> = records.try_iter().collect();
    let why: Vec<&Value> = records.iter().map(|r| &r["why"]).collect();
    assert_eq!(why, [&json!("every WebSocket slot for AbleSet is taken")]);
}

#[tokio::test]
async fn an_upgrade_ableset_closes_unanswered_is_no_outage_and_a_refused_one_is() {
    let pair = [("connection", "upgrade"), ("upgrade", "websocket")];
    // AbleSet reads the upgrade and closes without an answer (it serves no
    // WebSocket there): the client's fault, never an outage (#90 review 2).
    let (upstream, _) = closer().await;
    let (site, records) = site_with(&upstream, proxy::HEAD_TIMEOUT, 16);
    let closed = router(Arc::clone(&site), None)
        .oneshot(get_from(NAME, "/elsewhere", "10.0.0.5", &pair))
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_str(&text_of(closed).await).unwrap();
    assert_eq!(body["code"], "ABLESET_CLOSED");
    assert!(!site.failing.load(Ordering::Relaxed), "no outage");
    let records: Vec<Value> = records.try_iter().collect();
    assert_eq!(records.len(), 1, "{records:?}");
    let (key, why) = why_of(&records[0]);
    let text = "AbleSet closed the WebSocket request without an answer: ";
    assert!(
        key == "why" && why.starts_with(text) && why.len() > text.len(),
        "{key}: {why}"
    );
    // A connection AbleSet refuses is AbleSet down, an upgrade or not (the
    // real head bound: Windows retries a refused connect for about 1 s).
    let (site, records) = site_with(CLOSED, proxy::HEAD_TIMEOUT, 16);
    let down = router(Arc::clone(&site), None)
        .oneshot(get_from(NAME, "/socket.io/", "10.0.0.5", &pair))
        .await
        .unwrap();
    assert_eq!(down.status(), StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_str(&text_of(down).await).unwrap();
    assert_eq!(body["code"], "ABLESET_DOWN");
    assert!(site.failing.load(Ordering::Relaxed), "an outage");
    let records: Vec<Value> = records.try_iter().collect();
    assert_eq!(why_of(&records[0]).0, "error");
}

#[tokio::test]
async fn another_sites_websocket_on_ablesets_name_is_refused_before_ableset() {
    let (upstream, contacted) = closer().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    let site = Arc::clone(hub.ableset.as_ref().unwrap());
    let at = listen(crate::app_router(hub.clone())).await;
    let url = format!("ws://{NAME}:{}/socket.io/?EIO=4", at.port());
    for pairs in [
        // A browser that sends no fetch metadata with a WebSocket.
        &[("origin", "https://evil.example")][..],
        // One that does.
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "websocket"),
        ][..],
        // An Upgrade list that names websocket (#90 review 4: the Origin
        // guard reads it as the tunnel does).
        &[
            ("origin", "https://evil.example"),
            ("upgrade", "h2c, websocket"),
        ][..],
    ] {
        let page = TcpStream::connect(at).await.unwrap();
        assert_eq!(refused(socket(page, &url, pairs).await), 403, "{pairs:?}");
    }
    assert_eq!(contacted.load(Ordering::Relaxed), 0, "AbleSet was asked");
    assert_eq!(site.open_sockets(), 0);
    hub.stop();
}

#[tokio::test]
async fn an_http_1_0_upgrade_opens_no_tunnel() {
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_secs(5), 16);
    let pairs = [
        ("connection", "Upgrade"),
        ("upgrade", "websocket"),
        ("sec-websocket-key", "k"),
        ("sec-websocket-version", "13"),
    ];
    let mut ask = get_from(NAME, "/echo", "10.0.0.5", &pairs);
    *ask.version_mut() = Version::HTTP_10;
    let answer = router(Arc::clone(&site), None).oneshot(ask).await.unwrap();
    assert_eq!(answer.status(), StatusCode::OK);
    // hyper offers no upgrade on HTTP/1.0 (#90 review 5): it goes to
    // AbleSet as a plain GET, its hop-by-hop pair taken out, the rest kept.
    let seen = echo(answer).await;
    assert_eq!(
        (seen.get("connection"), seen.get("upgrade")),
        (None, None),
        "{seen}"
    );
    assert_eq!(seen["sec-websocket-key"], "k");
    let records: Vec<Value> = records.try_iter().collect();
    assert!(records.is_empty(), "{records:?}");
    assert_eq!(site.open_sockets(), 0);
}

#[tokio::test]
async fn an_internet_tunnel_opens_with_ablesets_assertion_and_ableset_sees_none_of_it() {
    let upstream = fake().await;
    let keys = KeyServer::start(jwks(&[key()])).await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, Some(&keys), &[ABLESET_AUD]);
    let mut ablesets = claims();
    ablesets["aud"] = json!([ABLESET_AUD]);
    let ablesets = sign(key(), &ablesets);
    let fohmixers = sign(key(), &claims());
    let at = listen(crate::app_router(hub.clone())).await;
    let url = format!("ws://{NAME}:{}/socket.io/?EIO=4", at.port());
    let from_cf = ("cf-connecting-ip", "203.0.113.7");
    let in_cookie = |token: &str| format!("CF_Authorization={token}; theme=dark");
    let (ablesets_cookie, fohmixers_cookie) = (in_cookie(&ablesets), in_cookie(&fohmixers));
    // Through the tunnel (cloudflared on the PC): AbleSet's assertion, in
    // the header or in Access's cookie, opens the tunnel; fohmixer's never.
    for pairs in [
        [from_cf, ("cf-access-jwt-assertion", ablesets.as_str())],
        [from_cf, ("cookie", ablesets_cookie.as_str())],
    ] {
        let page = TcpStream::connect(at).await.unwrap();
        let (mut ws, _) = socket(page, &url, &pairs).await.unwrap();
        let hi = Message::text("hi");
        assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    }
    for pairs in [
        [from_cf, ("cf-access-jwt-assertion", fohmixers.as_str())],
        [from_cf, ("cookie", fohmixers_cookie.as_str())],
    ] {
        let page = TcpStream::connect(at).await.unwrap();
        assert_eq!(refused(socket(page, &url, &pairs).await), 403);
    }
    // AbleSet's handshake carries none of it: no Cloudflare header, no
    // assertion, no Access cookie; the pair and the rest go on.
    let pairs = [
        ("connection", "Upgrade"),
        ("upgrade", "websocket"),
        ("sec-websocket-key", "k"),
        from_cf,
        ("cf-ray", "r"),
        ("cf-access-jwt-assertion", ablesets.as_str()),
        ("cookie", ablesets_cookie.as_str()),
    ];
    let seen = echo(plain(&hub, get_from(NAME, "/echo", "127.0.0.1", &pairs)).await).await;
    let names: Vec<&String> = seen.as_object().unwrap().keys().collect();
    assert!(names.iter().all(|n| !n.starts_with("cf-")), "{names:?}");
    assert_eq!(
        (
            &seen["cookie"],
            &seen["upgrade"],
            &seen["sec-websocket-key"]
        ),
        (&json!("theme=dark"), &json!("websocket"), &json!("k"))
    );
    hub.stop();
}

#[tokio::test]
async fn the_stop_closes_a_tunnel_with_its_bytes_and_one_opened_after_it_at_once() {
    let upstream = fake().await;
    let (site, records) = site_with(&upstream, Duration::from_secs(5), 16);
    let at = listen(router(Arc::clone(&site), None)).await;
    let url = format!("ws://{NAME}/socket.io/");
    let lan = || async move { TcpStream::connect(at).await.unwrap() };
    let (mut ws, _) = socket(lan().await, &url, &[]).await.unwrap();
    let hi = Message::text("hi");
    assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    site.stop();
    assert!(ends(&mut ws).await, "the stop closed the tunnel");
    tunnels_until(&site, 0).await;
    // A tunnel opened after the stop ends at once.
    let (mut late, _) = socket(lan().await, &url, &[]).await.unwrap();
    let after = Instant::now();
    assert!(ends(&mut late).await, "a tunnel after the stop was carried");
    assert!(after.elapsed() < CLOSE_BOUND, "{:?}", after.elapsed());
    tunnels_until(&site, 0).await;
    // Both closes are recorded with the bytes each way (#90 review 9): the
    // page's masked "hi" frame (8 bytes) and AbleSet's (4); none for the
    // one after the stop.
    let records: Vec<Value> = records.try_iter().collect();
    let closes: Vec<(&Value, &Value, &Value)> = ws_of(&records, "close")
        .into_iter()
        .map(|r| (&r["why"], &r["up"], &r["down"]))
        .collect();
    assert_eq!(
        closes,
        [
            (&json!(STOPPED), &json!(8), &json!(4)),
            (&json!(STOPPED), &json!(0), &json!(0))
        ]
    );
}

/// `bytes` written to `end`, then its close.
async fn send_and_close(end: &mut tokio::io::DuplexStream, bytes: &[u8]) {
    tokio::io::AsyncWriteExt::write_all(end, bytes)
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::shutdown(end).await.unwrap();
}

/// Everything `end` reads until its close.
async fn read_all(end: &mut tokio::io::DuplexStream) -> Vec<u8> {
    let mut got = Vec::new();
    end.read_to_end(&mut got).await.unwrap();
    got
}

/// A tunnel's two ends, each with its far end (what the page and AbleSet
/// hold), and a stop not sent yet.
#[allow(clippy::type_complexity)]
fn ends_and_stop() -> (
    (tokio::io::DuplexStream, tokio::io::DuplexStream),
    (tokio::io::DuplexStream, tokio::io::DuplexStream),
    (watch::Sender<bool>, watch::Receiver<bool>),
) {
    (
        tokio::io::duplex(64),
        tokio::io::duplex(64),
        watch::channel(false),
    )
}

#[test]
fn why_a_tunnel_ended_names_the_way_and_the_end_that_did_not_close() {
    assert_eq!(HALF_CLOSE_GRACE, Duration::from_secs(5));
    let grace = Duration::from_millis(5000);
    let ok = || Ok(());
    let reset = || Err(io::Error::from(io::ErrorKind::ConnectionReset));
    for first in [Way::Up, Way::Down] {
        assert_eq!(why_ended(first, &ok(), Some(&ok()), grace), "closed");
        assert_eq!(first.other().other(), first);
    }
    assert_eq!(Way::Up.other(), Way::Down);
    assert_eq!(
        why_ended(Way::Down, &ok(), None, grace),
        "the page did not close within 5000 ms of AbleSet's close"
    );
    assert_eq!(
        why_ended(Way::Up, &ok(), None, grace),
        "AbleSet did not close within 5000 ms of the page's close"
    );
    // An error is named by its way: the first one's, else the other's.
    assert_eq!(
        why_ended(Way::Up, &reset(), Some(&ok()), grace),
        "the page to AbleSet: connection reset"
    );
    assert_eq!(
        why_ended(Way::Down, &reset(), None, grace),
        "AbleSet to the page: connection reset"
    );
    assert_eq!(
        why_ended(Way::Up, &ok(), Some(&reset()), grace),
        "AbleSet to the page: connection reset"
    );
    // A source's close, a TLS page's without close_notify too (#90 review
    // 9); any other error ends its way.
    assert!(source_closed(Ok(7)).is_ok());
    assert!(source_closed(Err(io::ErrorKind::UnexpectedEof.into())).is_ok());
    let reset = source_closed(Err(io::ErrorKind::ConnectionReset.into()));
    assert_eq!(
        reset.err().map(|e| e.kind()),
        Some(io::ErrorKind::ConnectionReset)
    );
}

#[tokio::test]
async fn a_tunnels_ways_pass_their_bytes_and_their_closes_and_count_them() {
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    send_and_close(&mut page_far, b"abc").await;
    send_and_close(&mut ableset_far, b"hello").await;
    let ended = pipe(page, ableset, &mut stop, Duration::from_secs(5)).await;
    let closed = Ended {
        up: 3,
        down: 5,
        why: "closed".into(),
    };
    assert_eq!(ended, closed);
    // Each end got the other's bytes, then its close.
    assert_eq!(read_all(&mut ableset_far).await, b"abc");
    assert_eq!(read_all(&mut page_far).await, b"hello");
}

#[tokio::test]
async fn an_ends_close_reaches_the_other_at_once_and_its_answer_ends_the_tunnel() {
    // RFC 6455 7.1.1: AbleSet closes first, the page closes right after it
    // learns of that close, long before the grace.
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    send_and_close(&mut ableset_far, b"bye").await;
    let started = Instant::now();
    let piped = pipe(page, ableset, &mut stop, Duration::from_secs(5));
    let the_page = async {
        let got = read_all(&mut page_far).await;
        tokio::io::AsyncWriteExt::shutdown(&mut page_far)
            .await
            .unwrap();
        got
    };
    let (ended, got) = tokio::join!(piped, the_page);
    assert_eq!(got, b"bye");
    let closed = Ended {
        up: 0,
        down: 3,
        why: "closed".into(),
    };
    assert_eq!(ended, closed);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_ways_bytes_go_out_when_its_source_pauses() {
    // An end that buffers what it is given (as rustls does under TLS) gets
    // each burst at once: the copy flushes it when its source has nothing
    // more for now.
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    let buffering = tokio::io::BufWriter::new(page);
    let piped = pipe(buffering, ableset, &mut stop, Duration::from_secs(5));
    let the_ends = async {
        tokio::io::AsyncWriteExt::write_all(&mut ableset_far, b"hello")
            .await
            .unwrap();
        let mut got = [0_u8; 5];
        let wait = Duration::from_secs(2);
        let read = tokio::time::timeout(wait, page_far.read_exact(&mut got)).await;
        send_and_close(&mut ableset_far, b"").await;
        send_and_close(&mut page_far, b"").await;
        (read.is_ok(), got)
    };
    let (ended, (read, got)) = tokio::join!(piped, the_ends);
    assert!(read, "the burst stayed in the page end's buffer");
    assert_eq!(&got, b"hello");
    assert_eq!((ended.down, ended.why.as_str()), (5, "closed"));
}

#[tokio::test]
async fn a_tunnel_ends_after_the_grace_when_one_end_closed_and_the_other_stays_silent() {
    let grace = Duration::from_millis(100);
    // AbleSet closed; the page stays silent.
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    send_and_close(&mut ableset_far, b"").await;
    let started = Instant::now();
    let ended = pipe(page, ableset, &mut stop, grace).await;
    assert!(started.elapsed() >= grace, "{:?}", started.elapsed());
    let why = "the page did not close within 100 ms of AbleSet's close";
    assert_eq!((ended.up, ended.down, ended.why.as_str()), (0, 0, why));
    assert_eq!(
        read_all(&mut page_far).await,
        b"",
        "AbleSet's close passed on"
    );
    // The page closed; AbleSet stays silent.
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    send_and_close(&mut page_far, b"ab").await;
    let ended = pipe(page, ableset, &mut stop, grace).await;
    let why = "AbleSet did not close within 100 ms of the page's close";
    assert_eq!((ended.up, ended.down, ended.why.as_str()), (2, 0, why));
    assert_eq!(read_all(&mut ableset_far).await, b"ab");
}

#[tokio::test]
async fn a_tunnels_error_is_named_by_its_way_with_the_bytes_so_far() {
    let grace = Duration::from_secs(5);
    // AbleSet is gone while the page sends: the page's way fails first.
    let ((page, mut page_far), (ableset, ableset_far), (_sender, mut stop)) = ends_and_stop();
    drop(ableset_far);
    tokio::io::AsyncWriteExt::write_all(&mut page_far, b"abc")
        .await
        .unwrap();
    let ended = pipe(page, ableset, &mut stop, grace).await;
    let why = "the page to AbleSet: broken pipe";
    assert_eq!((ended.up, ended.down, ended.why.as_str()), (0, 0, why));
    // The page is gone while AbleSet sends: its close passed on, then
    // AbleSet's way fails.
    let ((page, page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    drop(page_far);
    tokio::io::AsyncWriteExt::write_all(&mut ableset_far, b"hello")
        .await
        .unwrap();
    let ended = pipe(page, ableset, &mut stop, grace).await;
    let why = "AbleSet to the page: broken pipe";
    assert_eq!((ended.up, ended.down, ended.why.as_str()), (0, 0, why));
    assert_eq!(read_all(&mut ableset_far).await, b"", "the page's close");
}

#[tokio::test]
async fn the_stop_shuts_both_ends_together_and_keeps_the_bytes() {
    let ((page, mut page_far), (ableset, mut ableset_far), (sender, mut stop)) = ends_and_stop();
    tokio::io::AsyncWriteExt::write_all(&mut page_far, b"abc")
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut ableset_far, b"hello")
        .await
        .unwrap();
    let piped = pipe(page, ableset, &mut stop, Duration::from_secs(5));
    let far = async {
        let mut up = [0_u8; 3];
        ableset_far.read_exact(&mut up).await.unwrap();
        let mut down = [0_u8; 5];
        page_far.read_exact(&mut down).await.unwrap();
        sender.send_replace(true);
        (
            read_all(&mut ableset_far).await,
            read_all(&mut page_far).await,
        )
    };
    let (ended, rest) = tokio::join!(piped, far);
    let stopped = Ended {
        up: 3,
        down: 5,
        why: STOPPED.into(),
    };
    assert_eq!(ended, stopped);
    assert_eq!(rest, (Vec::new(), Vec::new()), "both ends closed");
    // A page whose close cannot go out (TLS into a full send buffer) does
    // not hold AbleSet's close back (#90 review 8): both go together.
    let ((page, _page_far), (ableset, mut ableset_far), (sender, mut stop)) = ends_and_stop();
    sender.send_replace(true);
    let started = Instant::now();
    let piped = pipe(Stuck(page), ableset, &mut stop, Duration::from_secs(5));
    let far = async {
        read_all(&mut ableset_far).await;
        started.elapsed()
    };
    let (ended, closed_after) = tokio::join!(piped, far);
    assert!(closed_after < CLOSE_BOUND / 2, "{closed_after:?}");
    assert!(started.elapsed() >= CLOSE_BOUND, "{:?}", started.elapsed());
    assert_eq!(ended.why, STOPPED);
}

/// A tunnel end whose close never completes (a page's TLS `close_notify`
/// into a full send buffer).
struct Stuck(tokio::io::DuplexStream);

impl AsyncRead for Stuck {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stuck {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

#[tokio::test(start_paused = true)]
async fn a_write_that_makes_no_progress_for_the_bound_ends_the_tunnel() {
    // #90 re-review Low 1: a page that never reads (its receive window
    // zero) holds the write to it, and AbleSet's close queued behind that
    // write, for good. A write to either end that makes no progress for
    // 90 s ends the tunnel; its other way then gets the grace. On the
    // virtual clock: the real bound, without waiting for it.
    let held = Duration::from_secs(600);
    // The page takes 64 bytes and never reads again.
    let ((page, _page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    let started = tokio::time::Instant::now();
    let piped = tokio::time::timeout(held, pipe(page, ableset, &mut stop, HALF_CLOSE_GRACE));
    let pushed = tokio::io::AsyncWriteExt::write_all(&mut ableset_far, &[7; 1024]);
    let (ended, pushed) = tokio::join!(piped, pushed);
    pushed.unwrap();
    let ended = ended.expect("the tunnel was still held after 600 s");
    let why = "AbleSet to the page: a write made no progress for 90000 ms";
    assert_eq!((ended.down, ended.why.as_str()), (64, why));
    let took = started.elapsed();
    let (stall, grace) = (Duration::from_secs(90), Duration::from_secs(5));
    assert!(
        took >= stall + grace && took < stall + grace * 2,
        "{took:?}"
    );
    // The page closed, and AbleSet's end never completes the close passed
    // on to it: that write is bounded too.
    let ((page, mut page_far), (ableset, _ableset_far), (_sender, mut stop)) = ends_and_stop();
    send_and_close(&mut page_far, b"ab").await;
    let piped = pipe(page, Stuck(ableset), &mut stop, HALF_CLOSE_GRACE);
    let ended = tokio::time::timeout(held, piped)
        .await
        .expect("the tunnel was still held after 600 s");
    let why = "the page to AbleSet: a write made no progress for 90000 ms";
    assert_eq!((ended.up, ended.why.as_str()), (2, why));
}

#[tokio::test(start_paused = true)]
async fn a_flush_that_makes_no_progress_ends_the_tunnel_too() {
    // A page end that buffers what it takes (rustls under TLS) takes
    // AbleSet's bytes at once; its flush into a page that never reads is
    // bounded the same way (#90 B3h review).
    let ((page, _page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    let buffering = tokio::io::BufWriter::new(page);
    let held = Duration::from_secs(600);
    let piped = tokio::time::timeout(held, pipe(buffering, ableset, &mut stop, HALF_CLOSE_GRACE));
    let pushed = tokio::io::AsyncWriteExt::write_all(&mut ableset_far, &[7; 100]);
    let (ended, pushed) = tokio::join!(piped, pushed);
    pushed.unwrap();
    let ended = ended.expect("the tunnel was still held after 600 s");
    let why = "AbleSet to the page: a write made no progress for 90000 ms";
    assert_eq!((ended.down, ended.why.as_str()), (100, why));
}

#[tokio::test(start_paused = true)]
async fn a_slow_or_idle_tunnel_is_never_cut_by_the_stalled_write_bound() {
    assert_eq!(STALLED_WRITE, Duration::from_secs(90));
    let ((page, mut page_far), (ableset, mut ableset_far), (_sender, mut stop)) = ends_and_stop();
    let piped = pipe(page, ableset, &mut stop, HALF_CLOSE_GRACE);
    let the_ends = async {
        // Idle for twice the bound: no write waits, nothing is bounded.
        tokio::time::sleep(STALLED_WRITE * 2).await;
        // AbleSet sends 256 bytes and closes; the page takes them 16 at a
        // time, 50 s apart: each wait under the bound, all of them far over
        // it (any progress starts the bound afresh).
        send_and_close(&mut ableset_far, &[7; 256]).await;
        let mut got = Vec::new();
        let mut chunk = [0_u8; 16];
        while got.len() < 192 {
            tokio::time::sleep(Duration::from_secs(50)).await;
            page_far.read_exact(&mut chunk).await.unwrap();
            got.extend_from_slice(&chunk);
        }
        page_far.read_to_end(&mut got).await.unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut page_far)
            .await
            .unwrap();
        got.len()
    };
    let (ended, got) = tokio::join!(piped, the_ends);
    assert_eq!(got, 256);
    assert_eq!((ended.down, ended.why.as_str()), (256, "closed"));
}

#[tokio::test]
async fn a_tunnels_upgrades_are_bounded_and_end_at_the_stop() {
    let (sender, mut stop) = watch::channel(false);
    let ok = || std::future::ready(Ok::<u8, io::Error>(1));
    let never = std::future::pending::<Result<u8, io::Error>>;
    let bound = Duration::from_millis(50);
    assert_eq!(upgraded(ok(), ok(), bound, &mut stop).await, Ok((1, 1)));
    let failed = std::future::ready(Err::<u8, _>(io::Error::other("refused")));
    assert_eq!(
        upgraded(ok(), failed, bound, &mut stop).await,
        Err("the upgrade failed: refused".to_string())
    );
    let started = Instant::now();
    assert_eq!(
        upgraded(ok(), never(), bound, &mut stop).await,
        Err("the upgrade did not complete within 50 ms".to_string())
    );
    assert!(started.elapsed() >= bound);
    sender.send_replace(true);
    assert_eq!(
        upgraded(ok(), ok(), bound, &mut stop).await,
        Err(STOPPED.to_string())
    );
}

#[tokio::test]
async fn the_stop_waits_for_every_socket_slot_to_come_back() {
    let (events, _records) = EventLog::channel(4);
    let proxy = Proxy::new(CLOSED, Duration::from_secs(1), 16);
    let site = AbleSet::limited(NAME, &tls(), None, events, proxy, 1, 2);
    let short = Duration::from_millis(50);
    assert!(site.closed(short).await, "nothing open");
    let open = Arc::clone(&site.sockets).try_acquire_owned().unwrap();
    assert!(!site.closed(short).await, "a tunnel still open");
    drop(open);
    assert!(site.closed(short).await);
    assert_eq!(site.open_sockets(), 0);
    assert_eq!(CLOSED_WAIT, CLOSE_BOUND + Duration::from_millis(500));
}

#[tokio::test]
async fn a_tunnel_outlives_the_header_read_bound_on_both_listeners() {
    let upstream = fake().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    let bound = Duration::from_millis(300);
    let plain = listen_with(crate::app_router(hub.clone()), bound).await;
    let https = crate::https::Https::bind(
        "127.0.0.1:0".parse().unwrap(),
        crate::https_router(hub.clone()),
    )
    .unwrap()
    .header_read(bound);
    let ca = TestCa::new();
    let now = crate::auth::now_secs() as i64;
    for (slot, name) in [(Slot::Primary, TLS_NAME), (Slot::AbleSet, NAME)] {
        let (chain, key) = ca.leaf(&[name], now - 3600, now + 86_400);
        let pem = crate::tls::checked(chain, key, name).unwrap();
        https.set_cert(slot, name, &pem).unwrap();
    }
    // The bound is live on both listeners: a head that stops half way is
    // closed after it.
    let half = b"GET / HTTP/1.1\r\nHost: x\r\n";
    let mut lan = TcpStream::connect(plain).await.unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut lan, half)
        .await
        .unwrap();
    let mut tls = connect(https.addr(), NAME, &ca.pem, true).await.unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut tls, half)
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut tls).await.unwrap();
    let closed = async {
        let mut rest = Vec::new();
        // A TLS peer may end without close_notify.
        let _ = lan.read_to_end(&mut rest).await;
        let _ = tls.read_to_end(&mut rest).await;
    };
    tokio::time::timeout(Duration::from_secs(5), closed)
        .await
        .expect("both closed after the bound");
    // A tunnel on each, idle past the bound, still carries both ways.
    let url = format!("ws://{NAME}:{}/socket.io/?EIO=4", plain.port());
    let lan = TcpStream::connect(plain).await.unwrap();
    let (mut ws, _) = socket(lan, &url, &[]).await.unwrap();
    let tls = connect(https.addr(), NAME, &ca.pem, true).await.unwrap();
    let secure = format!("wss://{NAME}/socket.io/?EIO=4");
    let (mut wss, _) = socket(tls, &secure, &[]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let hi = Message::text("hi");
    assert_eq!(round_trip(&mut ws, hi.clone()).await, hi);
    assert_eq!(round_trip(&mut wss, hi.clone()).await, hi);
    hub.stop();
    https.stop(Duration::from_millis(10));
}

/// A fake AbleSet whose index answers 200 while `up`, else 503: its URL.
async fn switched(up: Arc<AtomicBool>) -> String {
    let index = move || {
        let up = Arc::clone(&up);
        async move {
            if up.load(Ordering::Relaxed) {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            }
        }
    };
    let at = listen(Router::new().route("/", get(index))).await;
    format!("http://{at}")
}

/// The link's next change (B3), within 5 s.
async fn change(changes: &mut tokio::sync::mpsc::UnboundedReceiver<bool>) -> Option<bool> {
    tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("a change of the link within 5 s")
}

#[tokio::test]
async fn the_link_goes_down_after_two_failed_reads_and_up_at_the_first_good_one() {
    let up = Arc::new(AtomicBool::new(true));
    let upstream = switched(Arc::clone(&up)).await;
    let (site, records) = site_with(&upstream, Duration::from_secs(1), 16);
    let (tx, mut changes) = tokio::sync::mpsc::unbounded_channel();
    // Reads every 50 ms, each bounded at 2 s: a loaded runner's slow read
    // is never a failed one (the hub bounds a read by its 5 s period).
    let period = Duration::from_millis(50);
    let reads = tokio::spawn(link::watch(
        Arc::clone(&site),
        link::client(Duration::from_secs(2)),
        format!("{upstream}/"),
        period,
        move |online| {
            let _ = tx.send(online);
        },
    ));
    let mut status = AbleSetStatus::default();
    assert_eq!(change(&mut changes).await, Some(true));
    site.fill(&mut status);
    assert!(
        status.online == Some(true) && status.last_error.is_none(),
        "{status:?}"
    );
    assert!(status.checked.is_some_and(|at| at > 1_700_000_000));
    up.store(false, Ordering::Relaxed);
    assert_eq!(change(&mut changes).await, Some(false));
    site.fill(&mut status);
    let why = Some("AbleSet answers 503");
    assert_eq!(
        (status.online, status.last_error.as_deref()),
        (Some(false), why)
    );
    up.store(true, Ordering::Relaxed);
    assert_eq!(change(&mut changes).await, Some(true));
    reads.abort();
    let links: Vec<Value> = records
        .try_iter()
        .filter(|r| r["ev"] == "ableset_link")
        .collect();
    let rows: Vec<(bool, u64, Option<&str>)> = links
        .iter()
        .map(|r| {
            let error = r.get("error").and_then(Value::as_str);
            (
                r["up"].as_bool().unwrap(),
                r["failures"].as_u64().unwrap(),
                error,
            )
        })
        .collect();
    // Down exactly at the second failed read in a row; up again with the
    // failed reads before it (more may come before the switch is read).
    assert_eq!(rows[..2], [(true, 0, None), (false, 2, why)]);
    assert_eq!((rows.len(), rows[2].0, rows[2].2), (3, true, None));
    assert!(rows[2].1 >= 2, "{rows:?}");
    assert!(links.iter().all(crate::events::is_warn), "{links:?}");
    assert_eq!(status.sockets, 0);
}

#[test]
fn the_sign_in_goes_back_to_fohmixers_name_with_the_requests_port_only() {
    for (host, target) in [
        (
            Some("ableset.example.org:8443"),
            "https://foh.example.org:8443/#ableset",
        ),
        (
            Some("ableset.example.org"),
            "https://foh.example.org/#ableset",
        ),
        (
            Some("ableset.example.org:443"),
            "https://foh.example.org/#ableset",
        ),
        // The name is never the request's.
        (
            Some("evil.example:8443"),
            "https://foh.example.org:8443/#ableset",
        ),
        (
            Some("ableset.example.org:84x"),
            "https://foh.example.org/#ableset",
        ),
        (None, "https://foh.example.org/#ableset"),
    ] {
        assert_eq!(signed_in_target(TLS_NAME, host), target, "{host:?}");
    }
    assert_eq!(routes::host_port("[::1]:8443"), Some(8443));
    assert_eq!(routes::host_port("[::1]"), None);
    assert_eq!(routes::host_port("a.example:1 x"), None);
}

#[tokio::test]
async fn the_probe_and_the_sign_in_are_the_hubs_own_and_only_after_the_access_check() {
    let keys = KeyServer::start(jwks(&[key()])).await;
    let dir = tempfile::tempdir().unwrap();
    // AbleSet is closed: anything forwarded would be a 502 and an outage.
    let hub = hub_with(dir.path(), CLOSED, Some(&keys), &[ABLESET_AUD]);
    let mut ablesets = claims();
    ablesets["aud"] = json!([ABLESET_AUD]);
    let ablesets = sign(key(), &ablesets);
    let internet = |path: &str, token: Option<&str>| {
        let mut pairs = vec![("cf-connecting-ip", "203.0.113.7")];
        pairs.extend(token.map(|t| ("cf-access-jwt-assertion", t)));
        get_from(NAME, path, "127.0.0.1", &pairs)
    };
    let lan = get_from(NAME, PROBE, "10.0.0.5", &[]);
    for request in [lan, internet(PROBE, Some(ablesets.as_str()))] {
        let response = plain(&hub, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let seen: Vec<Option<&str>> = [
            "content-type",
            "cache-control",
            "cross-origin-resource-policy",
        ]
        .iter()
        .map(|name| response.headers().get(*name).and_then(|v| v.to_str().ok()))
        .collect();
        let wanted = [Some("image/gif"), Some("no-store"), Some("same-site")];
        assert_eq!(seen, wanted);
        let body = axum::body::to_bytes(response.into_body(), 1024).await;
        assert_eq!(&body.unwrap()[..], PIXEL);
    }
    assert_eq!((PIXEL.len(), &PIXEL[..6]), (43, &b"GIF89a"[..]));
    // AbleSet's sign-in lands on its name and goes back to fohmixer's.
    let lan = get_from("AbleSet.example.org:8443", SIGNED_IN, "10.0.0.5", &[]);
    let back = |response: &Response| {
        let location = response.headers().get(header::LOCATION);
        (
            response.status(),
            location.map(|v| v.to_str().unwrap().to_string()),
        )
    };
    let response = plain(&hub, lan).await;
    let target = Some("https://foh.example.org:8443/#ableset".to_string());
    assert_eq!(back(&response), (StatusCode::SEE_OTHER, target));
    let response = plain(&hub, internet(SIGNED_IN, Some(ablesets.as_str()))).await;
    let target = Some("https://foh.example.org/#ableset".to_string());
    assert_eq!(back(&response), (StatusCode::SEE_OTHER, target));
    // From the internet without AbleSet's assertion: Access refuses both.
    for path in [PROBE, SIGNED_IN] {
        let refused = plain(&hub, internet(path, None)).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{path}");
    }
    let site = hub.ableset.as_ref().unwrap();
    assert!(!site.failing.load(Ordering::Relaxed), "AbleSet was asked");
    hub.stop();
}

#[test]
fn a_clients_text_is_clipped_at_a_character_never_inside_one() {
    // #90 B3 (the B3h review's minor item): 256 characters, not bytes.
    let two = "ž".repeat(CLIENT_TEXT);
    assert_eq!(
        clip(&two),
        two.as_str(),
        "256 two-byte characters stay whole"
    );
    let longer = format!("{two}ž");
    assert_eq!(clip(&longer), format!("{two}… ({} bytes)", 2 * 257));
    // A four-byte character over the 256th byte stays whole.
    let slider = "\u{1F39A}";
    let mixed = format!("{}{}", "a".repeat(255), slider.repeat(3));
    let kept = format!("{}{slider}… (267 bytes)", "a".repeat(255));
    assert_eq!(clip(&mixed), kept);
}

#[test]
fn an_outage_that_starts_and_ends_in_a_loop_writes_its_start_and_end_rationed() {
    // #90 B3 (the B3h review's minor item): a client that alternates a
    // request AbleSet drops with one it answers starts and ends an outage
    // each time. Its warned start goes out, then at most 8 starts and 8
    // ends a gap, in their order; the rest are left out.
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let refused = "AbleSet does not answer: refused";
    for _ in 0..40 {
        site.failed(&Method::GET, "/drop", None, Failure::Down(refused.into()));
        site.reached(&Method::GET, "/", StatusCode::OK, Instant::now());
    }
    let records: Vec<Value> = records.try_iter().collect();
    let rows: Vec<(&str, &str)> = records.iter().map(why_of).collect();
    let mut expected = vec![("error", refused), ("why", ANSWERS_AGAIN)];
    for _ in 0..7 {
        expected.extend([("why", refused), ("why", ANSWERS_AGAIN)]);
    }
    expected.push(("why", refused));
    assert_eq!(rows, expected);
}

#[test]
fn an_outages_start_and_end_carry_what_their_rations_left_out() {
    // #90 B3: after the gap, the next start counts the starts left out,
    // the next end the ends and the outage records left out.
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let refused = "AbleSet does not answer: refused";
    for _ in 0..40 {
        site.failed(&Method::GET, "/drop", None, Failure::Down(refused.into()));
        site.reached(&Method::GET, "/", StatusCode::OK, Instant::now());
    }
    assert_eq!(records.try_iter().count(), 17);
    // The gap goes by.
    for ration in site.rations.lock().unwrap().values_mut() {
        ration.expire();
    }
    // A start (31 left out: the 10th to the 40th), 9 more of the outage (8
    // allowed, 1 left out), then its end (32 ends left out: the 9th to the
    // 40th, and the 1).
    for _ in 0..10 {
        site.failed(&Method::GET, "/drop", None, Failure::Down(refused.into()));
    }
    site.reached(&Method::GET, "/", StatusCode::OK, Instant::now());
    let records: Vec<Value> = records.try_iter().collect();
    let skipped: Vec<u64> = records
        .iter()
        .map(|r| r["skipped"].as_u64().unwrap())
        .collect();
    let mut expected = vec![31];
    expected.extend([0; 8]);
    expected.push(33);
    assert_eq!(skipped, expected);
    assert_eq!(why_of(&records[9]).1, ANSWERS_AGAIN);
    assert_eq!(EDGE_RECORDS, 8);
}

/// A fake AbleSet that keeps every connection open, as a keep-alive server
/// does, and answers each request `ok`: its URL, the connections it took and
/// the request heads it read (lowercase).
async fn counting() -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let taken = Arc::new(AtomicUsize::new(0));
    let heads = Arc::new(Mutex::new(Vec::new()));
    let (count, seen) = (Arc::clone(&taken), Arc::clone(&heads));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::Relaxed);
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 1024]);
                loop {
                    while let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head: Vec<u8> = buffer.drain(..end + 4).collect();
                        let head = String::from_utf8_lossy(&head).to_lowercase();
                        seen.lock().unwrap().push(head);
                        let ok = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";
                        let written = tokio::io::AsyncWriteExt::write_all(&mut stream, ok);
                        if written.await.is_err() {
                            return;
                        }
                    }
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
            });
        }
    });
    (format!("http://{addr}"), taken, heads)
}

#[tokio::test]
async fn the_link_reads_on_a_fresh_connection_each_time_and_takes_any_answer() {
    // #90 B3 review 1: AbleSet's Node server closes a keep-alive connection
    // idle for 5 s, the link's period, so a pooled connection would be
    // used again just as AbleSet closes it.
    let (upstream, accepted, heads) = counting().await;
    let client = link::client(Duration::from_secs(2));
    let url = format!("{upstream}/");
    for _ in 0..3 {
        assert_eq!(link::read(&client, &url).await, Ok(()));
        // A pool would have the connection back by now.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(accepted.load(Ordering::Relaxed), 3, "a connection per read");
    let heads = heads.lock().unwrap();
    assert_eq!(heads.len(), 3);
    assert!(
        heads
            .iter()
            .all(|head| head.contains("\r\naccept: */*\r\n")),
        "{heads:?}"
    );
}

#[tokio::test]
async fn the_proxy_takes_a_fresh_connection_for_every_request() {
    // #90: Node closes a keep-alive connection idle for 5 s, counted from its
    // last write; a request the proxy writes on a connection AbleSet is
    // closing just then fails and reads as AbleSet down. No connection is
    // kept idle, so none is used again, as the link's reads.
    let (upstream, accepted, _) = counting().await;
    let proxy = Proxy::new(&upstream, proxy::HEAD_TIMEOUT, proxy::MAX_BODY);
    for _ in 0..3 {
        let request = axum::http::Request::builder()
            .uri("/")
            .header("host", NAME)
            .body(Body::empty())
            .unwrap();
        let answer = proxy.send(request).await.unwrap();
        let body = axum::body::to_bytes(answer.into_body(), 1024).await;
        assert_eq!(body.unwrap(), "ok");
        // A pool would have the connection back by now.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        accepted.load(Ordering::Relaxed),
        3,
        "a connection per request"
    );
}

#[test]
fn a_sign_in_in_a_loop_writes_its_info_line_rationed() {
    // #90 B3 review 2: a client, or another site's image, asking in a loop;
    // hub.out.log has no cap.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = logged(|| {
        runtime.block_on(async {
            let hub = hub_with(dir.path(), CLOSED, None, &[]);
            for _ in 0..12 {
                let request = get_from(NAME, SIGNED_IN, "10.0.0.5", &[]);
                let answer = plain(&hub, request).await;
                assert_eq!(answer.status(), StatusCode::SEE_OTHER);
            }
            hub.stop();
        });
    });
    let lines = log
        .lines()
        .filter(|line| line.contains(" INFO ") && line.contains("signed in through Access"));
    assert_eq!(lines.count(), 8, "{log}");
}

#[test]
fn the_sign_in_line_names_its_opener_and_carries_the_count_left_out() {
    let (site, _) = site_with(CLOSED, Duration::from_secs(1), 16);
    let lan = crate::ws::opener(SocketAddr::from(([10, 0, 0, 5], 50000)), &HeaderMap::new());
    let log = logged(|| {
        for _ in 0..10 {
            site.signed_in_line(&lan);
        }
        // The gap goes by.
        for ration in site.rations.lock().unwrap().values_mut() {
            ration.expire();
        }
        site.signed_in_line(&lan);
    });
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(" INFO ") && line.contains("signed in through Access"))
        .collect();
    assert_eq!(lines.len(), 9, "{log}");
    for line in &lines[..8] {
        assert!(line.contains("skipped=0"), "{line}");
    }
    assert!(lines[8].contains("skipped=2"), "{log}");
    let named = ["peer=10.0.0.5", "forwarded=\"-\"", "source=\"lan\""];
    assert!(named.iter().all(|n| lines[0].contains(n)), "{}", lines[0]);
    assert_eq!(SIGN_IN_LINES, 8);
}

#[tokio::test]
async fn the_links_read_names_the_hub_and_asks_to_close_its_connection() {
    let (upstream, _, heads) = counting().await;
    let client = link::client(Duration::from_secs(2));
    let read = link::read(&client, &format!("{upstream}/")).await;
    assert_eq!(read, Ok(()));
    let head = heads.lock().unwrap()[0].clone();
    let agent = format!(
        "\r\nuser-agent: fohmixer-hub/{}\r\n",
        fohmixer_proto::VERSION
    );
    assert!(head.starts_with("get / http/1.1\r\n"), "{head}");
    assert!(
        head.contains(&agent) && head.contains("\r\nconnection: close\r\n"),
        "{head}"
    );
}

/// Through the HTTPS listener's application.
async fn secure(hub: &Hub, request: axum::http::Request<Body>) -> Response {
    crate::https_router(hub.clone())
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_hubs_own_routes_answer_on_the_https_listener_too_and_take_no_other_method() {
    let dir = tempfile::tempdir().unwrap();
    // AbleSet is closed: anything forwarded would be a 502 and an outage.
    let hub = hub_with(dir.path(), CLOSED, None, &[]);
    let probe = secure(&hub, get_from(NAME, PROBE, "10.0.0.5", &[])).await;
    let kind = probe.headers().get(header::CONTENT_TYPE).cloned();
    assert_eq!(
        (probe.status(), kind),
        (StatusCode::OK, Some(HeaderValue::from_static("image/gif")))
    );
    let host = "ableset.example.org:8443";
    let back = secure(&hub, get_from(host, SIGNED_IN, "10.0.0.5", &[])).await;
    let location = back.headers().get(header::LOCATION).cloned();
    let target = HeaderValue::from_static("https://foh.example.org:8443/#ableset");
    assert_eq!(
        (back.status(), location),
        (StatusCode::SEE_OTHER, Some(target))
    );
    // Another method: after the Access check a 405, never forwarded.
    let post = |path: &str| request(Method::POST, NAME, path, "10.0.0.5", &[], Body::empty());
    for path in [PROBE, SIGNED_IN] {
        let answers = [
            plain(&hub, post(path)).await,
            secure(&hub, post(path)).await,
        ];
        for answer in answers {
            assert_eq!(answer.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        }
    }
    let site = hub.ableset.as_ref().unwrap();
    assert!(!site.failing.load(Ordering::Relaxed), "AbleSet was asked");
    hub.stop();
}

#[tokio::test]
async fn the_link_stops_with_the_hub() {
    let (upstream, accepted, _) = counting().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), &upstream, None, &[]);
    crate::start_link(&hub, Duration::from_millis(50));
    let deadline = Instant::now() + Duration::from_secs(5);
    while accepted.load(Ordering::Relaxed) < 2 {
        assert!(Instant::now() < deadline, "no reads");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    hub.stop();
    // A read already on its way may still land.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stopped = accepted.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        accepted.load(Ordering::Relaxed),
        stopped,
        "a read after the stop"
    );
}

#[tokio::test]
async fn the_locked_page_runs_only_its_own_script_is_never_stored_and_never_asks_ableset() {
    // #90, the owner's report of 2026-10-14: the tab's frame opens AbleSet
    // through this page, which stores AbleSet's lock and then opens
    // AbleSet's own page.
    let dir = tempfile::tempdir().unwrap();
    // AbleSet is closed: anything forwarded would be a 502 and an outage.
    let hub = hub_with(dir.path(), CLOSED, None, &[]);
    let named = |response: &Response, names: &[&str]| -> Vec<Option<String>> {
        names
            .iter()
            .map(|name| {
                let value = response.headers().get(*name);
                value.map(|v| v.to_str().unwrap().to_string())
            })
            .collect()
    };
    for https in [false, true] {
        let page = get_from(NAME, LOCKED, "10.0.0.5", &[]);
        let page = if https {
            secure(&hub, page).await
        } else {
            plain(&hub, page).await
        };
        assert_eq!(page.status(), StatusCode::OK, "https {https}");
        let seen = named(&page, &["content-type", "cache-control", "x-frame-options"]);
        let wanted = [
            Some("text/html; charset=utf-8".to_string()),
            Some("no-store".to_string()),
            None,
        ];
        assert_eq!(seen, wanted, "https {https}");
        // Its own policy, and fohmixer's framing beside it.
        assert_eq!(policies(page.headers()), [LOCK_POLICY, OURS]);
        assert_eq!(text_of(page).await, LOCK_PAGE);
        let script = get_from(NAME, LOCK_SCRIPT, "10.0.0.5", &[]);
        let script = if https {
            secure(&hub, script).await
        } else {
            plain(&hub, script).await
        };
        assert_eq!(script.status(), StatusCode::OK, "https {https}");
        let names = [
            "content-type",
            "cache-control",
            "cross-origin-resource-policy",
            "x-content-type-options",
        ];
        let wanted = [
            Some("text/javascript; charset=utf-8".to_string()),
            Some("no-store".to_string()),
            Some("same-origin".to_string()),
            Some("nosniff".to_string()),
        ];
        assert_eq!(named(&script, &names), wanted, "https {https}");
        assert_eq!(text_of(script).await, LOCK_JS);
    }
    assert_eq!(LOCK_POLICY, "default-src 'none'; script-src 'self'");
    let tag = format!("<script src=\"{LOCK_SCRIPT}\"></script>");
    assert!(LOCK_PAGE.contains(&tag), "{LOCK_PAGE}");
    // The script stores AbleSet's lock as AbleSet reads it, then opens
    // AbleSet's own page in its place; storage refused, through the
    // record's address.
    assert_eq!(LOCK_KEY, "AbleSetLocked");
    for part in [
        format!("localStorage.setItem(\"{LOCK_KEY}\", \"true\");"),
        format!("location.replace(failed === null ? \"/\" : \"{LOCKED}?failed=\""),
        "encodeURIComponent(failed)".to_string(),
    ] {
        assert!(LOCK_JS.contains(&part), "{part}: {LOCK_JS}");
    }
    // Another method: after the Access check a 405, never forwarded.
    let post = |path: &str| request(Method::POST, NAME, path, "10.0.0.5", &[], Body::empty());
    for path in [LOCKED, LOCK_SCRIPT] {
        let answers = [
            plain(&hub, post(path)).await,
            secure(&hub, post(path)).await,
        ];
        for answer in answers {
            assert_eq!(answer.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        }
    }
    let site = hub.ableset.as_ref().unwrap();
    assert!(!site.failing.load(Ordering::Relaxed), "AbleSet was asked");
    hub.stop();
}

#[tokio::test]
async fn the_locked_page_and_its_script_are_only_after_the_access_check() {
    let keys = KeyServer::start(jwks(&[key()])).await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with(dir.path(), CLOSED, Some(&keys), &[ABLESET_AUD]);
    let mut ablesets = claims();
    ablesets["aud"] = json!([ABLESET_AUD]);
    let ablesets = sign(key(), &ablesets);
    let fohmixers = sign(key(), &claims());
    let internet = |path: &str, token: Option<&str>| {
        let mut pairs = vec![("cf-connecting-ip", "203.0.113.7")];
        pairs.extend(token.map(|t| ("cf-access-jwt-assertion", t)));
        get_from(NAME, path, "127.0.0.1", &pairs)
    };
    for path in [LOCKED, LOCK_SCRIPT] {
        // From the internet only with AbleSet's own assertion.
        for token in [None, Some(fohmixers.as_str())] {
            let refused = plain(&hub, internet(path, token)).await;
            assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{path} {token:?}");
        }
        let allowed = plain(&hub, internet(path, Some(ablesets.as_str()))).await;
        assert_eq!(allowed.status(), StatusCode::OK, "{path}");
        // The LAN is never authenticated.
        let lan = plain(&hub, get_from(NAME, path, "10.0.0.5", &[])).await;
        assert_eq!(lan.status(), StatusCode::OK, "{path}");
    }
    // The page's word that storage was refused, too.
    let failed = format!("{LOCKED}?failed=SecurityError");
    let refused = plain(&hub, internet(&failed, None)).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    hub.stop();
}

#[tokio::test]
async fn a_lock_the_page_could_not_store_is_recorded_and_ableset_opens_all_the_same() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let app = router(Arc::clone(&site), None);
    let path = format!("{LOCKED}?failed=SecurityError");
    let answer = app
        .clone()
        .oneshot(get_from(NAME, &path, "10.0.0.5", &[]))
        .await
        .unwrap();
    // A 303 to AbleSet's own page, under fohmixer's framing.
    let location = answer.headers().get(header::LOCATION).cloned();
    assert_eq!(
        (answer.status(), location),
        (StatusCode::SEE_OTHER, Some(HeaderValue::from_static("/")))
    );
    assert_eq!(policies(answer.headers()), [OURS]);
    let written: Vec<Value> = records.try_iter().collect();
    assert_eq!(written.len(), 1, "{written:?}");
    let record = &written[0];
    assert_eq!(record["ev"], "ableset_lock");
    let fields = ["why", "peer", "forwarded", "source", "skipped"].map(|k| record[k].clone());
    let wanted = [
        json!("SecurityError"),
        json!("10.0.0.5"),
        json!("-"),
        json!("lan"),
        json!(0),
    ];
    assert_eq!(fields, wanted);
    // The page's own word: never a warn-class record.
    assert!(record.get("error").is_none(), "{record}");
    assert!(!crate::events::is_warn(record), "{record}");
    // The page itself writes no record.
    let page = app
        .oneshot(get_from(NAME, LOCKED, "10.0.0.5", &[]))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(records.try_iter().count(), 0);
    assert!(!site.failing.load(Ordering::Relaxed), "AbleSet was asked");
}

#[test]
fn the_pages_word_of_a_refused_lock_is_its_failed_value_clipped() {
    for (query, why) in [
        (None, None),
        (Some(""), None),
        (Some("x=1"), None),
        (Some("notfailed=1"), None),
        (Some("failed"), None),
        (Some("failed=SecurityError"), Some("SecurityError")),
        (
            Some("a=1&failed=Quota%20Exceeded&b=2"),
            Some("Quota%20Exceeded"),
        ),
        (Some("failed="), Some("")),
    ] {
        assert_eq!(lock_failure(query).as_deref(), why, "{query:?}");
    }
    let long = format!("failed={}", "a".repeat(300));
    let clipped = format!("{}… (300 bytes)", "a".repeat(CLIENT_TEXT));
    assert_eq!(lock_failure(Some(&long)), Some(clipped));
}

#[test]
fn a_refused_locks_record_names_its_opener() {
    let peer = SocketAddr::from(([127, 0, 0, 1], 50000));
    let internet = crate::ws::opener(peer, &headers(&[("cf-connecting-ip", "203.0.113.7")]));
    assert_eq!(
        lock_fields(&internet, "QuotaExceededError", 3),
        json!({
            "why": "QuotaExceededError", "peer": "127.0.0.1",
            "forwarded": "203.0.113.7", "source": "internet", "skipped": 3
        })
    );
}

#[test]
fn a_refused_lock_in_a_loop_is_recorded_and_warned_rationed() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let lan = crate::ws::opener(SocketAddr::from(([10, 0, 0, 5], 50000)), &HeaderMap::new());
    let log = logged(|| {
        for _ in 0..12 {
            site.lock_failed(&lan, "SecurityError");
        }
        // The gap goes by.
        for ration in site.rations.lock().unwrap().values_mut() {
            ration.expire();
        }
        site.lock_failed(&lan, "SecurityError");
    });
    let skipped: Vec<u64> = records
        .try_iter()
        .map(|r| r["skipped"].as_u64().unwrap())
        .collect();
    let mut expected = vec![0; 8];
    expected.push(4);
    assert_eq!(skipped, expected);
    let warned: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(" WARN ") && line.contains("lock not stored"))
        .collect();
    assert_eq!(warned.len(), 9, "{log}");
    let named = [
        "peer=10.0.0.5",
        "forwarded=\"-\"",
        "source=\"lan\"",
        "why=\"SecurityError\"",
        "skipped=0",
    ];
    assert!(named.iter().all(|n| warned[0].contains(n)), "{}", warned[0]);
    assert!(warned[8].contains("skipped=4"), "{}", warned[8]);
    assert_eq!(LOCK_RECORDS, 8);
}

#[test]
fn the_locked_pages_line_names_its_opener_rationed_with_the_count_left_out() {
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let lan = crate::ws::opener(SocketAddr::from(([10, 0, 0, 5], 50000)), &HeaderMap::new());
    let log = logged(|| {
        for _ in 0..10 {
            site.locked_line(&lan);
        }
        // The gap goes by.
        for ration in site.rations.lock().unwrap().values_mut() {
            ration.expire();
        }
        site.locked_line(&lan);
    });
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(" INFO ") && line.contains("opened locked"))
        .collect();
    assert_eq!(lines.len(), 9, "{log}");
    for line in &lines[..8] {
        assert!(line.contains("skipped=0"), "{line}");
    }
    assert!(lines[8].contains("skipped=2"), "{log}");
    let named = ["peer=10.0.0.5", "forwarded=\"-\"", "source=\"lan\""];
    assert!(named.iter().all(|n| lines[0].contains(n)), "{}", lines[0]);
    // A line, never a record.
    assert_eq!(records.try_iter().count(), 0);
    assert_eq!(LOCK_LINES, 8);
}

#[test]
fn the_locked_page_is_dark_and_goes_on_to_ableset_without_its_script() {
    // #90 review M1 and M3: a refresh needs no script, and the page's
    // scheme is fohmixer's (no white canvas at every open).
    let refresh = format!("<meta http-equiv=\"refresh\" content=\"3;url={LOCKED}?failed=script\">");
    let script = format!("<script src=\"{LOCK_SCRIPT}\"></script>");
    let wanted = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
         <meta name=\"color-scheme\" content=\"dark\">{refresh}<title>AbleSet</title>{script}</html>"
    );
    assert_eq!(LOCK_PAGE, wanted);
    // The refresh's word is the hub's to read.
    let query = "failed=script";
    assert_eq!(lock_failure(Some(query)).as_deref(), Some("script"));
}

#[test]
fn only_the_lock_pages_own_navigation_is_its_word() {
    // #90 review M2: by the fetch metadata `refusal` reads; a browser that
    // sends none passes.
    let navigate = ("sec-fetch-mode", "navigate");
    let cases: &[(&[(&str, &str)], bool)] = &[
        (&[], true),
        (&[("sec-fetch-site", "same-origin"), navigate], true),
        (
            &[
                ("sec-fetch-site", "Same-Origin"),
                ("sec-fetch-mode", "Navigate"),
            ],
            true,
        ),
        (&[("sec-fetch-site", "same-origin")], true),
        (&[navigate], true),
        // Another site's image.
        (
            &[
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "no-cors"),
            ],
            false,
        ),
        // Another site's navigation, and fohmixer's own (same-site) page.
        (&[("sec-fetch-site", "cross-site"), navigate], false),
        (&[("sec-fetch-site", "same-site"), navigate], false),
        (&[("sec-fetch-site", "none")], false),
        // The page's own origin, but no navigation.
        (
            &[
                ("sec-fetch-site", "same-origin"),
                ("sec-fetch-mode", "no-cors"),
            ],
            false,
        ),
        (&[("sec-fetch-mode", "cors")], false),
    ];
    for (pairs, own) in cases {
        assert_eq!(own_navigation(&headers(pairs)), *own, "{pairs:?}");
    }
}

#[test]
fn a_lock_failure_not_of_the_lock_pages_own_navigation_is_never_recorded() {
    // #90 review M2: on the LAN anyone may ask, another site's image too;
    // it still goes on to AbleSet, with a debug line only.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (site, records) = site_with(CLOSED, Duration::from_secs(1), 16);
    let app = router(Arc::clone(&site), None);
    let ask = |pairs: &[(&str, &str)]| {
        let path = format!("{LOCKED}?failed=forged");
        let request = get_from(NAME, &path, "10.0.0.5", pairs);
        let answer = runtime.block_on(app.clone().oneshot(request)).unwrap();
        let location = answer.headers().get(header::LOCATION).cloned();
        assert_eq!(
            (answer.status(), location),
            (StatusCode::SEE_OTHER, Some(HeaderValue::from_static("/"))),
            "{pairs:?}"
        );
    };
    let log = logged(|| {
        ask(&[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "no-cors"),
            ("sec-fetch-dest", "image"),
        ]);
        ask(&[
            ("sec-fetch-site", "same-site"),
            ("sec-fetch-mode", "navigate"),
        ]);
    });
    assert_eq!(records.try_iter().count(), 0);
    assert!(!log.contains(" WARN "), "{log}");
    // The page's own navigation (its refresh, or its script's replace) is.
    let log = logged(|| {
        ask(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "iframe"),
        ]);
    });
    let written: Vec<Value> = records.try_iter().collect();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0]["why"], "forged");
    assert!(log.contains(" WARN "), "{log}");
}
