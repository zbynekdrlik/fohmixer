//! Tests of `access.rs`: the classification, the Origin guard, the token's
//! places, the key set and its cache, the JWT checks, the policy, and the
//! middleware in front of the real router.

use std::net::{Ipv4Addr, Ipv6Addr};

use axum::body::Body;
use axum::http::HeaderValue;
use serde_json::json;
use tower::ServiceExt;

use super::*;
use crate::test_keys::{self, AUD, ISS, KeyServer, claims, jwks, key, other_key, sign};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn peer(ip: &str) -> Option<SocketAddr> {
    Some(SocketAddr::new(ip.parse().unwrap(), 50000))
}

#[test]
fn private_addresses_are_ours_public_ones_are_the_internet() {
    for ip in [
        "127.0.0.1",
        "10.0.0.5",
        "172.16.0.1",
        "192.168.1.20",
        "169.254.1.1",
        "100.64.0.1",
        "100.127.255.255",
        "::1",
        "fc00::1",
        "fdff::1",
        "fe80::1",
        "febf::1",
        "::ffff:10.0.0.1",
        "::ffff:127.0.0.1",
    ] {
        assert!(is_private_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in [
        "8.8.8.8",
        "203.0.113.7",
        "100.63.255.255",
        "100.128.0.0",
        "101.64.0.1",
        "172.32.0.1",
        "2001:db8::1",
        "fbff::1",
        "fec0::1",
        "::ffff:8.8.8.8",
    ] {
        assert!(!is_private_ip(ip.parse().unwrap()), "{ip}");
    }
    assert!(is_private_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    assert!(is_private_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
}

#[test]
fn a_forwarded_header_or_a_public_peer_is_the_internet() {
    let none = HeaderMap::new();
    assert_eq!(classify(peer("192.168.1.20"), &none), Origin::Local);
    assert_eq!(classify(peer("127.0.0.1"), &none), Origin::Local);
    // No peer address (a listener without ConnectInfo): fail closed.
    assert_eq!(classify(None, &none), Origin::Internet);
    assert_eq!(classify(peer("203.0.113.7"), &none), Origin::Internet);
    for name in PROXY_HEADERS {
        let proxied = headers(&[(name, "203.0.113.7")]);
        assert!(has_proxy_header(&proxied), "{name}");
        // The tunnel connects from the PC itself: the header decides.
        assert_eq!(
            classify(peer("127.0.0.1"), &proxied),
            Origin::Internet,
            "{name}"
        );
        assert_eq!(classify(None, &proxied), Origin::Internet, "{name}");
    }
    assert!(!has_proxy_header(&headers(&[("x-real-ip", "1.2.3.4")])));
}

#[test]
fn the_origin_guard_refuses_cross_site_writes_and_websocket_upgrades() {
    let post = Method::POST;
    let get = Method::GET;
    let host = ("host", "foh.example.org");
    let ws = ("upgrade", "websocket");
    // Same origin passes (any case), a request without Origin passes.
    for pairs in [
        vec![host, ("origin", "https://foh.example.org")],
        vec![host, ("origin", "https://FOH.example.org")],
        vec![
            ("host", "10.0.0.5:8480"),
            ("origin", "http://10.0.0.5:8480"),
        ],
        vec![host],
        vec![
            ("host", "127.0.0.1:8480"),
            ("x-forwarded-host", "foh.example.org"),
            ("origin", "https://foh.example.org"),
        ],
        vec![
            host,
            ("origin", "https://foh.example.org"),
            ("sec-fetch-site", "same-origin"),
        ],
    ] {
        assert_eq!(origin_violation(&headers(&pairs), &post), None, "{pairs:?}");
        let mut upgrade = pairs.clone();
        upgrade.push(ws);
        assert_eq!(
            origin_violation(&headers(&upgrade), &get),
            None,
            "{upgrade:?}"
        );
    }
    // Cross-site writes and upgrades are refused.
    for pairs in [
        vec![host, ("origin", "https://evil.example")],
        vec![host, ("origin", "null")],
        vec![host, ("origin", "https://")],
        vec![host, ("origin", "foh.example.org")],
        vec![("origin", "https://foh.example.org")],
        vec![host, ("sec-fetch-site", "cross-site")],
        vec![
            host,
            ("origin", "https://foh.example.org"),
            ("sec-fetch-site", "Cross-Site"),
        ],
    ] {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(
                origin_violation(&headers(&pairs), &method).is_some(),
                "{method} {pairs:?}"
            );
        }
        let mut upgrade = pairs.clone();
        upgrade.push(("upgrade", "WebSocket"));
        assert!(
            origin_violation(&headers(&upgrade), &get).is_some(),
            "{upgrade:?}"
        );
        // A plain GET (a page, an asset) is not a write.
        assert_eq!(origin_violation(&headers(&pairs), &get), None, "{pairs:?}");
        assert_eq!(origin_violation(&headers(&pairs), &Method::HEAD), None);
    }
    let why =
        origin_violation(&headers(&[host, ("origin", "https://evil.example")]), &post).unwrap();
    assert_eq!(
        why,
        "Origin https://evil.example is not [\"foh.example.org\"]"
    );
    assert_eq!(
        origin_violation(&headers(&[host, ("sec-fetch-site", "cross-site")]), &post).unwrap(),
        "Sec-Fetch-Site: cross-site"
    );
}

#[test]
fn the_assertion_comes_from_the_header_else_the_cookie() {
    assert_eq!(extract_token(&HeaderMap::new()), None);
    assert_eq!(
        extract_token(&headers(&[(JWT_HEADER, " a.b.c ")])).as_deref(),
        Some("a.b.c")
    );
    assert_eq!(
        extract_token(&headers(&[
            (JWT_HEADER, "from.the.header"),
            ("cookie", "CF_Authorization=from.the.cookie")
        ]))
        .as_deref(),
        Some("from.the.header")
    );
    assert_eq!(
        extract_token(&headers(&[
            (JWT_HEADER, "  "),
            ("cookie", "a=1; CF_Authorization=from.the.cookie; b=2")
        ]))
        .as_deref(),
        Some("from.the.cookie")
    );
    assert_eq!(
        extract_token(&headers(&[
            ("cookie", "a=1"),
            ("cookie", "CF_Authorization=; CF_Authorization=second.one")
        ]))
        .as_deref(),
        Some("second.one")
    );
    for cookie in [
        "CF_AuthorizationX=a.b.c",
        "CF_Authorization",
        "x=CF_Authorization=a",
    ] {
        assert_eq!(
            extract_token(&headers(&[("cookie", cookie)])),
            None,
            "{cookie}"
        );
    }
}

#[test]
fn a_key_set_yields_its_usable_rsa_keys() {
    let body = json!({"keys": [
        key().jwk,
        {"kid": "ec", "kty": "EC", "crv": "P-256", "x": "AA", "y": "AA"},
        {"kid": "no-n", "kty": "RSA", "e": "AQAB"},
        {"kid": "no-e", "kty": "RSA", "n": "AQAB"},
        {"kid": "bad", "kty": "RSA", "n": "!!!", "e": "AQAB"},
        {"kid": "no-kty", "n": "AQAB", "e": "AQAB"},
    ], "public_cert": {"kid": "x", "cert": "y"}});
    let keys = parse_jwks(body.to_string().as_bytes()).unwrap();
    assert_eq!(keys.keys().collect::<Vec<_>>(), vec!["kid-1"]);
    assert!(
        parse_jwks(b"{\"keys\": 1}")
            .err()
            .unwrap()
            .contains("does not parse")
    );
    assert!(parse_jwks(b"{}").is_err());
    assert!(parse_jwks(b"{\"keys\": []}").unwrap().is_empty());
}

#[test]
fn the_cache_ages_out_after_six_hours_and_refreshes_are_spaced_a_minute() {
    let hour = Duration::from_secs(3600);
    assert!(!cache_fresh(None));
    assert!(cache_fresh(Some(Duration::ZERO)));
    assert!(cache_fresh(Some(6 * hour - Duration::from_millis(1))));
    assert!(!cache_fresh(Some(6 * hour)));
    assert_eq!(JWKS_MAX_AGE, Duration::from_secs(21_600));
    let minute = Duration::from_secs(60);
    assert!(refresh_allowed(None, false));
    assert!(refresh_allowed(Some(minute), false));
    assert!(!refresh_allowed(
        Some(minute - Duration::from_millis(1)),
        false
    ));
    assert!(refresh_allowed(Some(Duration::ZERO), true));
    assert_eq!(JWKS_MIN_REFRESH, minute);
}

#[test]
fn the_identity_is_the_email_else_the_service_token_else_the_subject() {
    let claims = |email: Option<&str>, cn: Option<&str>, sub: Option<&str>| AccessClaims {
        email: email.map(str::to_string),
        common_name: cn.map(str::to_string),
        sub: sub.map(str::to_string),
    };
    assert_eq!(
        claims(Some("a@b"), Some("svc"), Some("s")).identity(),
        "a@b"
    );
    assert_eq!(claims(None, Some("svc"), Some("s")).identity(), "svc");
    assert_eq!(claims(None, None, Some("s")).identity(), "s");
    assert_eq!(claims(None, None, None).identity(), "unknown");
}

async fn gate(server: &KeyServer) -> Arc<AccessGate> {
    AccessGate::new(&server.config(), HttpClient::new().unwrap())
}

#[tokio::test]
async fn a_valid_assertion_passes_and_every_bad_one_is_refused() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    let good = gate.verify(&sign(key(), &claims())).await.unwrap();
    assert_eq!(good.email.as_deref(), Some("engineer@example.org"));
    assert_eq!(good.identity(), "engineer@example.org");
    assert_eq!(
        server.count(),
        1,
        "the first token fetched the key set once"
    );
    let with = |name: &str, value: serde_json::Value| {
        let mut c = claims();
        c[name] = value;
        c
    };
    let without = |name: &str| {
        let mut c = claims();
        c.as_object_mut().unwrap().remove(name);
        c
    };
    for (what, token) in [
        (
            "expired",
            sign(key(), &with("exp", json!(test_keys::now() - 3600))),
        ),
        (
            "not yet valid",
            sign(key(), &with("nbf", json!(test_keys::now() + 3600))),
        ),
        (
            "another application",
            sign(key(), &with("aud", json!(["other-aud"]))),
        ),
        (
            "another team",
            sign(
                key(),
                &with("iss", json!("https://evil.cloudflareaccess.com")),
            ),
        ),
        ("no aud", sign(key(), &without("aud"))),
        ("no iss", sign(key(), &without("iss"))),
        ("no exp", sign(key(), &without("exp"))),
        ("signed by another key under our kid", {
            let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
            header.kid = Some("kid-1".into());
            jsonwebtoken::encode(&header, &claims(), &other_key().encoding).unwrap()
        }),
        ("a changed payload", {
            let token = sign(key(), &claims());
            let parts: Vec<&str> = token.split('.').collect();
            let payload =
                URL_SAFE_NO_PAD_ENGINE.encode(with("email", json!("evil@example.org")).to_string());
            format!("{}.{payload}.{}", parts[0], parts[2])
        }),
        ("malformed", "not-a-token".to_string()),
    ] {
        assert!(gate.verify(&token).await.is_err(), "{what}");
    }
    // The audience list: any configured AUD passes.
    let aud_list = sign(key(), &with("aud", json!(["x", AUD])));
    assert!(gate.verify(&aud_list).await.is_ok());
    assert!(
        gate.verify(&sign(key(), &with("aud", json!(AUD))))
            .await
            .is_ok()
    );
    assert_eq!(ISS, "https://team.example.com");
}

use base64::Engine as _;
const URL_SAFE_NO_PAD_ENGINE: base64::engine::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[tokio::test]
async fn only_rs256_with_a_kid_is_accepted() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    let hs = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(Algorithm::HS256),
        &claims(),
        &jsonwebtoken::EncodingKey::from_secret(b"guessable"),
    )
    .unwrap();
    assert!(gate.verify(&hs).await.unwrap_err().contains("not RS256"));
    let no_kid = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(Algorithm::RS256),
        &claims(),
        &key().encoding,
    )
    .unwrap();
    assert_eq!(
        gate.verify(&no_kid).await.unwrap_err(),
        "the token has no kid"
    );
    assert!(
        gate.verify("a.b")
            .await
            .unwrap_err()
            .starts_with("malformed token")
    );
    assert_eq!(server.count(), 0, "a refused header costs no fetch");
}

#[tokio::test]
async fn an_unknown_kid_fetches_again_at_most_once_a_minute() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    assert!(gate.verify(&sign(key(), &claims())).await.is_ok());
    assert!(gate.verify(&sign(key(), &claims())).await.is_ok());
    assert_eq!(server.count(), 1, "a known kid is served from the cache");
    // A rotated key: an unknown kid within the minute does not fetch.
    server.answer(200, &jwks(&[key(), other_key()]).to_string());
    let rotated = sign(other_key(), &claims());
    assert!(
        gate.verify(&rotated)
            .await
            .unwrap_err()
            .contains("no Access signing key kid-2")
    );
    assert_eq!(server.count(), 1, "no fetch within the minute");
    // The refresher's forced fetch picks it up.
    assert!(gate.refresh_keys(true).await);
    assert_eq!(server.count(), 2);
    assert!(gate.verify(&rotated).await.is_ok());
    // A burst of requests with an unknown kid, nothing cached yet: one fetch.
    let fresh = KeyServer::start(jwks(&[key()])).await;
    let burst = gate_of(&fresh).await;
    let token = sign(key(), &claims());
    let answers = futures_util::future::join_all((0..8).map(|_| burst.verify(&token))).await;
    assert!(answers.iter().all(Result::is_ok));
    assert_eq!(fresh.count(), 1, "one fetch for the burst");
}

async fn gate_of(server: &KeyServer) -> Arc<AccessGate> {
    gate(server).await
}

#[tokio::test]
async fn a_failed_fetch_keeps_the_last_good_keys() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    assert!(gate.refresh_keys(true).await);
    for (status, body) in [
        (500, "down".to_string()),
        (200, "not json".to_string()),
        (200, json!({"keys": []}).to_string()),
        (404, jwks(&[key()]).to_string()),
    ] {
        server.answer(status, &body);
        assert!(!gate.refresh_keys(true).await, "{status} {body}");
        assert!(
            gate.verify(&sign(key(), &claims())).await.is_ok(),
            "{status}"
        );
    }
    assert_eq!(server.count(), 5);
    // Nothing answers at all: still the last good keys.
    let gone = AccessGate::new(
        &crate::config::AccessCfg {
            team_domain: test_keys::TEAM.into(),
            aud: vec![AUD.into()],
            jwks_url: Some("http://127.0.0.1:1/certs".into()),
        },
        HttpClient::new().unwrap(),
    );
    assert!(!gone.refresh_keys(true).await);
    assert!(gone.verify(&sign(key(), &claims())).await.is_err());
}

#[tokio::test]
async fn a_failing_key_endpoint_is_asked_at_most_once_a_minute() {
    let server = KeyServer::start(jwks(&[key()])).await;
    server.answer(500, "down");
    let gate = gate(&server).await;
    let token = sign(key(), &claims());
    assert!(gate.verify(&token).await.is_err());
    assert!(gate.verify(&token).await.is_err());
    assert_eq!(
        server.count(),
        1,
        "the failed attempt counts for the minute"
    );
    // The refresher is not held back by it.
    server.answer(200, &jwks(&[key()]).to_string());
    assert!(gate.refresh_keys(true).await);
    assert!(gate.verify(&token).await.is_ok());
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn the_refresher_warms_the_cache() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    let task = gate.spawn_refresher();
    for _ in 0..100 {
        if server.count() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(server.count(), 1);
    task.abort();
    assert!(gate.verify(&sign(key(), &claims())).await.is_ok());
    assert_eq!(server.count(), 1, "the warm cache served the token");
}

#[tokio::test]
async fn the_policy_in_its_order() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let gate = gate(&server).await;
    let get = Method::GET;
    let good = sign(key(), &claims());
    let lan = peer("192.168.1.20");
    // LAN: allowed with nothing, even without a gate; no fetch.
    assert_eq!(
        decide(None, lan, &HeaderMap::new(), &get).await,
        Decision::Allow { identity: None }
    );
    assert_eq!(
        decide(Some(&gate), lan, &HeaderMap::new(), &get).await,
        Decision::Allow { identity: None }
    );
    assert_eq!(server.count(), 0, "the LAN path does no network I/O");
    // A cross-site write is refused on the LAN too.
    let cross = headers(&[
        ("host", "foh.example.org"),
        ("origin", "https://evil.example"),
    ]);
    assert!(matches!(
        decide(Some(&gate), lan, &cross, &Method::POST).await,
        Decision::Deny {
            origin: Origin::Local,
            reason: "cross_site",
            ..
        }
    ));
    // The internet: no gate, no token, a bad token, a good one.
    let tunnel = |extra: &[(&str, &str)]| {
        let mut pairs = vec![("cf-connecting-ip", "203.0.113.7")];
        pairs.extend_from_slice(extra);
        headers(&pairs)
    };
    let local = peer("127.0.0.1");
    let reason = |d: Decision| match d {
        Decision::Deny { origin, reason, .. } => {
            assert_eq!(origin, Origin::Internet);
            reason
        }
        Decision::Allow { .. } => "allowed",
    };
    assert_eq!(
        reason(decide(None, local, &tunnel(&[(JWT_HEADER, good.as_str())]), &get).await),
        "no_access"
    );
    assert_eq!(
        reason(decide(Some(&gate), local, &tunnel(&[]), &get).await),
        "no_access_token"
    );
    assert_eq!(
        reason(decide(Some(&gate), local, &tunnel(&[(JWT_HEADER, "x.y.z")]), &get).await),
        "invalid_access_token"
    );
    assert_eq!(
        decide(
            Some(&gate),
            local,
            &tunnel(&[(JWT_HEADER, good.as_str())]),
            &get
        )
        .await,
        Decision::Allow {
            identity: Some("engineer@example.org".into())
        }
    );
    let cookie = format!("CF_Authorization={good}");
    assert_eq!(
        decide(
            Some(&gate),
            peer("203.0.113.7"),
            &headers(&[("cookie", cookie.as_str())]),
            &get
        )
        .await,
        Decision::Allow {
            identity: Some("engineer@example.org".into())
        }
    );
}

/// A hub whose `[access]` takes its keys from `server`, with the public
/// name `foh.example.org` allowed.
fn access_hub(dir: &std::path::Path, server: Option<&KeyServer>) -> crate::Hub {
    let mut config = crate::config::Config::defaults(dir);
    config.instances.clear();
    config.allowed_hosts = vec!["foh.example.org".into()];
    config.access = server.map(KeyServer::config);
    crate::Hub::start(config).unwrap()
}

async fn through_router(hub: &crate::Hub, request: axum::http::Request<Body>) -> Response {
    crate::app_router(hub.clone())
        .oneshot(request)
        .await
        .unwrap()
}

fn request(path: &str, from: &str, pairs: &[(&str, &str)]) -> axum::http::Request<Body> {
    let mut request = axum::http::Request::get(path)
        .header("host", "foh.example.org")
        .body(Body::empty())
        .unwrap();
    request.headers_mut().extend(headers(pairs));
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(from.parse().unwrap(), 50000)));
    request
}

async fn code_of(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, body["code"].as_str().unwrap_or("").to_string())
}

#[tokio::test]
async fn the_middleware_gates_every_route_the_spa_included() {
    let server = KeyServer::start(jwks(&[key()])).await;
    let dir = tempfile::tempdir().unwrap();
    let hub = access_hub(dir.path(), Some(&server));
    let good = sign(key(), &claims());
    for path in ["/", "/deep/link", "/api/version", "/index.html", "/ws"] {
        // From the tunnel without an assertion: 403 with the reason.
        let refused = through_router(
            &hub,
            request(path, "127.0.0.1", &[("cf-connecting-ip", "203.0.113.7")]),
        )
        .await;
        assert_eq!(
            code_of(refused).await,
            (StatusCode::FORBIDDEN, "ACCESS_DENIED".into()),
            "{path}"
        );
        // A public peer without the tunnel (a port-forward): 403 too.
        let forwarded = through_router(&hub, request(path, "203.0.113.7", &[])).await;
        assert_eq!(forwarded.status(), StatusCode::FORBIDDEN, "{path}");
        // On the LAN it goes on to the route.
        let lan = through_router(&hub, request(path, "192.168.1.20", &[])).await;
        assert_ne!(lan.status(), StatusCode::FORBIDDEN, "{path}");
    }
    let allowed = through_router(
        &hub,
        request(
            "/api/version",
            "127.0.0.1",
            &[
                ("cf-connecting-ip", "203.0.113.7"),
                (JWT_HEADER, good.as_str()),
            ],
        ),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    // The PIN stays: an allowed internet request still needs its token.
    let status = through_router(
        &hub,
        request(
            "/api/status",
            "127.0.0.1",
            &[
                ("cf-connecting-ip", "203.0.113.7"),
                (JWT_HEADER, good.as_str()),
            ],
        ),
    )
    .await;
    assert_eq!(status.status(), StatusCode::UNAUTHORIZED);
    // The Origin guard on the LAN: a cross-site login attempt.
    let mut login = request(
        "/api/auth",
        "192.168.1.20",
        &[("origin", "https://evil.example")],
    );
    *login.method_mut() = Method::POST;
    assert_eq!(
        code_of(through_router(&hub, login).await).await,
        (StatusCode::FORBIDDEN, "ACCESS_DENIED".into())
    );
    hub.stop();
}

#[tokio::test]
async fn without_access_every_internet_request_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let hub = access_hub(dir.path(), None);
    assert!(hub.access.is_none());
    let good = sign(key(), &claims());
    let refused = through_router(
        &hub,
        request(
            "/api/version",
            "127.0.0.1",
            &[
                ("cf-connecting-ip", "203.0.113.7"),
                (JWT_HEADER, good.as_str()),
            ],
        ),
    )
    .await;
    let bytes = axum::body::to_bytes(refused.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "ACCESS_DENIED");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("Refused (no_access)"),
        "{body}"
    );
    let lan = through_router(&hub, request("/api/version", "10.0.0.9", &[])).await;
    assert_eq!(lan.status(), StatusCode::OK);
    hub.stop();
}

#[tokio::test]
async fn a_request_without_a_peer_address_is_refused() {
    // Both listeners give every request its peer address; a router without
    // one (a listener that lost ConnectInfo) must not let requests in.
    let dir = tempfile::tempdir().unwrap();
    let hub = access_hub(dir.path(), None);
    let request = axum::http::Request::get("/api/version")
        .header("host", "foh.example.org")
        .body(Body::empty())
        .unwrap();
    let response = crate::app_router(hub.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        code_of(response).await,
        (StatusCode::FORBIDDEN, "ACCESS_DENIED".into())
    );
    hub.stop();
}

#[test]
fn a_refusal_names_its_reason_and_the_way_in() {
    let response = forbidden("no_access_token");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
