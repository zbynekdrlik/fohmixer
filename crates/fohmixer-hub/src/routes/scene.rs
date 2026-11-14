//! `POST /api/scene` (#207): Companion's trigger on the service's scene
//! tells the hub the scene, `{"scene": "<name>"}`, with the automation
//! token as `Authorization: Bearer <token>` (`crate::automation`). Taken on
//! the LAN only; the router learns the name and moves the faders with a
//! rule for it (`router/scenes.rs`), and the answer says how many.
//!
//! The order of the checks ([`check`], pure): an internet request is 403
//! (`LAN_ONLY`) before anything else; a hub without a token 503
//! (`NO_AUTOMATION`, naming the command that sets one); a missing or wrong
//! token 401 (`UNAUTHORIZED`); a body that is not `{"scene": <name>}` with
//! a scene name 400 (`BAD_SCENE`, why). Every refusal is one `scene`
//! record with `refused` and one warn line, at most
//! [`REFUSED_LINES`] per `ration::GAP`.

use std::net::SocketAddr;
use std::time::Instant;

use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use fohmixer_proto::scenes::scene_name;
use serde::Deserialize;
use serde_json::json;

use crate::Hub;
use crate::access::{self, Origin};
use crate::auth::{bearer, error_response};
use crate::router::RouterMsg;

/// The most refusal records and lines per `ration::GAP`.
pub const REFUSED_LINES: u32 = 8;

/// The body: exactly the scene's name.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneBody {
    scene: String,
}

/// Why a scene request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// An internet request (403).
    Internet,
    /// The hub has no automation token (503).
    NoToken,
    /// A missing or wrong token (401).
    Token,
    /// The body is no scene (400), why.
    Body(String),
}

impl Refused {
    /// Its word in the record.
    pub fn word(&self) -> &'static str {
        match self {
            Refused::Internet => "internet",
            Refused::NoToken => "no_token",
            Refused::Token => "token",
            Refused::Body(_) => "body",
        }
    }

    /// Its answer.
    pub fn response(&self) -> Response {
        match self {
            Refused::Internet => error_response(
                StatusCode::FORBIDDEN,
                "LAN_ONLY",
                "Scenes are taken on the local network only",
            ),
            Refused::NoToken => error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "NO_AUTOMATION",
                "This hub takes no scenes: set a token with `fohmixer-hub automation set-token` and restart it",
            ),
            Refused::Token => error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Send the automation token as Authorization: Bearer <token>",
            ),
            Refused::Body(why) => error_response(StatusCode::BAD_REQUEST, "BAD_SCENE", why),
        }
    }
}

/// The scene a body names, or why it names none.
pub fn body_scene(body: &[u8]) -> Result<String, String> {
    let parsed: SceneBody = serde_json::from_slice(body)
        .map_err(|e| format!("the body is not {{\"scene\": \"<name>\"}}: {e}"))?;
    scene_name(&parsed.scene).map_err(str::to_string)
}

/// The checks in their order: the LAN, a token set, the token, the body.
pub fn check(
    origin: Origin,
    token_set: bool,
    token_ok: bool,
    body: &[u8],
) -> Result<String, Refused> {
    if origin == Origin::Internet {
        return Err(Refused::Internet);
    }
    if !token_set {
        return Err(Refused::NoToken);
    }
    if !token_ok {
        return Err(Refused::Token);
    }
    body_scene(body).map_err(Refused::Body)
}

/// The warn line of a refusal (`skipped`: the refusals left out before it).
pub fn refused_line(refused: &Refused, peer: &str, skipped: u64) -> String {
    let why = match refused {
        Refused::Body(why) => format!("body: {why}"),
        other => other.word().to_string(),
    };
    format!("a scene request is refused ({why}) from {peer}; {skipped} left out before")
}

/// `POST /api/scene`.
pub async fn post_scene(
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let origin = access::classify(Some(peer), &headers);
    let source = crate::client_report::source(origin);
    let peer_ip = peer.ip().to_string();
    let token = hub.automation.as_ref();
    let ok = token.is_some_and(|t| t.accepts(bearer(&headers)));
    let scene = match check(origin, token.is_some(), ok, &body) {
        Ok(scene) => scene,
        Err(refused) => {
            let allowed = hub
                .scene_refusals
                .lock()
                .map(|mut r| r.take(Instant::now(), REFUSED_LINES))
                .ok()
                .flatten();
            if let Some(skipped) = allowed {
                tracing::warn!("{}", refused_line(&refused, &peer_ip, skipped));
                hub.events.record(
                    "scene",
                    json!({"refused": refused.word(), "source": source, "peer": peer_ip,
                           "skipped": skipped}),
                );
            }
            return refused.response();
        }
    };
    let (reply, answer) = tokio::sync::oneshot::channel();
    hub.route(RouterMsg::Scene {
        name: scene.clone(),
        source: source.to_string(),
        peer: peer_ip,
        reply,
    });
    match answer.await {
        Ok(moved) => (
            StatusCode::OK,
            Json(json!({"scene": scene, "moved": moved})),
        )
            .into_response(),
        Err(_) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "STOPPING",
            "The hub is stopping",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checks_go_lan_token_set_token_body() {
        let good = br#"{"scene": " intro "}"#;
        assert_eq!(check(Origin::Local, true, true, good), Ok("intro".into()));
        assert_eq!(
            check(Origin::Internet, true, true, good),
            Err(Refused::Internet)
        );
        assert_eq!(
            check(Origin::Internet, false, false, b"x"),
            Err(Refused::Internet)
        );
        assert_eq!(check(Origin::Local, false, false, good), Err(Refused::NoToken));
        assert_eq!(check(Origin::Local, true, false, good), Err(Refused::Token));
        assert!(matches!(
            check(Origin::Local, true, true, b"{}"),
            Err(Refused::Body(_))
        ));
    }

    #[test]
    fn a_body_names_exactly_one_scene() {
        assert_eq!(body_scene(br#"{"scene": "talk"}"#), Ok("talk".into()));
        assert_eq!(
            body_scene(br#"{"scene": "  "}"#),
            Err("the scene name is empty".into())
        );
        for bad in [
            &br#"{"scene": "talk", "x": 1}"#[..],
            br#"{"scene": 5}"#,
            b"talk",
            b"",
        ] {
            let why = body_scene(bad).unwrap_err();
            assert!(why.starts_with("the body is not {\"scene\": \"<name>\"}: "), "{why}");
        }
    }

    #[test]
    fn each_refusal_has_its_status_code_and_word() {
        for (refused, status, word) in [
            (Refused::Internet, StatusCode::FORBIDDEN, "internet"),
            (Refused::NoToken, StatusCode::SERVICE_UNAVAILABLE, "no_token"),
            (Refused::Token, StatusCode::UNAUTHORIZED, "token"),
            (Refused::Body("x".into()), StatusCode::BAD_REQUEST, "body"),
        ] {
            assert_eq!(refused.response().status(), status);
            assert_eq!(refused.word(), word);
        }
        assert_eq!(REFUSED_LINES, 8);
    }

    #[test]
    fn a_refusal_line_names_why_and_the_peer() {
        assert_eq!(
            refused_line(&Refused::Token, "10.0.0.5", 2),
            "a scene request is refused (token) from 10.0.0.5; 2 left out before"
        );
        assert_eq!(
            refused_line(&Refused::Body("the scene name is empty".into()), "10.0.0.5", 0),
            "a scene request is refused (body: the scene name is empty) from 10.0.0.5; 0 left out before"
        );
    }
}
