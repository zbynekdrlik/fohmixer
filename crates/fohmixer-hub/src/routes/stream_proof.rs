//! `GET /api/mbc/stream-proof` (#267): camera-box's stream-test guard asks,
//! before each test, whether the MBC stream mix carries only HAND 1 sync,
//! with the automation token (#207) or the stream proof's read-only token
//! (#274: `fohmixer-hub stream-proof set-token`; it opens this route only)
//! as `Authorization: Bearer <token>`. The answer is `fohmixer_proto::stream_proof::Answer` (the walk and its
//! cache: `crate::stream_proof`); it is 200 whatever the proof says, also
//! `ok: false` (camera-box then stays strict).
//!
//! The checks are the automation gate's (`automation_gate::gate`, as
//! `POST /api/scene`'s): an internet request 403 (`LAN_ONLY`), a hub
//! with neither token 503 (`NO_AUTOMATION`), a missing or wrong token 401
//! (`UNAUTHORIZED`; the PIN's session token is no automation token). Every
//! refusal is one `stream_proof` record with `refused` and one warn line,
//! at most [`REFUSED_LINES`] per `ration::GAP` (`automation_gate::note`).

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use fohmixer_proto::roles::MBC_INSTANCE;

use super::automation_gate::{self, Gate, Refusal};
use crate::Hub;
use crate::access;
use crate::auth::bearer;
use crate::automation::AutomationToken;

/// The route's path.
pub const PATH: &str = "/api/mbc/stream-proof";
/// The most refusal records and lines per `ration::GAP`.
pub const REFUSED_LINES: u32 = 8;

/// The warn line of a refusal (`skipped`: the refusals left out before it).
pub fn refused_line(refused: Gate, peer: &str, skipped: u64) -> String {
    format!(
        "a stream proof request is refused ({}) from {peer}; {skipped} left out before",
        refused.word()
    )
}

/// Whether any of the route's `tokens` is set, and whether `presented` is
/// one of them: the automation token (#207) or the read-only one (#274).
pub fn takes(tokens: [Option<&AutomationToken>; 2], presented: Option<&str>) -> (bool, bool) {
    let set = tokens.iter().any(Option::is_some);
    let ok = tokens.iter().flatten().any(|t| t.accepts(presented));
    (set, ok)
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
    let tokens = [hub.automation.as_ref(), hub.stream_proof_token.as_ref()];
    let (set, ok) = takes(tokens, bearer(&headers));
    if let Err(refused) = automation_gate::gate(origin, set, ok) {
        automation_gate::note(
            &hub.stream_proofs.refusals,
            REFUSED_LINES,
            &hub.events,
            Refusal {
                ev: "stream_proof",
                word: refused.word(),
                source,
                peer: &peer_ip,
                line: |skipped| refused_line(refused, &peer_ip, skipped),
            },
        );
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
    fn either_token_opens_the_route_and_none_set_is_no_token() {
        let a = AutomationToken::of(&"a".repeat(32)).unwrap();
        let b = AutomationToken::of(&"b".repeat(32)).unwrap();
        let (a_text, b_text, c_text) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
        assert_eq!(takes([None, None], Some(&a_text)), (false, false));
        assert_eq!(takes([Some(&a), None], Some(&a_text)), (true, true));
        assert_eq!(takes([Some(&a), None], Some(&b_text)), (true, false));
        assert_eq!(takes([None, Some(&b)], Some(&b_text)), (true, true));
        assert_eq!(takes([None, Some(&b)], Some(&a_text)), (true, false));
        assert_eq!(takes([Some(&a), Some(&b)], Some(&a_text)), (true, true));
        assert_eq!(takes([Some(&a), Some(&b)], Some(&b_text)), (true, true));
        assert_eq!(takes([Some(&a), Some(&b)], Some(&c_text)), (true, false));
        assert_eq!(takes([Some(&a), Some(&b)], None), (true, false));
        // The read-only token passes the token checks only on the LAN.
        let (set, ok) = takes([None, Some(&b)], Some(&b_text));
        assert_eq!(
            automation_gate::gate(access::Origin::Internet, set, ok),
            Err(Gate::Internet)
        );
        assert_eq!(
            automation_gate::gate(access::Origin::Local, set, ok),
            Ok(())
        );
    }

    #[test]
    fn a_refusal_line_names_why_and_the_peer() {
        assert_eq!(
            refused_line(Gate::Token, "10.0.0.5", 2),
            "a stream proof request is refused (token) from 10.0.0.5; 2 left out before"
        );
        assert_eq!(
            refused_line(Gate::Internet, "10.0.0.5", 0),
            "a stream proof request is refused (internet) from 10.0.0.5; 0 left out before"
        );
        assert_eq!(REFUSED_LINES, 8);
        assert_eq!(PATH, "/api/mbc/stream-proof");
    }
}
