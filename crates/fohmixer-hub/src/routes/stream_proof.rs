//! `GET /api/mbc/stream-proof` (#267): camera-box's stream-test guard asks,
//! before each test, whether the MBC stream mix carries only HAND 1 sync,
//! with the automation token (#207) as `Authorization: Bearer <token>`. The
//! answer is `fohmixer_proto::stream_proof::Answer` (the walk and its
//! cache: `crate::stream_proof`); it is 200 whatever the proof says, also
//! `ok: false` (camera-box then stays strict).
//!
//! The order of the checks ([`check`], pure, as `POST /api/scene`'s): an
//! internet request is 403 (`LAN_ONLY`); a hub without a token 503
//! (`NO_AUTOMATION`); a missing or wrong token 401 (`UNAUTHORIZED`; the
//! PIN's session token is no automation token). Every refusal is one
//! `stream_proof` record with `refused` and one warn line, at most
//! [`REFUSED_LINES`] per `ration::GAP`.

use std::net::SocketAddr;
use std::time::Instant;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use fohmixer_proto::roles::MBC_INSTANCE;
use serde_json::json;

use crate::Hub;
use crate::access::{self, Origin};
use crate::auth::{bearer, error_response};

/// The route's path.
pub const PATH: &str = "/api/mbc/stream-proof";
/// The most refusal records and lines per `ration::GAP`.
pub const REFUSED_LINES: u32 = 8;

/// Why a request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// An internet request (403).
    Internet,
    /// The hub has no automation token (503).
    NoToken,
    /// A missing or wrong token (401).
    Token,
}

impl Refused {
    /// Its word in the record.
    pub fn word(self) -> &'static str {
        match self {
            Refused::Internet => "internet",
            Refused::NoToken => "no_token",
            Refused::Token => "token",
        }
    }

    /// Its answer.
    pub fn response(self) -> Response {
        match self {
            Refused::Internet => error_response(
                StatusCode::FORBIDDEN,
                "LAN_ONLY",
                "The stream proof is served on the local network only",
            ),
            Refused::NoToken => error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "NO_AUTOMATION",
                "This hub has no automation token: set one with `fohmixer-hub automation set-token` and restart it",
            ),
            Refused::Token => error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Send the automation token as Authorization: Bearer <token>",
            ),
        }
    }
}

/// The checks in their order: the LAN, a token set, the token.
pub fn check(origin: Origin, token_set: bool, token_ok: bool) -> Result<(), Refused> {
    if origin == Origin::Internet {
        return Err(Refused::Internet);
    }
    if !token_set {
        return Err(Refused::NoToken);
    }
    if !token_ok {
        return Err(Refused::Token);
    }
    Ok(())
}

/// The warn line of a refusal (`skipped`: the refusals left out before it).
pub fn refused_line(refused: Refused, peer: &str, skipped: u64) -> String {
    format!(
        "a stream proof request is refused ({}) from {peer}; {skipped} left out before",
        refused.word()
    )
}

/// `GET /api/mbc/stream-proof`.
pub async fn get_stream_proof(
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let origin = access::classify(Some(peer), &headers);
    let source = crate::client_report::source(origin);
    let peer_ip = peer.ip().to_string();
    let token = hub.automation.as_ref();
    let ok = token.is_some_and(|t| t.accepts(bearer(&headers)));
    if let Err(refused) = check(origin, token.is_some(), ok) {
        let allowed = hub
            .stream_proofs
            .refusals
            .lock()
            .map(|mut r| r.take(Instant::now(), REFUSED_LINES))
            .ok()
            .flatten();
        if let Some(skipped) = allowed {
            tracing::warn!("{}", refused_line(refused, &peer_ip, skipped));
            hub.events.record(
                "stream_proof",
                json!({"refused": refused.word(), "source": source, "peer": peer_ip,
                       "skipped": skipped}),
            );
        }
        return refused.response();
    }
    tracing::debug!(peer = %peer_ip, source, "stream proof asked");
    let answer = hub
        .stream_proofs
        .answer(hub.live(MBC_INSTANCE), &hub.events)
        .await;
    (StatusCode::OK, Json(answer)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checks_go_lan_token_set_token() {
        assert_eq!(check(Origin::Local, true, true), Ok(()));
        assert_eq!(check(Origin::Internet, true, true), Err(Refused::Internet));
        assert_eq!(
            check(Origin::Internet, false, false),
            Err(Refused::Internet)
        );
        assert_eq!(check(Origin::Local, false, false), Err(Refused::NoToken));
        assert_eq!(check(Origin::Local, false, true), Err(Refused::NoToken));
        assert_eq!(check(Origin::Local, true, false), Err(Refused::Token));
    }

    #[test]
    fn each_refusal_has_its_status_code_and_word() {
        for (refused, status, word) in [
            (Refused::Internet, StatusCode::FORBIDDEN, "internet"),
            (
                Refused::NoToken,
                StatusCode::SERVICE_UNAVAILABLE,
                "no_token",
            ),
            (Refused::Token, StatusCode::UNAUTHORIZED, "token"),
        ] {
            assert_eq!(refused.response().status(), status);
            assert_eq!(refused.word(), word);
        }
        assert_eq!(REFUSED_LINES, 8);
        assert_eq!(PATH, "/api/mbc/stream-proof");
    }

    #[test]
    fn a_refusal_line_names_why_and_the_peer() {
        assert_eq!(
            refused_line(Refused::Token, "10.0.0.5", 2),
            "a stream proof request is refused (token) from 10.0.0.5; 2 left out before"
        );
    }
}
