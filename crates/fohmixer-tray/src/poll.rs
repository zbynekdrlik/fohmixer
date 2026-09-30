//! The hub's state as the tray sees it: `GET /api/version` on the hub's
//! local address, the one endpoint it answers without a login (the
//! instances and the layout, `/api/status`, need a token). An answer with
//! the hub's `VersionInfo` is a running hub; anything else is a hub that does
//! not answer, with a short reason for the tooltip and the whole error for
//! the log.

use std::time::Duration;

use fohmixer_proto::VersionInfo;

/// How long one poll may take (connect, answer, body): the hub answers
/// `/api/version` at once, and the next poll comes anyway.
pub const TIMEOUT: Duration = Duration::from_secs(2);
/// The time between two polls.
pub const EVERY: Duration = Duration::from_secs(5);

/// The hub's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubState {
    /// No poll has answered yet (the tray just started).
    Unknown,
    /// The hub answered with its version.
    Up(VersionInfo),
    /// The hub did not answer: `reason` for the tooltip (short, stable while
    /// the problem lasts), `detail` for the log.
    Down { reason: String, detail: String },
}

/// The reason for a hub that does not answer at all.
pub const NO_ANSWER: &str = "no answer";
/// The reason for an answer that is not the hub's version.
pub const NOT_THE_HUB: &str = "the answer is not the hub's version";

/// A running hub's `/api/version` body, or why it is not one.
pub fn parse_version(body: &str) -> Result<VersionInfo, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// The state an HTTP answer gives: a success status with the hub's version
/// is `Up`; another status or body is `Down`.
pub fn from_answer(status: u16, body: &str) -> HubState {
    if !(200..300).contains(&status) {
        return HubState::Down {
            reason: format!("HTTP {status}"),
            detail: format!("HTTP {status}: {}", first_chars(body, 200)),
        };
    }
    match parse_version(body) {
        Ok(info) => HubState::Up(info),
        Err(e) => HubState::Down {
            reason: NOT_THE_HUB.to_string(),
            detail: format!("{e}: {}", first_chars(body, 200)),
        },
    }
}

/// At most `n` characters of `text` (a body quoted in a log line).
fn first_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// One poll of `url` (the hub's `/api/version`), within `timeout`. No proxy:
/// the hub is on this PC.
pub fn poll(url: &str, timeout: Duration) -> HubState {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    match agent.get(url).call() {
        Ok(mut response) => {
            let status = response.status().as_u16();
            match response.body_mut().read_to_string() {
                Ok(body) => from_answer(status, &body),
                Err(e) => HubState::Down {
                    reason: NO_ANSWER.to_string(),
                    detail: format!("HTTP {status}, the body did not arrive: {e}"),
                },
            }
        }
        Err(e) => HubState::Down {
            reason: NO_ANSWER.to_string(),
            detail: format!("GET {url}: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn info() -> VersionInfo {
        VersionInfo {
            version: "0.1.0-dev.25".to_string(),
            git_hash: "abc1234".to_string(),
            branch: "master".to_string(),
            build_time: "1790000000".to_string(),
        }
    }

    /// A one-shot HTTP server on 127.0.0.1 answering `response` (the whole
    /// raw answer); returns its base URL and the request it received.
    fn serve_once(response: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            // Read up to the end of the request's head (a GET has no body).
            for _ in 0..64 {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                if n == 0 || request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            // A test that does not read the request has dropped its end.
            let _ = tx.send(String::from_utf8_lossy(&request).into_owned());
        });
        (format!("http://127.0.0.1:{port}"), rx)
    }

    fn answer(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_version_answer_is_a_running_hub() {
        let body = serde_json::to_string(&info()).unwrap();
        let (base, request) = serve_once(answer("200 OK", &body));
        let state = poll(&format!("{base}/api/version"), TIMEOUT);
        assert_eq!(state, HubState::Up(info()));
        let request = request.recv().unwrap();
        assert!(
            request.starts_with("GET /api/version HTTP/1.1\r\n"),
            "{request}"
        );
    }

    #[test]
    fn an_error_status_is_a_hub_that_does_not_answer() {
        let (base, _request) = serve_once(answer("503 Service Unavailable", "{\"error\":\"x\"}"));
        match poll(&format!("{base}/api/version"), TIMEOUT) {
            HubState::Down { reason, detail } => {
                assert_eq!(reason, "HTTP 503");
                assert!(detail.contains("{\"error\":\"x\"}"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_body_that_is_not_the_version_is_not_the_hub() {
        let (base, _request) = serve_once(answer("200 OK", "<html>hello</html>"));
        match poll(&format!("{base}/api/version"), TIMEOUT) {
            HubState::Down { reason, detail } => {
                assert_eq!(reason, NOT_THE_HUB);
                assert!(detail.contains("<html>hello</html>"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_refused_connection_is_no_answer() {
        // Port 1 on 127.0.0.1: nothing listens there on the test runners.
        match poll("http://127.0.0.1:1/api/version", TIMEOUT) {
            HubState::Down { reason, detail } => {
                assert_eq!(reason, NO_ANSWER);
                assert!(
                    detail.starts_with("GET http://127.0.0.1:1/api/version: "),
                    "{detail}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_hub_that_never_answers_is_no_answer_within_the_timeout() {
        // Accepted by the kernel (the listener's backlog), never answered.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/api/version",
            listener.local_addr().unwrap().port()
        );
        let started = std::time::Instant::now();
        let state = poll(&url, Duration::from_millis(300));
        let took = started.elapsed();
        assert!(
            matches!(&state, HubState::Down { reason, .. } if reason == NO_ANSWER),
            "{state:?}"
        );
        assert!(took < Duration::from_secs(5), "{took:?}");
        drop(listener);
    }

    #[test]
    fn from_answer_reads_the_status_boundaries() {
        let body = serde_json::to_string(&info()).unwrap();
        assert_eq!(from_answer(200, &body), HubState::Up(info()));
        assert_eq!(from_answer(299, &body), HubState::Up(info()));
        assert!(
            matches!(from_answer(300, &body), HubState::Down { reason, .. } if reason == "HTTP 300")
        );
        assert!(
            matches!(from_answer(199, &body), HubState::Down { reason, .. } if reason == "HTTP 199")
        );
    }

    #[test]
    fn a_long_body_is_cut_in_the_detail() {
        let body = "x".repeat(500);
        match from_answer(500, &body) {
            HubState::Down { detail, .. } => {
                assert_eq!(detail, format!("HTTP 500: {}", "x".repeat(200)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_version_reads_the_hubs_answer() {
        let body = r#"{"version":"0.1.0-dev.25","git_hash":"abc1234","branch":"master","build_time":"1790000000"}"#;
        assert_eq!(parse_version(body), Ok(info()));
        assert!(parse_version("{}").is_err());
    }

    #[test]
    fn the_poll_cadence_is_every_five_seconds_within_two() {
        assert_eq!(EVERY, Duration::from_secs(5));
        assert_eq!(TIMEOUT, Duration::from_secs(2));
    }
}
