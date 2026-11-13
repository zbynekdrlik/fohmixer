use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use super::*;
use crate::link::{LINK_TOKEN_SECS, link_token};

const WAIT: Duration = Duration::from_secs(5);

fn secret(byte: u8) -> LinkSecret {
    LinkSecret::from_bytes([byte; 32])
}

/// A fake FohMixer script: it sends `connect` for `mbc`, hands the test
/// every text it reads, and sends every text the test gives it.
struct FakeScript {
    port: u16,
    heard: mpsc::UnboundedReceiver<String>,
    say: mpsc::UnboundedSender<String>,
    pings: mpsc::UnboundedReceiver<()>,
}

async fn fake_script() -> FakeScript {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (heard_tx, heard) = mpsc::unbounded_channel();
    let (say, mut said) = mpsc::unbounded_channel::<String>();
    let (ping_tx, pings) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let connect = r#"{"event":"connect","data":{"instance":"mbc","set_name":"MBC"}}"#;
        ws.send(Message::Text(connect.into())).await.unwrap();
        loop {
            tokio::select! {
                frame = ws.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        let _ = heard_tx.send(text.to_string());
                    }
                    Some(Ok(Message::Ping(_))) => {
                        let _ = ping_tx.send(());
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                },
                text = said.recv() => match text {
                    Some(text) => {
                        if ws.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                    None => return,
                },
            }
        }
    });
    FakeScript {
        port,
        heard,
        say,
        pings,
    }
}

struct Served {
    addr: SocketAddr,
    state: Arc<LinkState>,
    task: JoinHandle<()>,
    tap: Tap,
    records: std::sync::mpsc::Receiver<Value>,
}

fn serve(script_port: u16, secret: Option<LinkSecret>) -> Served {
    let cfg = LinkCfg {
        listen: "127.0.0.1:0".parse().unwrap(),
        instance: "mbc".into(),
    };
    let (events, records) = EventLog::channel(64);
    let tap = tap();
    let (addr, state, task) = spawn(&cfg, script_port, secret, &tap, events).unwrap();
    Served {
        addr,
        state,
        task,
        tap,
        records,
    }
}

type Link = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

fn request(
    addr: SocketAddr,
    header: Option<String>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("ws://{addr}").into_client_request().unwrap();
    if let Some(header) = header {
        request
            .headers_mut()
            .insert("authorization", header.parse().unwrap());
    }
    request
}

fn bearer(secret: &LinkSecret) -> Option<String> {
    Some(format!(
        "Bearer {}",
        link_token(secret, crate::auth::now_secs())
    ))
}

async fn open(addr: SocketAddr, header: Option<String>) -> Result<Link, u16> {
    match tokio_tungstenite::connect_async(request(addr, header)).await {
        Ok((link, _)) => Ok(link),
        Err(WsError::Http(response)) => Err(response.status().as_u16()),
        Err(e) => panic!("{e}"),
    }
}

async fn next_text(link: &mut Link) -> String {
    loop {
        match tokio::time::timeout(WAIT, link.next()).await.unwrap() {
            Some(Ok(Message::Text(text))) => return text.to_string(),
            Some(Ok(Message::Close(frame))) => panic!("closed: {frame:?}"),
            Some(Ok(_)) => {}
            other => panic!("{other:?}"),
        }
    }
}

/// The next `what` record within [`WAIT`] (a tokio wait: the test's
/// runtime runs the server meanwhile).
async fn record_of(records: &std::sync::mpsc::Receiver<Value>, what: &str) -> Value {
    record_of_within(records, what, WAIT).await
}

#[tokio::test]
async fn a_link_relays_the_scripts_frames_both_ways() {
    let mut script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    let mut link = open(served.addr, bearer(&secret(1))).await.unwrap();
    let connect = next_text(&mut link).await;
    assert_eq!(
        connect, r#"{"event":"connect","data":{"instance":"mbc","set_name":"MBC"}}"#,
        "byte-equal"
    );
    let ask = r#"{"uuid":"c1","commands":[{"target":"live_set","name":"get_prop"}]}"#;
    link.send(Message::Text(ask.into())).await.unwrap();
    let heard = tokio::time::timeout(WAIT, script.heard.recv())
        .await
        .unwrap();
    assert_eq!(heard.as_deref(), Some(ask));
    let result = r#"{"event":"result","uuid":"c1","data":[{"ok":true}]}"#;
    script.say.send(result.into()).unwrap();
    assert_eq!(next_text(&mut link).await, result);
    let pushed = r#"{"event":"values","data":[{"key":"k","value":0.5}]}"#;
    script.say.send(pushed.into()).unwrap();
    assert_eq!(next_text(&mut link).await, pushed);
    // The link's keep-alive reaches the script (its silence watch).
    link.send(Message::Ping(Default::default())).await.unwrap();
    assert!(
        tokio::time::timeout(WAIT, script.pings.recv())
            .await
            .unwrap()
            .is_some()
    );
    let status = served.state.status();
    assert_eq!(status.links, 1);
    assert!(status.peer.is_some());
    assert!(status.connected_since_ms.is_some());
    assert_eq!(
        record_of(&served.records, "connected").await["ev"],
        "instance_link"
    );
    drop(link);
    let dropped = record_of(&served.records, "dropped").await;
    assert_eq!(dropped["why"], "the link closed");
    assert_eq!(dropped["warn"], true);
    // The open link is cleared.
    let deadline = std::time::Instant::now() + WAIT;
    while served.state.status().peer.is_some() {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    served.task.abort();
}

#[tokio::test]
async fn a_wrong_token_gets_401_and_a_record() {
    let script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    assert_eq!(open(served.addr, bearer(&secret(2))).await.err(), Some(401));
    let record = record_of(&served.records, "refused").await;
    assert_eq!(record["why"], "not a token of the link secret");
    assert_eq!(record["warn"], true);
    assert_eq!(open(served.addr, None).await.err(), Some(401));
    let old = format!(
        "Bearer {}",
        link_token(&secret(1), crate::auth::now_secs() - LINK_TOKEN_SECS - 5)
    );
    assert_eq!(open(served.addr, Some(old)).await.err(), Some(401));
    let status = served.state.status();
    assert_eq!(status.refusals, 3);
    assert!(
        status
            .last_refusal
            .as_deref()
            .is_some_and(|r| r.starts_with("the token expired (127.0.0.1:")),
        "{status:?}"
    );
    assert_eq!(status.links, 0);
    served.task.abort();
}

#[tokio::test]
async fn a_second_link_gets_409_while_one_is_open() {
    let script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    let mut first = open(served.addr, bearer(&secret(1))).await.unwrap();
    next_text(&mut first).await;
    assert_eq!(open(served.addr, bearer(&secret(1))).await.err(), Some(409));
    assert_eq!(record_of(&served.records, "refused").await["why"], "busy");
    // A caller without a token learns nothing of the open link.
    assert_eq!(open(served.addr, None).await.err(), Some(401));
    served.task.abort();
}

#[tokio::test]
async fn a_link_without_a_stored_secret_refuses_everything() {
    let script = fake_script().await;
    let served = serve(script.port, None);
    assert_eq!(open(served.addr, bearer(&secret(1))).await.err(), Some(401));
    assert_eq!(
        record_of(&served.records, "refused").await["why"],
        "no link secret"
    );
    served.task.abort();
}

#[tokio::test]
async fn a_dead_script_closes_the_link_with_1011() {
    // A port nothing listens on: bound, then dropped.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let served = serve(port, Some(secret(1)));
    let mut link = open(served.addr, bearer(&secret(1))).await.unwrap();
    let close = loop {
        match tokio::time::timeout(WAIT, link.next()).await.unwrap() {
            Some(Ok(Message::Close(frame))) => break frame,
            Some(Ok(_)) => {}
            other => panic!("{other:?}"),
        }
    };
    let close = close.expect("a close frame");
    assert_eq!(u16::from(close.code), 1011);
    assert_eq!(close.reason.as_str(), SCRIPT_UNREACHABLE);
    assert_eq!(
        record_of(&served.records, "dropped").await["why"],
        SCRIPT_UNREACHABLE
    );
    served.task.abort();
}

#[tokio::test]
async fn a_lufs_datagram_reaches_the_link_as_base64() {
    let script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    let mut link = open(served.addr, bearer(&secret(1))).await.unwrap();
    next_text(&mut link).await;
    // Subscribed once the link is open: wait for the relay's subscription.
    let deadline = std::time::Instant::now() + WAIT;
    while served.tap.receiver_count() == 0 {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let datagram: Arc<[u8]> = Arc::from(&b"/lufs\0\0\0,ii\0\x01\x02\x03"[..]);
    served.tap.send(Arc::clone(&datagram)).unwrap();
    let frame: Value = serde_json::from_str(&next_text(&mut link).await).unwrap();
    assert_eq!(frame["event"], "lufs");
    let osc = STANDARD.decode(frame["osc"].as_str().unwrap()).unwrap();
    assert_eq!(osc.as_slice(), &*datagram);
    served.task.abort();
}

#[tokio::test]
async fn a_link_that_never_reads_ends_after_the_stall() {
    let script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    // A client on a 4 KiB receive buffer that never reads.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let stream = socket.connect(served.addr).await.unwrap();
    let (link, _) = tokio_tungstenite::client_async(
        request(served.addr, bearer(&secret(1))),
        tokio_tungstenite::MaybeTlsStream::Plain(stream),
    )
    .await
    .unwrap();
    let _held = link;
    let value = format!(
        r#"{{"event":"values","data":[{{"key":"k","value":"{}"}}]}}"#,
        "x".repeat(64 * 1024)
    );
    let start = std::time::Instant::now();
    for _ in 0..256 {
        script.say.send(value.clone()).unwrap();
    }
    let dropped = record_of_within(&served.records, "dropped", Duration::from_secs(15)).await;
    assert_eq!(dropped["why"], "the link stalled");
    assert!(
        start.elapsed() >= LINK_STALL,
        "no earlier than the stall: {:?}",
        start.elapsed()
    );
    served.task.abort();
}

async fn record_of_within(
    records: &std::sync::mpsc::Receiver<Value>,
    what: &str,
    limit: Duration,
) -> Value {
    let deadline = std::time::Instant::now() + limit;
    loop {
        while let Ok(record) = records.try_recv() {
            if record["what"] == what {
                return record;
            }
        }
        assert!(std::time::Instant::now() < deadline, "no {what} record");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_request_that_is_no_upgrade_is_neither_a_link_nor_a_refusal() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let script = fake_script().await;
    let served = serve(script.port, Some(secret(1)));
    let mut plain = TcpStream::connect(served.addr).await.unwrap();
    plain
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(WAIT, plain.read_to_end(&mut answer)).await;
    let status = served.state.status();
    assert_eq!((status.links, status.refusals), (0, 0), "{status:?}");
    assert!(served.records.try_recv().is_err(), "no record");
    // A real link still opens afterwards.
    let mut link = open(served.addr, bearer(&secret(1))).await.unwrap();
    next_text(&mut link).await;
    served.task.abort();
}

#[test]
fn admit_checks_the_token_first_then_one_link() {
    assert_eq!(admit(false, Ok(())), Admission::Accept);
    assert_eq!(admit(true, Ok(())), Admission::Busy);
    assert_eq!(
        admit(true, Err(LinkRefusal::Expired)),
        Admission::Refuse(LinkRefusal::Expired)
    );
    assert_eq!(
        admit(false, Err(LinkRefusal::NoHeader)),
        Admission::Refuse(LinkRefusal::NoHeader)
    );
}

#[test]
fn a_lufs_frame_is_its_datagram_in_base64() {
    assert_eq!(
        lufs_frame(&[0, 1, 2, 255]),
        r#"{"event":"lufs","osc":"AAEC/w=="}"#
    );
}

#[test]
fn the_records_fields() {
    let peer: SocketAddr = "10.0.0.6:50000".parse().unwrap();
    assert_eq!(
        link_fields("connected", &peer, None),
        serde_json::json!({"what": "connected", "peer": "10.0.0.6:50000"})
    );
    assert_eq!(
        link_fields("refused", &peer, Some("busy")),
        serde_json::json!({"what": "refused", "peer": "10.0.0.6:50000", "why": "busy", "warn": true})
    );
}

#[test]
fn the_constants() {
    assert_eq!(LINK_STALL, Duration::from_secs(3));
    assert_eq!(LUFS_TAP, 256);
    assert_eq!(REFUSED_LINES, 8);
}
