//! A stopping hub closes every page (#246), also one whose session attaches
//! after the router has stopped: a page's session sends its `hello`, then
//! tells the router of it, so a stop between the two, or an upgrade still
//! open when the stop comes (it gets the listener's drain), reaches a
//! router that has closed its pages already. Such a page must be closed
//! too, never left open with nothing to tell it the hub went.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::support::{TestHub, http, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);

/// Reads `ws` until the hub's close or the end; panics when neither comes
/// within 3 s (`who` names the page), or on anything but a text message
/// before it.
async fn closed_within_3_s<S>(ws: &mut WebSocketStream<S>, who: &str)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match tokio::time::timeout(SECS_3, ws.next()).await {
            Ok(Some(Ok(Message::Text(_)))) => {}
            Ok(Some(Ok(Message::Close(_))) | None) => return,
            Ok(other) => panic!("{who} got {other:?} instead of the hub's close"),
            Err(_) => panic!("{who} saw no close within 3 s of the hub's stop"),
        }
    }
}

/// The head of a WebSocket upgrade of `path` on `addr`, but its last empty
/// line: the hub has not got the whole request yet.
fn upgrade_head(addr: SocketAddr, path: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    )
}

/// Reads an HTTP response's head from `stream` byte by byte (nothing past
/// it is taken: the WebSocket's first frame may follow at once).
async fn response_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        let n = tokio::time::timeout(SECS_3, stream.read(&mut byte))
            .await
            .expect("the hub answers the upgrade within 3 s")
            .unwrap();
        assert_eq!(n, 1, "the hub ended the upgrade's connection: {head:?}");
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

#[test]
fn a_page_whose_upgrade_ends_after_the_stop_is_closed() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let addr = hub.addr;
        let path = format!("/ws?token={}&proto=2", hub.token);
        // An attached page: its first message after the hello is the
        // router's, and its close proves the router has taken the stop.
        let mut attached = hub.client().await;
        attached
            .next_message(SECS_3)
            .await
            .expect("the router greets the attached page");
        let mut attached = attached.into_socket();
        // The late page's upgrade, all but its last line. The listener
        // accepts in order, so the answer on a connection opened after it
        // proves the hub accepted this one; in practice its task has also
        // read the head into HTTP/1 by then, a request in progress the
        // stop's drain waits for. A task first polled only after the stop's
        // notice would be cancelled in hyper-util's version read (the
        // connection ends with no answer, `response_head` says so): that
        // takes a worker preempted across a whole round trip, 0 of 1008
        // loaded runs.
        let mut late = TcpStream::connect(addr).await.unwrap();
        late.write_all(upgrade_head(addr, &path).as_bytes())
            .await
            .unwrap();
        let (code, _) = http(addr, "GET", "/api/version", None, None).await;
        assert_ne!(code, 0);
        let stopping = tokio::spawn(hub.stop());
        closed_within_3_s(&mut attached, "the attached page").await;
        // The router has stopped: now the late page's session starts.
        late.write_all(b"\r\n").await.unwrap();
        let head = response_head(&mut late).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let mut late = WebSocketStream::from_raw_socket(late, Role::Client, None).await;
        match tokio::time::timeout(SECS_3, late.next()).await {
            Ok(Some(Ok(Message::Text(hello)))) => {
                assert!(hello.contains("\"hello\""), "{hello}");
            }
            other => panic!("the late page got no hello: {other:?}"),
        }
        closed_within_3_s(&mut late, "the page attached after the stop").await;
        tokio::time::timeout(Duration::from_secs(10), stopping)
            .await
            .expect("the hub stops within 10 s")
            .unwrap();
    });
}
