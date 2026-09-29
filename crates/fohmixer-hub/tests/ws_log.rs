//! The socket lines end to end (#9): a WebSocket opened on the hub's real
//! listener leaves `client connected` and `client disconnected` lines that
//! name its peer, the LAN and the page's host. They are read back from the
//! hub's log through a global subscriber. This is the only test of this
//! binary, and nextest runs every test in its own process. The internet
//! path and the page host's cleaning and quoting are `ws.rs`'s unit tests.
//! No Live host.

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::SinkExt;
use support::{TestHub, runtime};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

/// The hub's log, as this test reads it back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The first line of the log that `matches`, waited for up to 5 s.
    async fn line(&self, matches: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = self.text();
            if let Some(line) = text.lines().find(|&l| matches(l)) {
                return line.to_string();
            }
            assert!(Instant::now() < deadline, "no such line in:\n{text}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[test]
fn a_socket_names_its_peer_the_lan_and_its_page_host_in_the_log() {
    let captured = Captured::default();
    let writer = captured.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .finish(),
    )
    .expect("the only subscriber of this test binary");
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        // A page opened at the hub's own address, as a LAN browser opens it.
        let mut request = hub.ws_url().as_str().into_client_request().unwrap();
        let page = format!("http://{}", hub.addr);
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_str(&page).unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("the WebSocket opens");
        let fields = format!(r#" peer=127.0.0.1 source=lan origin="{}""#, hub.addr);
        let connected = captured
            .line(|l| l.contains("client connected client=") && l.ends_with(&fields))
            .await;
        // The disconnect line names the same socket.
        let client = connected
            .split_whitespace()
            .find(|word| word.starts_with("client="))
            .unwrap()
            .to_string();
        ws.send(Message::Close(None)).await.unwrap();
        captured
            .line(|l| l.contains(&format!("client disconnected {client} ")) && l.ends_with(&fields))
            .await;
        hub.stop().await;
    });
}
