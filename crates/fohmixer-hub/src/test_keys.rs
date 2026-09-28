//! Test support for the Access tests (#17): RSA keys made by `openssl
//! genrsa` (the RSA crate carries an unfixed advisory; ring cannot make RSA
//! keys), Access-shaped JWTs signed with them, and a local key-set server
//! that counts its fetches. Test-only: never compiled into the hub.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rustls::pki_types::PrivatePkcs1KeyDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

/// The team, application and issuer the tests' gates are configured with.
pub const TEAM: &str = "team.example.com";
pub const AUD: &str = "aud-fohmixer-test";
pub const ISS: &str = "https://team.example.com";

/// An RSA key: its `kid`, the signing half and the public JWK.
pub struct TestKey {
    pub kid: String,
    pub encoding: EncodingKey,
    pub jwk: Value,
}

fn generate(kid: &str) -> TestKey {
    let out = std::process::Command::new("openssl")
        .args(["genrsa", "-traditional", "2048"])
        .output()
        .expect("openssl makes the test RSA keys (install it for these tests)");
    assert!(out.status.success(), "openssl genrsa failed: {out:?}");
    let der = PrivatePkcs1KeyDer::from_pem_slice(&out.stdout).expect("a PKCS#1 key");
    let pair = ring::signature::RsaKeyPair::from_der(der.secret_pkcs1_der()).expect("an RSA key");
    let public = ring::signature::RsaPublicKeyComponents::<Vec<u8>>::from(pair.public());
    TestKey {
        kid: kid.to_string(),
        encoding: EncodingKey::from_rsa_der(der.secret_pkcs1_der()),
        jwk: json!({
            "kid": kid,
            "kty": "RSA",
            "alg": "RS256",
            "use": "sig",
            "n": URL_SAFE_NO_PAD.encode(public.n),
            "e": URL_SAFE_NO_PAD.encode(public.e),
        }),
    }
}

/// The tests' Access key.
pub fn key() -> &'static TestKey {
    static KEY: OnceLock<TestKey> = OnceLock::new();
    KEY.get_or_init(|| generate("kid-1"))
}

/// A second key (a rotation, or a key of another team).
pub fn other_key() -> &'static TestKey {
    static KEY: OnceLock<TestKey> = OnceLock::new();
    KEY.get_or_init(|| generate("kid-2"))
}

/// The Unix time now.
pub fn now() -> i64 {
    crate::auth::now_secs() as i64
}

/// Access-shaped claims for the test application, valid for an hour.
pub fn claims() -> Value {
    json!({
        "aud": [AUD],
        "iss": ISS,
        "exp": now() + 3600,
        "iat": now(),
        "nbf": now() - 10,
        "email": "engineer@example.org",
        "sub": "user-1",
    })
}

/// `claims` signed RS256 by `key` (its `kid` in the header).
pub fn sign(key: &TestKey, claims: &Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    jsonwebtoken::encode(&header, claims, &key.encoding).expect("a signed test token")
}

/// A key-set document of `keys`.
pub fn jwks(keys: &[&TestKey]) -> Value {
    json!({ "keys": keys.iter().map(|k| k.jwk.clone()).collect::<Vec<_>>() })
}

/// A local key-set server: what it answers can change, its fetches are
/// counted.
pub struct KeyServer {
    pub url: String,
    pub fetches: Arc<AtomicUsize>,
    answer: Arc<Mutex<(u16, String)>>,
}

impl KeyServer {
    /// A server answering `body` with 200 at `/cdn-cgi/access/certs`.
    pub async fn start(body: Value) -> Self {
        let fetches = Arc::new(AtomicUsize::new(0));
        let answer = Arc::new(Mutex::new((200, body.to_string())));
        let app = {
            let fetches = Arc::clone(&fetches);
            let answer = Arc::clone(&answer);
            axum::Router::new().route(
                "/cdn-cgi/access/certs",
                axum::routing::get(move || {
                    let fetches = Arc::clone(&fetches);
                    let answer = Arc::clone(&answer);
                    async move {
                        fetches.fetch_add(1, Ordering::SeqCst);
                        let (status, body) = answer.lock().unwrap().clone();
                        (axum::http::StatusCode::from_u16(status).unwrap(), body)
                    }
                }),
            )
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url: format!("http://{addr}/cdn-cgi/access/certs"),
            fetches,
            answer,
        }
    }

    /// From now on it answers `status` with `body`.
    pub fn answer(&self, status: u16, body: &str) {
        *self.answer.lock().unwrap() = (status, body.to_string());
    }

    /// How many times it was fetched.
    pub fn count(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    /// The `[access]` config of the test application, keys from here.
    pub fn config(&self) -> crate::config::AccessCfg {
        crate::config::AccessCfg {
            team_domain: TEAM.to_string(),
            aud: vec![AUD.to_string()],
            jwks_url: Some(self.url.clone()),
        }
    }
}
