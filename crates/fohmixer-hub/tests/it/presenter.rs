//! Presenter's stage tab (#90) through a served hub and a fake Presenter
//! (`support/presenter.rs`): the pages are told Presenter's layouts and the
//! one on stage, follow anyone's switch, switch it with a held key (each
//! switch recorded with the page that asked), see Presenter go down and
//! come back, and read Presenter's stage page and its socket on the hub's
//! own origin, never writing anything of Presenter's through it. Host-free.

use std::net::SocketAddr;
use std::time::Duration;

use fohmixer_hub::config::{Config, PresenterCfg};
use fohmixer_proto::client::{ClientMsg, ServerMsg, StageLayout};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::Message;

use crate::support::presenter::{FakePresenter, LAYOUTS, PAGE};
use crate::support::{Client, TestHub};

const WAIT: Duration = Duration::from_secs(10);

async fn hub(dir: &std::path::Path, url: Option<String>) -> TestHub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.lufs.port = 0;
    config.presenter = url.map(|url| PresenterCfg { url });
    TestHub::start_config(config).await
}

fn layouts() -> Vec<StageLayout> {
    LAYOUTS
        .iter()
        .map(|(code, name)| StageLayout {
            code: (*code).into(),
            name: (*name).into(),
            description: format!("{name} on the stage display"),
        })
        .collect()
}

/// The next `presenter` message that satisfies `check`.
async fn stage_until(
    page: &mut Client,
    check: impl Fn(Option<bool>, &[StageLayout], Option<&str>) -> bool,
) -> ServerMsg {
    page.wait(WAIT, |msg| match msg {
        ServerMsg::Presenter {
            online,
            layouts,
            current,
        } if check(*online, layouts, current.as_deref()) => Some(msg.clone()),
        _ => None,
    })
    .await
}

/// The page's `presenter_ack` of `seq`.
async fn ack(page: &mut Client, seq: u64) -> ServerMsg {
    page.wait(WAIT, |msg| match msg {
        ServerMsg::PresenterAck { seq: s, .. } if *s == seq => Some(msg.clone()),
        _ => None,
    })
    .await
}

/// One raw HTTP/1.1 request to `addr`: the status, the head (lower case)
/// and the body.
async fn raw(addr: SocketAddr, request: &str) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(WAIT, stream.read_to_end(&mut bytes))
        .await
        .expect("an answer")
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head.to_ascii_lowercase(), body.to_string())
}

fn get(addr: SocketAddr, path: &str, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{extra}\r\n")
}

#[tokio::test]
async fn the_pages_are_told_the_layouts_the_one_on_stage_and_every_switch_made_in_presenter() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let mut first = hub.client().await;
    let told = stage_until(&mut first, |online, _, _| online == Some(true)).await;
    assert_eq!(
        told,
        ServerMsg::Presenter {
            online: Some(true),
            layouts: layouts(),
            current: Some("worship".into()),
        }
    );
    let mut second = hub.client().await;
    stage_until(&mut second, |online, list, current| {
        online == Some(true) && list.len() == 3 && current == Some("worship")
    })
    .await;
    // The operator switches in Presenter: both pages follow.
    fake.switch("preach");
    for page in [&mut first, &mut second] {
        stage_until(page, |_, _, current| current == Some("preach")).await;
    }
    let status = hub
        .status_until(WAIT, |s| {
            s.presenter
                .as_ref()
                .is_some_and(|p| p.current.as_deref() == Some("preach"))
        })
        .await;
    let presenter = status.presenter.unwrap();
    assert_eq!(presenter.url, fake.url());
    assert_eq!(presenter.online, Some(true));
    assert_eq!(presenter.layouts, 3);
    assert_eq!(presenter.last_error, None);
    // The hub's own socket is a preview: Presenter never counts it.
    let link = hub
        .events_until(WAIT, |r| r.iter().any(|r| r["ev"] == "presenter_link"))
        .await;
    let up: Vec<&Value> = link
        .iter()
        .filter(|r| r["ev"] == "presenter_link")
        .collect();
    assert_eq!(up[0]["up"], true, "{up:?}");
    first.close().await;
    second.close().await;
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn a_page_switches_the_stage_with_its_held_key_and_each_switch_is_recorded() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let mut page = hub.client().await;
    let mut other = hub.client().await;
    stage_until(&mut page, |online, _, _| online == Some(true)).await;
    stage_until(&mut other, |online, _, _| online == Some(true)).await;
    page.send(&ClientMsg::PresenterSwitch {
        code: "timer".into(),
        seq: 1,
    })
    .await;
    assert_eq!(
        ack(&mut page, 1).await,
        ServerMsg::PresenterAck {
            seq: 1,
            code: "timer".into(),
            ok: true,
            error: None,
        }
    );
    assert_eq!(fake.current(), "timer");
    stage_until(&mut other, |_, _, current| current == Some("timer")).await;
    // A code Presenter does not list never reaches it.
    page.send(&ClientMsg::PresenterSwitch {
        code: "nope".into(),
        seq: 2,
    })
    .await;
    assert_eq!(
        ack(&mut page, 2).await,
        ServerMsg::PresenterAck {
            seq: 2,
            code: "nope".into(),
            ok: false,
            error: Some("Presenter does not list \"nope\"".into()),
        }
    );
    let posts = fake
        .seen()
        .iter()
        .filter(|s| *s == "POST /stage/layout")
        .count();
    assert_eq!(posts, 1, "{:?}", fake.seen());
    let records = hub
        .events_until(WAIT, |r| {
            r.iter().filter(|r| r["ev"] == "presenter_switch").count() == 2
        })
        .await;
    let switches: Vec<&Value> = records
        .iter()
        .filter(|r| r["ev"] == "presenter_switch")
        .collect();
    assert_eq!(switches[0]["ok"], true);
    assert_eq!(switches[0]["code"], "timer");
    assert_eq!(switches[0]["from"], "worship");
    assert_eq!(switches[0]["on_stage"], "timer");
    assert_eq!(switches[0]["peer"], "127.0.0.1");
    assert!(switches[0]["ms"].is_u64(), "{}", switches[0]);
    assert_eq!(switches[1]["ok"], false);
    assert!(switches[1]["ms"].is_null());
    page.close().await;
    other.close().await;
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn the_pages_see_presenter_go_down_and_come_back_by_itself() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let mut page = hub.client().await;
    stage_until(&mut page, |online, _, _| online == Some(true)).await;
    let addr = fake.stop().await;
    // Down keeps the layouts the hub last read.
    let down = stage_until(&mut page, |online, _, _| online == Some(false)).await;
    assert_eq!(
        down,
        ServerMsg::Presenter {
            online: Some(false),
            layouts: layouts(),
            current: Some("worship".into()),
        }
    );
    page.send(&ClientMsg::PresenterSwitch {
        code: "timer".into(),
        seq: 5,
    })
    .await;
    assert_eq!(
        ack(&mut page, 5).await,
        ServerMsg::PresenterAck {
            seq: 5,
            code: "timer".into(),
            ok: false,
            error: Some("Presenter can't be reached".into()),
        }
    );
    let status = hub.status().await.presenter.unwrap();
    assert_eq!(status.online, Some(false));
    assert!(status.last_error.is_some(), "{status:?}");
    // The stage page's frame gets the hub's own page while it is down.
    let (code, head, body) = raw(
        hub.addr,
        &get(hub.addr, "/stage?preview=1", "Sec-Fetch-Dest: iframe\r\n"),
    )
    .await;
    assert_eq!(code, 502, "{head}");
    assert!(body.contains("Presenter can't be reached"), "{body}");
    assert!(head.contains("x-frame-options: sameorigin"), "{head}");
    // Back on the same address, with another layout on stage.
    let fake = FakePresenter::start_on(addr, "preach").await;
    stage_until(&mut page, |online, _, current| {
        online == Some(true) && current == Some("preach")
    })
    .await;
    let records = hub
        .events_until(WAIT, |r| {
            r.iter().filter(|r| r["ev"] == "presenter_link").count() >= 3
        })
        .await;
    let link: Vec<&Value> = records
        .iter()
        .filter(|r| r["ev"] == "presenter_link")
        .collect();
    assert_eq!(link[0]["up"], true);
    assert_eq!(link[1]["up"], false);
    assert_eq!(link[1]["failures"], 2, "{}", link[1]);
    assert!(link[1]["error"].is_string(), "{}", link[1]);
    assert_eq!(link[2]["up"], true);
    page.close().await;
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn the_stage_video_starts_and_ends_through_whep_and_nothing_else_is_written() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let addr = hub.addr;
    let offer = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\n";
    let request = |method: &str, path: &str, body: &str| {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    // The offer reaches Presenter as it was sent; its answer comes back
    // whole, its Location a path on fohmixer's own origin.
    let (code, head, body) = raw(addr, &request("POST", "/whep/ndi-1", offer)).await;
    assert_eq!(code, 201, "{head}");
    assert_eq!(body, format!("answer to: {offer}"));
    assert!(head.contains("content-type: application/sdp"), "{head}");
    assert!(head.contains("x-offer-type: application/sdp"), "{head}");
    assert!(head.contains("location: /whep/ndi-1/s1\r\n"), "{head}");
    assert!(
        !head.contains(&fake.url()),
        "Presenter's address never reaches the page: {head}"
    );
    let (code, head, _) = raw(addr, &request("PATCH", "/whep/ndi-1/s1", "a=candidate")).await;
    assert_eq!(code, 204, "{head}");
    let (code, head, _) = raw(addr, &request("DELETE", "/whep/ndi-1/s1", "")).await;
    assert_eq!(code, 200, "{head}");
    // A write anywhere else is still the hub's 405, WHEP's methods included.
    for (method, path) in [
        ("POST", "/stage/layout"),
        ("PATCH", "/ndi/x"),
        ("DELETE", "/stage/state"),
        ("PUT", "/whep/ndi-1"),
    ] {
        let (code, head, _) = raw(addr, &request(method, path, "{}")).await;
        assert_eq!(code, 405, "{method} {path}: {head}");
    }
    let seen = fake.seen();
    assert_eq!(
        seen.iter()
            .filter(|s| !s.starts_with("GET "))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "POST /whep/ndi-1".to_string(),
            "PATCH /whep/ndi-1/s1".to_string(),
            "DELETE /whep/ndi-1/s1".to_string(),
        ],
        "only WHEP's requests reached Presenter: {seen:?}"
    );
}

#[tokio::test]
async fn the_stage_page_is_read_on_the_hubs_origin_and_nothing_of_presenter_is_written() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let addr = hub.addr;
    let (code, head, body) = raw(addr, &get(addr, "/stage?preview=1", "")).await;
    assert_eq!(code, 200, "{head}");
    assert_eq!(body, PAGE);
    assert!(head.contains("x-frame-options: sameorigin"), "{head}");
    assert!(
        head.contains("content-security-policy: frame-ancestors 'self'"),
        "{head}"
    );
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    assert!(!head.contains("x-frame-options: deny"), "{head}");
    let (code, _, body) = raw(addr, &get(addr, "/ui-pkg/presenter-ui-0123456789ab.js", "")).await;
    assert_eq!((code, body.as_str()), (200, "/* app */"));
    let (code, _, body) = raw(addr, &get(addr, "/stage-displays", "")).await;
    assert_eq!(code, 200);
    assert_eq!(
        serde_json::from_str::<Vec<StageLayout>>(&body).unwrap(),
        layouts()
    );
    // No write passes, the switch's own endpoint included.
    for path in ["/stage/state", "/stage/layout"] {
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        let (code, head, _) = raw(addr, &request).await;
        assert_eq!(code, 405, "{path}: {head}");
        assert!(head.contains("allow: get, head"), "{head}");
    }
    // A dot segment never leaves the list on Presenter's side.
    let (code, _, _) = raw(addr, &get(addr, "/stage/../ui/settings", "")).await;
    assert_eq!(code, 400);
    let (code, _, _) = raw(addr, &get(addr, "/ui-pkg/%2E%2e/x", "")).await;
    assert_eq!(code, 400);
    // A socket only on Presenter's socket path.
    let upgrade = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
    let (code, _, _) = raw(addr, &get(addr, "/stage", upgrade)).await;
    assert_eq!(code, 400);
    let seen = fake.seen();
    assert!(
        seen.iter().all(|s| s.starts_with("GET ")),
        "only reads reached Presenter: {seen:?}"
    );
    assert!(seen.contains(&"GET /stage".to_string()), "{seen:?}");
    // fohmixer's own page keeps its own headers.
    let (code, head, _) = raw(addr, &get(addr, "/", "")).await;
    assert_eq!(code, 200);
    assert!(head.contains("x-frame-options: deny"), "{head}");
    assert!(head.contains("frame-src 'self'"), "{head}");
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn presenters_paths_keep_fohmixers_host_access_and_origin_checks() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let addr = hub.addr;
    // The hub's own link first: its socket and its two reads.
    let mut page = hub.client().await;
    stage_until(&mut page, |online, _, _| online == Some(true)).await;
    let linked = fake.seen().len();
    // An internet request (the tunnel's header) without Access: refused,
    // whatever the path, and Presenter never asked.
    for path in ["/stage", "/stage-displays", "/ui-pkg/a.js"] {
        let (code, _, body) =
            raw(addr, &get(addr, path, "cf-connecting-ip: 203.0.113.7\r\n")).await;
        assert_eq!(code, 403, "{path}");
        assert!(body.contains("ACCESS_DENIED"), "{path}: {body}");
    }
    // A Host the hub does not answer for (DNS rebinding): 421.
    let request = "GET /stage HTTP/1.1\r\nHost: rebind.example.org\r\nConnection: close\r\n\r\n";
    let (code, _, _) = raw(addr, request).await;
    assert_eq!(code, 421);
    // Another site's page may not open the stage page's socket.
    let upgrade = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                   Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nOrigin: http://evil.example.org\r\n";
    let (code, _, _) = raw(
        addr,
        &get(addr, "/live/ws?surface=stage&preview=1", upgrade),
    )
    .await;
    assert_eq!(code, 403);
    assert_eq!(
        fake.seen().len(),
        linked,
        "nothing refused reached Presenter: {:?}",
        fake.seen()
    );
    // Presenter's content other than its stage page is sandboxed as a
    // document; the stage page is not.
    let (code, head, _) = raw(addr, &get(addr, "/stage-displays", "")).await;
    assert_eq!(code, 200);
    assert!(
        head.contains("content-security-policy: sandbox; default-src 'none'"),
        "{head}"
    );
    let (_, head, _) = raw(addr, &get(addr, "/stage", "")).await;
    assert!(!head.contains("sandbox"), "{head}");
    page.close().await;
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn the_stage_pages_socket_is_tunneled_to_presenter_both_ways() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let mut page = hub.client().await;
    stage_until(&mut page, |online, _, _| online == Some(true)).await;
    let url = format!("ws://{}/live/ws?surface=stage&preview=1", hub.addr);
    let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    socket.send(Message::Text("hello".into())).await.unwrap();
    let mut texts = Vec::new();
    let read = async {
        while let Some(Ok(frame)) = socket.next().await {
            if let Message::Text(text) = frame {
                texts.push(text.to_string());
                if texts.iter().any(|t| t.contains("stage_layout")) {
                    break;
                }
                if texts.iter().any(|t| t == "echo: hello") {
                    fake.switch("timer");
                }
            }
        }
    };
    tokio::time::timeout(WAIT, read)
        .await
        .expect("both ways within 10 s");
    assert!(texts.contains(&"echo: hello".to_string()), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("\"code\":\"timer\"")),
        "{texts:?}"
    );
    let status = hub.status().await.presenter.unwrap();
    assert_eq!(status.sockets, 1);
    // An upgrade Presenter answers without switching goes back as it came,
    // its body too, and opens no tunnel.
    let refused = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                   Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
    let (code, _, body) = raw(hub.addr, &get(hub.addr, "/live/ws?refuse=1", refused)).await;
    assert_eq!((code, body.as_str()), (403, "refused by the fake"));
    socket.close(None).await.unwrap();
    let records = hub
        .events_until(WAIT, |r| {
            r.iter()
                .any(|r| r["ev"] == "presenter_ws" && r["what"] == "close")
        })
        .await;
    let open = records
        .iter()
        .find(|r| r["ev"] == "presenter_ws" && r["what"] == "open")
        .expect("an open record");
    assert_eq!(open["peer"], "127.0.0.1");
    // Tunnels are numbered from 1, each its own; the refused upgrade opened none.
    assert_eq!(open["client"], 1, "{open}");
    let opens = records
        .iter()
        .filter(|r| r["ev"] == "presenter_ws" && r["what"] == "open")
        .count();
    assert_eq!(opens, 1);
    assert!(fake.seen().contains(&"GET /live/ws".to_string()));
    page.close().await;
    hub.stop().await;
    fake.stop().await;
}

#[tokio::test]
async fn the_hubs_stop_ends_an_open_stage_socket_and_records_it() {
    let fake = FakePresenter::start().await;
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), Some(fake.url())).await;
    let url = format!("ws://{}/live/ws?surface=stage&preview=1", hub.addr);
    let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    socket.send(Message::Text("hello".into())).await.unwrap();
    let echoed = tokio::time::timeout(WAIT, socket.next())
        .await
        .expect("the echo");
    assert!(matches!(echoed, Some(Ok(Message::Text(_)))), "{echoed:?}");
    let dir_path = hub.dir.clone();
    let stopped = std::time::Instant::now();
    hub.stop().await;
    // The tunnel is the hub's to end at its stop: the page's socket ends
    // with it, Presenter still up.
    let ended = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await;
    assert!(ended.is_ok(), "the stage socket outlived the hub's stop");
    assert!(stopped.elapsed() < Duration::from_secs(5));
    // The event log's writer thread puts the record on disk.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let close = loop {
        let records = crate::support::events_in(&dir_path);
        let close = records
            .into_iter()
            .find(|r| r["ev"] == "presenter_ws" && r["what"] == "close");
        if let Some(close) = close {
            break close;
        }
        assert!(std::time::Instant::now() < deadline, "no close record");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(close["why"], fohmixer_hub::ableset::STOPPED, "{close}");
    fake.stop().await;
}

#[tokio::test]
async fn a_hub_without_presenter_tells_no_page_and_keeps_those_paths_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), None).await;
    let mut page = hub.client().await;
    let mut told = Vec::new();
    while let Some(msg) = page.next_message(Duration::from_millis(500)).await {
        told.push(msg);
    }
    assert!(
        !told
            .iter()
            .any(|m| matches!(m, ServerMsg::Presenter { .. })),
        "{told:?}"
    );
    page.send(&ClientMsg::PresenterSwitch {
        code: "timer".into(),
        seq: 1,
    })
    .await;
    assert!(
        page.next_message(Duration::from_millis(500))
            .await
            .is_none()
    );
    let (code, head, _) = raw(hub.addr, &get(hub.addr, "/stage", "")).await;
    assert_eq!(code, 200, "fohmixer's own page");
    assert!(head.contains("x-frame-options: deny"), "{head}");
    assert!(hub.status().await.presenter.is_none());
    page.close().await;
    hub.stop().await;
}
