//! The gate of the automation routes (#207 `POST /api/scene`, #267 `GET
//! /api/mbc/stream-proof`): one order of checks for every caller that
//! shows the automation token (`crate::automation`) instead of a login.
//!
//! [`gate`] (pure): an internet request is 403 (`LAN_ONLY`) before anything
//! else; a hub without a token 503 (`NO_AUTOMATION`, naming the command
//! that sets one); a missing or wrong token 401 (`UNAUTHORIZED`; the PIN's
//! session token is no automation token). [`note`] writes a refusal's
//! record and warn line, at most `per_gap` per `ration::GAP` from the
//! route's own ration, with the count left out before it.

use std::sync::Mutex;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use crate::access::Origin;
use crate::auth::error_response;
use crate::events::EventLog;
use crate::ration::Ration;

/// Why the gate refuses a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// An internet request (403).
    Internet,
    /// The hub has no automation token (503).
    NoToken,
    /// A missing or wrong token (401).
    Token,
}

impl Gate {
    /// Its word in the record.
    pub fn word(self) -> &'static str {
        match self {
            Gate::Internet => "internet",
            Gate::NoToken => "no_token",
            Gate::Token => "token",
        }
    }

    /// Its answer.
    pub fn response(self) -> Response {
        match self {
            Gate::Internet => error_response(
                StatusCode::FORBIDDEN,
                "LAN_ONLY",
                "Automation requests are taken on the local network only",
            ),
            Gate::NoToken => error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "NO_AUTOMATION",
                "This hub has no automation token: set one with `fohmixer-hub automation set-token` and restart it",
            ),
            Gate::Token => error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Send the automation token as Authorization: Bearer <token>",
            ),
        }
    }
}

/// The checks in their order: the LAN, a token set, the token.
pub fn gate(origin: Origin, token_set: bool, token_ok: bool) -> Result<(), Gate> {
    if origin == Origin::Internet {
        return Err(Gate::Internet);
    }
    if !token_set {
        return Err(Gate::NoToken);
    }
    if !token_ok {
        return Err(Gate::Token);
    }
    Ok(())
}

/// One refusal of a route: its record kind (`ev`), why (`word`), the
/// opener's `source` and `peer`, and its warn line given the count left
/// out before it.
pub struct Refusal<'a, F: FnOnce(u64) -> String> {
    pub ev: &'a str,
    pub word: &'a str,
    pub source: &'a str,
    pub peer: &'a str,
    pub line: F,
}

/// Writes `refusal`'s record and warn line when `ration` allows one now.
pub fn note<F: FnOnce(u64) -> String>(
    ration: &Mutex<Ration>,
    per_gap: u32,
    events: &EventLog,
    refusal: Refusal<'_, F>,
) {
    let allowed = ration
        .lock()
        .map(|mut r| r.take(Instant::now(), per_gap))
        .ok()
        .flatten();
    if let Some(skipped) = allowed {
        tracing::warn!("{}", (refusal.line)(skipped));
        events.record(
            refusal.ev,
            json!({"refused": refusal.word, "source": refusal.source, "peer": refusal.peer,
                   "skipped": skipped}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checks_go_lan_token_set_token() {
        assert_eq!(gate(Origin::Local, true, true), Ok(()));
        assert_eq!(gate(Origin::Internet, true, true), Err(Gate::Internet));
        assert_eq!(gate(Origin::Internet, false, false), Err(Gate::Internet));
        assert_eq!(gate(Origin::Local, false, false), Err(Gate::NoToken));
        assert_eq!(gate(Origin::Local, false, true), Err(Gate::NoToken));
        assert_eq!(gate(Origin::Local, true, false), Err(Gate::Token));
    }

    #[test]
    fn each_refusal_has_its_status_code_and_word() {
        for (refused, status, word) in [
            (Gate::Internet, StatusCode::FORBIDDEN, "internet"),
            (Gate::NoToken, StatusCode::SERVICE_UNAVAILABLE, "no_token"),
            (Gate::Token, StatusCode::UNAUTHORIZED, "token"),
        ] {
            assert_eq!(refused.response().status(), status);
            assert_eq!(refused.word(), word);
        }
    }

    #[test]
    fn a_refusal_is_noted_within_its_ration() {
        let (events, records) = EventLog::channel(16);
        let ration = Mutex::new(Ration::default());
        for _ in 0..3 {
            note(
                &ration,
                2,
                &events,
                Refusal {
                    ev: "x",
                    word: "token",
                    source: "lan",
                    peer: "10.0.0.5",
                    line: |skipped| format!("refused; {skipped} left out"),
                },
            );
        }
        let first = records.try_recv().unwrap();
        assert_eq!(first["ev"], "x");
        assert_eq!(first["refused"], "token");
        assert_eq!(first["source"], "lan");
        assert_eq!(first["peer"], "10.0.0.5");
        assert_eq!(first["skipped"], 0);
        assert!(records.try_recv().is_ok());
        assert!(records.try_recv().is_err(), "the third is left out");
    }
}
