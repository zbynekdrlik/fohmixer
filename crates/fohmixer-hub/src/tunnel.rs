//! cloudflared's readiness in `/api/status` (#17, iemmixer's
//! `tunnel_watch.rs` without the service restart: the hub runs as the band
//! user, who may not restart a service). Every [`POLL`] the hub reads
//! cloudflared's `/ready` (`[tunnel] ready_url`, cloudflared's `--metrics`
//! address): its ready edge connections, 0 with the reason when it does not
//! answer. A change is logged; the latest is in the status.

use std::sync::Arc;
use std::time::Duration;

use fohmixer_proto::client::TunnelStatus;
use serde::Deserialize;

use crate::http_client::HttpClient;
use crate::remote::RemoteState;

/// How often the readiness is read.
pub const POLL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct ReadyBody {
    #[serde(rename = "readyConnections")]
    ready_connections: u32,
}

/// The ready connections of a `/ready` answer: only a 200 with
/// `{"readyConnections": N}` counts (cloudflared answers 503 without an
/// edge connection).
pub fn parse_ready(status: u16, body: &[u8]) -> Result<u32, String> {
    if status != 200 {
        return Err(format!(
            "cloudflared answers {status}: {}",
            String::from_utf8_lossy(body)
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    serde_json::from_slice::<ReadyBody>(body)
        .map(|b| b.ready_connections)
        .map_err(|e| format!("cloudflared's /ready does not parse: {e}"))
}

/// One check at `now` (Unix seconds).
pub async fn check(http: &HttpClient, url: &str, now: i64) -> TunnelStatus {
    let read = match http.get(url).await {
        Ok(reply) => parse_ready(reply.status, &reply.body),
        Err(e) => Err(format!("cloudflared does not answer: {e:#}")),
    };
    TunnelStatus {
        ready_connections: *read.as_ref().unwrap_or(&0),
        error: read.err(),
        checked: Some(now),
    }
}

/// What to log when the status moves from `before` to `after`: `Some` on a
/// change of the connection count or of the error.
pub fn change_note(before: Option<&TunnelStatus>, after: &TunnelStatus) -> Option<String> {
    let same = before
        .is_some_and(|b| b.ready_connections == after.ready_connections && b.error == after.error);
    (!same).then(|| match &after.error {
        None => format!("tunnel: {} ready connections", after.ready_connections),
        Some(why) => format!("tunnel DOWN: {why}"),
    })
}

/// Checks `url` every [`POLL`] into `state`.
pub fn spawn(
    http: HttpClient,
    url: String,
    state: Arc<RemoteState>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let status = check(&http, &url, crate::auth::now_secs() as i64).await;
            if let Some(note) = change_note(state.tunnel().as_ref(), &status) {
                tracing::info!(url = %url, "{note}");
            }
            state.set_tunnel(status);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_200_with_the_count_is_ready() {
        assert_eq!(
            parse_ready(200, br#"{"status":200,"readyConnections":4}"#),
            Ok(4)
        );
        assert_eq!(parse_ready(200, br#"{"readyConnections":0}"#), Ok(0));
        assert_eq!(
            parse_ready(503, br#"{"status":503,"readyConnections":0}"#).unwrap_err(),
            r#"cloudflared answers 503: {"status":503,"readyConnections":0}"#
        );
        assert!(
            parse_ready(200, b"ok")
                .unwrap_err()
                .starts_with("cloudflared's /ready does not parse")
        );
        let long = "x".repeat(500);
        assert_eq!(
            parse_ready(500, long.as_bytes()).unwrap_err().len(),
            "cloudflared answers 500: ".len() + 200
        );
    }

    #[test]
    fn a_change_is_noted_once() {
        let up = |n| TunnelStatus {
            ready_connections: n,
            error: None,
            checked: Some(1),
        };
        let down = TunnelStatus {
            ready_connections: 0,
            error: Some("cloudflared does not answer: x".into()),
            checked: Some(2),
        };
        assert_eq!(
            change_note(None, &up(4)).unwrap(),
            "tunnel: 4 ready connections"
        );
        assert_eq!(change_note(Some(&up(4)), &up(4)), None);
        let later = TunnelStatus {
            checked: Some(99),
            ..up(4)
        };
        assert_eq!(
            change_note(Some(&up(4)), &later),
            None,
            "the check time is no change"
        );
        assert!(change_note(Some(&up(4)), &up(2)).is_some());
        assert_eq!(
            change_note(Some(&up(4)), &down).unwrap(),
            "tunnel DOWN: cloudflared does not answer: x"
        );
        assert_eq!(change_note(Some(&down), &down.clone()), None);
        let other_error = TunnelStatus {
            error: Some("other".into()),
            ..down.clone()
        };
        assert!(change_note(Some(&down), &other_error).is_some());
    }

    #[tokio::test]
    async fn a_check_reads_the_count_or_says_why_not() {
        use axum::routing::get;
        let app = axum::Router::new()
            .route(
                "/ready",
                get(|| async { r#"{"status":200,"readyConnections":4}"# }),
            )
            .route(
                "/down",
                get(|| async { (axum::http::StatusCode::SERVICE_UNAVAILABLE, "no edge") }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = HttpClient::new().unwrap();
        let up = check(&http, &format!("http://{addr}/ready"), 7).await;
        assert_eq!(
            up,
            TunnelStatus {
                ready_connections: 4,
                error: None,
                checked: Some(7)
            }
        );
        let down = check(&http, &format!("http://{addr}/down"), 8).await;
        assert_eq!(down.ready_connections, 0);
        assert_eq!(
            down.error.as_deref(),
            Some("cloudflared answers 503: no edge")
        );
        let gone = check(&http, "http://127.0.0.1:1/ready", 9).await;
        assert_eq!(gone.ready_connections, 0);
        assert!(
            gone.error
                .unwrap()
                .starts_with("cloudflared does not answer: ")
        );
        // The poller writes the state.
        let state = Arc::new(RemoteState::default());
        let task = spawn(http, format!("http://{addr}/ready"), Arc::clone(&state));
        for _ in 0..100 {
            if state.tunnel().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        task.abort();
        assert_eq!(state.tunnel().unwrap().ready_connections, 4);
    }
}
