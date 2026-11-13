//! Login: PIN → JWT, and token checks (from iemmixer's
//! `iem-server/src/auth.rs` @ 22372bc; spec §2.4, X8: one login per device,
//! the token lasts 7 days). Since #184 two roles (`fohmixer_proto::roles`):
//! the PIN names the role (the engineer's or the MBC operator's, which must
//! differ), and the token's subject carries it; an engineer token of before
//! (`sub: "engineer"`) stays valid.
//!
//! Login protection: admission by [`LoginGuard`] before any hashing, a
//! bounded hashing gate, argon2id verification against the stored hash,
//! failure-only budgets, never a lockout. The PIN is set with
//! `fohmixer-hub pin set-engineer`; the store is read at each login, so a
//! PIN set while the hub runs takes effect at once.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use fohmixer_proto::client::{ApiError, AuthRequest, AuthResponse};
use fohmixer_proto::roles::Role;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::login_guard::{FailureEffect, HASH_CONCURRENCY, HASH_QUEUE, HashGate, LoginGuard};
use crate::pin_hash::PinHasher;
use crate::pin_store::PinStore;
use crate::secrets::SECRETS_DIR;

/// Token lifetime: 7 days (iemmixer's engineer token).
pub const TOKEN_EXPIRY_SECS: u64 = 7 * 24 * 60 * 60;
/// The token's claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
}

impl Claims {
    /// The role the token's subject names.
    pub fn role(&self) -> Option<Role> {
        Role::from_sub(&self.sub)
    }
}

/// The role a login's PIN matched: the engineer's first (the two PINs
/// differ, so both matching is a store written by hand).
pub fn login_role(engineer_ok: bool, mbc_ok: bool) -> Option<Role> {
    if engineer_ok {
        Some(Role::Engineer)
    } else if mbc_ok {
        Some(Role::Mbc)
    } else {
        None
    }
}

/// A rejection already rendered as a response (boxed: `Response` is large).
pub struct Rejection(Box<Response>);

impl From<Response> for Rejection {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        *self.0
    }
}

/// An API error response.
pub fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(ApiError::new(code, message))).into_response()
}

/// 401 for a missing, invalid or expired token.
pub fn unauthorized() -> Response {
    error_response(
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
        "Log in with the engineer PIN",
    )
}

/// 429 with `Retry-After` in whole seconds (at least 1).
pub fn too_many_attempts(wait: Duration) -> Response {
    let secs = wait.as_millis().div_ceil(1000).max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, secs.to_string())],
        Json(ApiError::new(
            "TOO_MANY_ATTEMPTS",
            "Too many attempts, try again later",
        )),
    )
        .into_response()
}

/// The Unix time now (seconds).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A token for `role`, issued at `now` (Unix seconds).
pub fn issue_token(
    secret: &str,
    now: u64,
    role: Role,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: role.sub().to_string(),
        iat: now,
        exp: now + TOKEN_EXPIRY_SECS,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// The claims of a token signed with `secret`, valid through the second of
/// its `exp` at the Unix time `now`, whose subject names a role.
pub fn claims_at(token: Option<&str>, secret: &str, now: u64) -> Option<Claims> {
    let data = decode::<Claims>(
        token?,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    (data.claims.exp >= now && data.claims.role().is_some()).then_some(data.claims)
}

/// The token of an `Authorization: Bearer …` header.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// The start-up warning about the PIN store: without an engineer PIN
/// nobody can log in as the engineer.
fn pin_warning(has_pin: bool) -> Option<&'static str> {
    (!has_pin).then_some(
        "no engineer PIN yet: set one with `fohmixer-hub pin set-engineer` (nobody can log in as the engineer)",
    )
}

/// What a failed login changed that is worth a warning: the hub-wide
/// budget running out (every later attempt is spaced).
fn failure_note(effect: FailureEffect) -> Option<&'static str> {
    (effect == FailureEffect::GlobalBudgetExhausted)
        .then_some("login failures exhausted the hourly budget: attempts are now spaced")
}

/// The login state: the signing key, the hasher and the budgets.
pub struct Auth {
    jwt_secret: String,
    secrets_dir: PathBuf,
    hasher: PinHasher,
    guard: LoginGuard,
    gate: HashGate,
}

impl Auth {
    /// Loads the signing key and the pepper from `<data_dir>/secrets/`
    /// (creating them on first use) and checks the PIN store is readable: a
    /// corrupt or plaintext store stops the start, never ignored.
    pub fn load(data_dir: &Path) -> io::Result<Self> {
        let secrets_dir = data_dir.join(SECRETS_DIR);
        let secrets = crate::secrets::load_or_create(&secrets_dir)?;
        let pepper = crate::pepper::load_or_create(&secrets_dir)?;
        let store = PinStore::load(&secrets_dir)?;
        // The engineer's PIN, whatever the MBC one: without it nobody
        // reaches the FOH side (#184).
        if let Some(warning) = pin_warning(store.engineer_hash().is_some()) {
            tracing::warn!("{warning}");
        }
        Ok(Self::with(
            secrets.jwt_secret,
            secrets_dir,
            PinHasher::new(pepper),
        ))
    }

    fn with(jwt_secret: String, secrets_dir: PathBuf, hasher: PinHasher) -> Self {
        Self {
            jwt_secret,
            secrets_dir,
            hasher,
            guard: LoginGuard::new(),
            gate: HashGate::new(HASH_CONCURRENCY, HASH_QUEUE),
        }
    }

    /// A new token for `role` (the login's; tests and tools).
    pub fn issue(&self, role: Role) -> Result<String, jsonwebtoken::errors::Error> {
        issue_token(&self.jwt_secret, now_secs(), role)
    }

    /// The claims of a valid token now (one whose subject names a role).
    pub fn claims(&self, token: Option<&str>) -> Option<Claims> {
        claims_at(token, &self.jwt_secret, now_secs())
    }

    /// The role of a valid token now.
    pub fn role(&self, token: Option<&str>) -> Option<Role> {
        self.claims(token).as_ref().and_then(Claims::role)
    }

    /// The claims and role of the request's bearer token, or the 401 to
    /// answer.
    pub fn require(&self, headers: &HeaderMap) -> Result<(Claims, Role), Rejection> {
        self.claims(bearer(headers))
            .and_then(|claims| claims.role().map(|role| (claims, role)))
            .ok_or_else(|| Rejection::from(unauthorized()))
    }

    /// Runs argon2id work on the blocking pool, admitted by the gate: the
    /// PIN against the engineer's and the MBC operator's hash, in one
    /// permit (one login is one admission, whichever role it is).
    async fn verify(
        &self,
        pin: String,
        engineer: Option<String>,
        mbc: Option<String>,
    ) -> Result<Option<Role>, Rejection> {
        let Some(permit) = self.gate.acquire().await else {
            tracing::warn!("PIN hashing gate full: answering 429");
            return Err(Rejection::from(too_many_attempts(Duration::from_secs(1))));
        };
        let hasher = self.hasher.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            login_role(
                hasher.verify_optional(&pin, engineer.as_deref()),
                hasher.verify_optional(&pin, mbc.as_deref()),
            )
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "PIN hashing task failed");
            Rejection::from(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "HASH_ERROR",
                "PIN check failed",
            ))
        })
    }
}

/// `POST /api/auth {pin}`: the token of the role the PIN matches, or 401 /
/// 429.
pub async fn login(
    State(auth): State<Arc<Auth>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, Rejection> {
    let now = Instant::now();
    if let Err(wait) = auth.guard.check(peer.ip(), now) {
        tracing::info!(peer = %peer.ip(), wait_ms = wait.as_millis() as u64, "login throttled");
        return Err(Rejection::from(too_many_attempts(wait)));
    }
    let (engineer, mbc) = match PinStore::load(&auth.secrets_dir) {
        Ok(store) => (
            store.hash(Role::Engineer).map(str::to_owned),
            store.hash(Role::Mbc).map(str::to_owned),
        ),
        Err(e) => {
            tracing::error!(error = %e, "the PIN store is unreadable");
            return Err(Rejection::from(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PIN_STORE",
                "The PIN store is unreadable",
            )));
        }
    };
    if let Some(role) = auth.verify(req.pin, engineer, mbc).await? {
        auth.guard.record_success(peer.ip());
        let token = auth.issue(role).map_err(|e| {
            tracing::error!(error = %e, "cannot sign a token");
            Rejection::from(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "TOKEN_ERROR",
                "Failed to create token",
            ))
        })?;
        tracing::info!(peer = %peer.ip(), "{} logged in", role.as_str());
        Ok(Json(AuthResponse {
            token,
            expires_in: TOKEN_EXPIRY_SECS,
            role: Some(role),
        }))
    } else {
        if let Some(note) = failure_note(auth.guard.record_failure(peer.ip(), now)) {
            tracing::warn!("{note}");
        }
        tracing::info!(peer = %peer.ip(), "login failed: invalid PIN");
        Err(Rejection::from(error_response(
            StatusCode::UNAUTHORIZED,
            "INVALID_PIN",
            "Invalid PIN",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_come_only_when_something_is_wrong() {
        assert!(pin_warning(false).unwrap().contains("pin set-engineer"));
        assert_eq!(pin_warning(true), None);
        assert!(
            failure_note(FailureEffect::GlobalBudgetExhausted)
                .unwrap()
                .contains("hourly budget")
        );
        assert_eq!(failure_note(FailureEffect::Counted), None);
    }
    use crate::pin_hash::PEPPER_LEN;
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::Request;
    use axum::routing::post;
    use tower::util::ServiceExt;

    const PIN: &str = "2468";
    const WRONG_PIN: &str = "9753";
    const SECRET: &str = "login-test-secret";

    const MBC_PIN: &str = "1357";

    fn auth(dir: &Path, with_pin: bool) -> Arc<Auth> {
        let hasher = PinHasher::for_tests([5u8; PEPPER_LEN]);
        if with_pin {
            PinStore::load(dir)
                .unwrap()
                .set_engineer_hash(hasher.hash(PIN))
                .unwrap();
        }
        Arc::new(Auth::with(SECRET.to_string(), dir.to_path_buf(), hasher))
    }

    /// An `Auth` with both PINs stored.
    fn both(dir: &Path) -> Arc<Auth> {
        let hasher = PinHasher::for_tests([5u8; PEPPER_LEN]);
        let mut store = PinStore::load(dir).unwrap();
        store.set_engineer_hash(hasher.hash(PIN)).unwrap();
        store.set_hash(Role::Mbc, hasher.hash(MBC_PIN)).unwrap();
        Arc::new(Auth::with(SECRET.to_string(), dir.to_path_buf(), hasher))
    }

    #[test]
    fn login_role_names_the_pin_that_matched() {
        assert_eq!(login_role(true, false), Some(Role::Engineer));
        assert_eq!(login_role(false, true), Some(Role::Mbc));
        assert_eq!(login_role(false, false), None);
        assert_eq!(login_role(true, true), Some(Role::Engineer));
    }

    #[test]
    fn a_token_carries_its_role() {
        let now = now_secs();
        let token = issue_token(SECRET, now, Role::Mbc).unwrap();
        let claims = claims_at(Some(&token), SECRET, now).unwrap();
        assert_eq!(claims.sub, "mbc");
        assert_eq!(claims.role(), Some(Role::Mbc));
        let token = issue_token(SECRET, now, Role::Engineer).unwrap();
        assert_eq!(
            claims_at(Some(&token), SECRET, now).unwrap().role(),
            Some(Role::Engineer)
        );
    }

    /// A token signed with `claims` as they are, the way a hub before #184
    /// signed every token.
    fn signed(claims: &Claims) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn an_engineer_token_of_before_stays_valid() {
        let now = now_secs();
        let old = signed(&Claims {
            sub: "engineer".into(),
            iat: now - 3600,
            exp: now + 3600,
        });
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path(), false);
        assert_eq!(auth.role(Some(&old)), Some(Role::Engineer));
    }

    #[test]
    fn a_token_naming_no_role_is_refused() {
        let now = now_secs();
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path(), false);
        for sub in ["hub", "", "Engineer"] {
            let token = signed(&Claims {
                sub: sub.into(),
                iat: now,
                exp: now + 3600,
            });
            assert_eq!(claims_at(Some(&token), SECRET, now), None, "{sub:?}");
            assert_eq!(auth.role(Some(&token)), None, "{sub:?}");
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            assert!(auth.require(&headers).is_err(), "{sub:?}");
        }
    }

    #[tokio::test]
    async fn the_login_names_the_role_its_pin_matches() {
        let dir = tempfile::tempdir().unwrap();
        let auth = both(dir.path());
        for (pin, role) in [(PIN, Role::Engineer), (MBC_PIN, Role::Mbc)] {
            let resp = login_with(&app(Arc::clone(&auth), [10, 0, 0, 50]), pin).await;
            assert_eq!(resp.status(), StatusCode::OK, "{pin}");
            let body = json_body(resp).await;
            assert_eq!(body["role"], role.as_str());
            let token = body["token"].as_str().unwrap();
            assert_eq!(auth.role(Some(token)), Some(role));
        }
        let resp = login_with(&app(Arc::clone(&auth), [10, 0, 0, 50]), WRONG_PIN).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(resp).await["code"], "INVALID_PIN");
    }

    #[tokio::test]
    async fn only_the_mbc_pin_logs_in_as_mbc() {
        let dir = tempfile::tempdir().unwrap();
        let hasher = PinHasher::for_tests([5u8; PEPPER_LEN]);
        PinStore::load(dir.path())
            .unwrap()
            .set_hash(Role::Mbc, hasher.hash(MBC_PIN))
            .unwrap();
        let auth = Arc::new(Auth::with(
            SECRET.to_string(),
            dir.path().to_path_buf(),
            hasher,
        ));
        let app = app(Arc::clone(&auth), [10, 0, 0, 50]);
        assert_eq!(
            login_with(&app, PIN).await.status(),
            StatusCode::UNAUTHORIZED
        );
        let body = json_body(login_with(&app, MBC_PIN).await).await;
        assert_eq!(body["role"], "mbc");
    }

    #[tokio::test]
    async fn a_wrong_pin_counts_as_one_failure() {
        let dir = tempfile::tempdir().unwrap();
        let auth = both(dir.path());
        let app = app(Arc::clone(&auth), [10, 0, 0, 50]);
        assert_eq!(
            login_with(&app, WRONG_PIN).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(auth.guard.stats().failures, 1);
    }

    fn app(auth: Arc<Auth>, peer: [u8; 4]) -> axum::Router {
        axum::Router::new()
            .route("/api/auth", post(login))
            .with_state(auth)
            .layer(MockConnectInfo(SocketAddr::from((peer, 40000))))
    }

    async fn login_with(app: &axum::Router, pin: &str) -> Response {
        let req = Request::post("/api/auth")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "pin": pin }).to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    async fn json_body(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn the_engineer_pin_gives_a_seven_day_token() {
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path(), true);
        let resp = login_with(&app(Arc::clone(&auth), [10, 0, 0, 50]), PIN).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert_eq!(body["expires_in"], 7 * 24 * 60 * 60);
        let token = body["token"].as_str().unwrap();
        let claims = auth.claims(Some(token)).expect("a valid token");
        assert_eq!(claims.sub, "engineer");
        assert!(claims.exp.abs_diff(now_secs() + TOKEN_EXPIRY_SECS) < 5);
        assert_eq!(claims.exp - claims.iat, TOKEN_EXPIRY_SECS);
    }

    #[tokio::test]
    async fn a_wrong_or_empty_pin_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(auth(dir.path(), true), [10, 0, 0, 50]);
        for pin in [WRONG_PIN, ""] {
            let resp = login_with(&app, pin).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{pin:?}");
            let body = json_body(resp).await;
            assert_eq!(body["code"], "INVALID_PIN");
            assert!(body.get("token").is_none());
        }
    }

    #[tokio::test]
    async fn without_a_stored_pin_nobody_logs_in() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(auth(dir.path(), false), [10, 0, 0, 50]);
        assert_eq!(
            login_with(&app, PIN).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn the_fourth_attempt_after_three_failures_is_throttled_even_with_the_right_pin() {
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path(), true);
        let app = app(Arc::clone(&auth), [10, 0, 0, 50]);
        for _ in 0..3 {
            assert_eq!(
                login_with(&app, WRONG_PIN).await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let resp = login_with(&app, PIN).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers()[header::RETRY_AFTER], "1");
        assert_eq!(json_body(resp).await["code"], "TOO_MANY_ATTEMPTS");
        assert_eq!(auth.guard.stats().failures, 3);
        // Another device is not slowed.
        let other = app_for(&auth, [10, 0, 0, 51]);
        assert_eq!(login_with(&other, PIN).await.status(), StatusCode::OK);
    }

    fn app_for(auth: &Arc<Auth>, peer: [u8; 4]) -> axum::Router {
        app(Arc::clone(auth), peer)
    }

    #[tokio::test]
    async fn success_resets_the_failure_streak() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(auth(dir.path(), true), [10, 0, 0, 50]);
        for _ in 0..2 {
            login_with(&app, WRONG_PIN).await;
        }
        assert_eq!(login_with(&app, PIN).await.status(), StatusCode::OK);
        for _ in 0..2 {
            assert_eq!(
                login_with(&app, WRONG_PIN).await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(login_with(&app, PIN).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_full_hashing_gate_answers_429_without_counting_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let hasher = PinHasher::for_tests([5u8; PEPPER_LEN]);
        let mut auth = Auth::with(SECRET.to_string(), dir.path().to_path_buf(), hasher);
        auth.gate = HashGate::new(0, 0);
        let auth = Arc::new(auth);
        let resp = tokio::time::timeout(
            Duration::from_secs(5),
            login_with(&app(Arc::clone(&auth), [10, 0, 0, 50]), PIN),
        )
        .await
        .expect("a full gate answers at once instead of queueing");
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers()[header::RETRY_AFTER], "1");
        assert_eq!(auth.guard.stats().failures, 0);
    }

    #[tokio::test]
    async fn an_unreadable_pin_store_is_a_server_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(crate::pin_store::PIN_HASHES_FILE),
            "{not json",
        )
        .unwrap();
        let resp = login_with(&app(auth(dir.path(), false), [10, 0, 0, 50]), PIN).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json_body(resp).await["code"], "PIN_STORE");
    }

    #[test]
    fn retry_after_is_whole_seconds_rounded_up() {
        for (wait, expected) in [
            (Duration::from_millis(1), "1"),
            (Duration::from_millis(1001), "2"),
            (Duration::from_secs(60), "60"),
        ] {
            assert_eq!(
                too_many_attempts(wait).headers()[header::RETRY_AFTER],
                expected
            );
        }
    }

    #[test]
    fn a_token_is_valid_through_its_expiry_second() {
        let now = now_secs();
        let token = issue_token(SECRET, now, Role::Engineer).unwrap();
        let exp = now + TOKEN_EXPIRY_SECS;
        assert_eq!(
            claims_at(Some(&token), SECRET, exp),
            Some(Claims {
                sub: "engineer".into(),
                iat: now,
                exp
            })
        );
        assert_eq!(claims_at(Some(&token), SECRET, exp + 1), None);
        assert_eq!(claims_at(Some(&token), "another-secret", now), None);
        assert_eq!(claims_at(Some("not.a.token"), SECRET, now), None);
        assert_eq!(claims_at(None, SECRET, now), None);
        // A token that expired in the real clock does not decode at all.
        let old = issue_token(SECRET, now - TOKEN_EXPIRY_SECS - 3600, Role::Engineer).unwrap();
        assert_eq!(claims_at(Some(&old), SECRET, 0), None);
    }

    #[test]
    fn the_bearer_token_is_read_from_the_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        headers.insert(header::AUTHORIZATION, "Bearer abc.def".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc.def"));
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path(), false);
        assert!(auth.require(&headers).is_err());
        let token = issue_token(SECRET, now_secs(), Role::Engineer).unwrap();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let (claims, role) = auth.require(&headers).ok().unwrap();
        assert_eq!((claims.sub.as_str(), role), ("engineer", Role::Engineer));
        let token = issue_token(SECRET, now_secs(), Role::Mbc).unwrap();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(auth.require(&headers).ok().unwrap().1, Role::Mbc);
        let resp = auth
            .require(&HeaderMap::new())
            .err()
            .unwrap()
            .into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn load_creates_the_secrets_and_refuses_a_plaintext_store() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::load(dir.path()).unwrap();
        let token = issue_token(&auth.jwt_secret, now_secs(), Role::Engineer).unwrap();
        assert!(auth.claims(Some(&token)).is_some());
        assert!(dir.path().join(SECRETS_DIR).join("jwt_secret").is_file());
        let again = Auth::load(dir.path()).unwrap();
        assert_eq!(again.jwt_secret, auth.jwt_secret, "the key is kept");
        std::fs::write(
            dir.path()
                .join(SECRETS_DIR)
                .join(crate::pin_store::PIN_HASHES_FILE),
            r#"{"engineer":"2468"}"#,
        )
        .unwrap();
        assert!(Auth::load(dir.path()).is_err());
    }
}
