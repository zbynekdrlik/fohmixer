//! The MBC stream proof end to end (#267): `GET /api/mbc/stream-proof` with
//! the automation token walks the `mbc` SimLive set (`sim/fixtures/
//! mbc-stream-site.json`: the bus with HAND 1 sync, a muted group, a muted
//! group holding a nested one, a Sends Only track, the stream track on the
//! external output taking `No Input`), answers the proof, serves it again
//! inside its window; a stream track with an input set is `ok: false`; and
//! the refusals: a wrong or missing token 401, an internet request 403,
//! a hub without a token 503; no `mbc`, or `mbc` offline, is `ok: false`.
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use crate::support::{Host, TestHub, repo, runtime, serial};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SECS_5: Duration = Duration::from_secs(5);
/// An invented automation token (the CLI's shape: 32+ visible characters).
const TOKEN: &str = "test-automation-token-0123456789abcdef";
const PATH: &str = "/api/mbc/stream-proof";

/// The automation token stored as the CLI stores it.
fn with_token(dir: &Path) {
    let token = fohmixer_hub::automation::AutomationToken::of(TOKEN).unwrap();
    fohmixer_hub::automation::store(fohmixer_hub::automation::AUTOMATION, dir, &token).unwrap();
}

fn site() -> std::path::PathBuf {
    repo()
        .join("sim")
        .join("fixtures")
        .join("mbc-stream-site.json")
}

/// `GET /api/mbc/stream-proof` with `bearer` and the `extra` headers: the
/// status code and the JSON body.
async fn proof(addr: SocketAddr, bearer: Option<&str>, extra: &[(&str, &str)]) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!("GET {PATH} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(token) = bearer {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut bytes))
        .await
        .expect("an answer within 10 s")
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let code: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("not an HTTP answer: {text}"));
    let json = text
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    (code, json)
}

/// The fixture's outputs.
fn outputs() -> Value {
    json!({"bus": "ToStream 1s sync #", "bus_to": "OM STREAM", "stream": "OM STREAM",
           "stream_to": {"type": "Ext. Out", "channel": "3/4 Final LR"},
           "stream_input": "No Input", "others": []})
}

/// The fixture with the stream track's input set to `input`, written into
/// `dir`.
fn site_with_input(dir: &Path, input: &str) -> std::path::PathBuf {
    let mut site: Value = serde_json::from_slice(&std::fs::read(site()).unwrap()).unwrap();
    let stream = site["tracks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["name"] == "OM STREAM")
        .expect("the fixture's stream track");
    stream["input"] = json!({"type": input});
    let path = dir.join("mbc-stream-input.json");
    std::fs::write(&path, serde_json::to_vec(&site).unwrap()).unwrap();
    path
}

#[test]
fn a_stream_track_taking_an_input_is_no_proof() {
    let _serial = serial();
    runtime().block_on(async {
        let sites = tempfile::tempdir().unwrap();
        let host = Host::start_site("mbc", &site_with_input(sites.path(), "Ext. In"), 0, 0.0);
        let dir = tempfile::tempdir().unwrap();
        with_token(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        let (code, answer) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(code, 200, "{answer}");
        assert_eq!(
            answer,
            json!({"ok": false, "ts": answer["ts"], "age_ms": 0, "hand1_sync_only": false,
                   "reason": "OM STREAM takes its input from Ext. In, not No Input"})
        );
        let records = hub
            .events_until(SECS_5, |r| r.iter().any(|r| r["ev"] == "stream_proof"))
            .await;
        let walk = records.iter().find(|r| r["ev"] == "stream_proof").unwrap();
        assert_eq!(walk["ok"], json!(false));
        assert_eq!(
            walk["error"],
            json!("OM STREAM takes its input from Ext. In, not No Input")
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_proof_walks_mbc_and_is_served_again_inside_its_window() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start_site("mbc", &site(), 0, 0.0);
        let dir = tempfile::tempdir().unwrap();
        with_token(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        let (code, first) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(code, 200, "{first}");
        assert_eq!(first["ok"], json!(true), "{first}");
        assert_eq!(first["open"], json!(["Hand1sync #"]));
        assert_eq!(first["hand1_sync_only"], json!(true));
        assert_eq!(first["outputs"], outputs());
        assert_eq!(first["age_ms"], json!(0));
        assert!(first["ts"].as_u64().unwrap() > 0);
        // The muted group opened in Live: inside the window the answer is
        // the walk's, older.
        let mut page = hub.client().await;
        page.set(
            "mbc",
            "live_set tracks[name=Talk grp]",
            "mute",
            json!(false),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (code, again) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(code, 200);
        assert_eq!(again["ts"], first["ts"]);
        assert_eq!(again["open"], json!(["Hand1sync #"]));
        assert!(again["age_ms"].as_u64().unwrap() >= 50, "{again}");
        let records = hub
            .events_until(SECS_5, |r| r.iter().any(|r| r["ev"] == "stream_proof"))
            .await;
        let walks: Vec<&Value> = records
            .iter()
            .filter(|r| r["ev"] == "stream_proof")
            .collect();
        assert_eq!(walks.len(), 1, "{walks:?}");
        assert_eq!(walks[0]["ok"], json!(true));
        assert_eq!(walks[0]["hand1_sync_only"], json!(true));
        page.close().await;
        hub.stop().await;
        // A hub of its own walks afresh: the opened group's tracks are open.
        let dir = tempfile::tempdir().unwrap();
        with_token(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        let (_, fresh) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(fresh["ok"], json!(true), "{fresh}");
        assert_eq!(fresh["open"], json!(["Hand1sync #", "Talk A", "Talk B"]));
        assert_eq!(fresh["hand1_sync_only"], json!(false));
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_proof_request_is_refused_without_the_token_or_from_the_internet() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        with_token(dir.path());
        let hub = TestHub::start(Vec::new(), dir.path()).await;
        let (code, answer) = proof(hub.addr, None, &[]).await;
        assert_eq!(
            (code, &answer["code"]),
            (401, &json!("UNAUTHORIZED")),
            "{answer}"
        );
        let (code, _) = proof(
            hub.addr,
            Some("wrong-token-0123456789abcdef0123456789"),
            &[],
        )
        .await;
        assert_eq!(code, 401);
        // The PIN login's token is no automation token.
        let (code, _) = proof(hub.addr, Some(&hub.token), &[]).await;
        assert_eq!(code, 401);
        // Through a proxy (the internet path): refused before the token.
        let (code, _) = proof(
            hub.addr,
            Some(TOKEN),
            &[("cf-connecting-ip", "203.0.113.9")],
        )
        .await;
        assert_eq!(code, 403);
        // The token on a hub with no mbc: an answer, ok false.
        let (code, answer) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(code, 200);
        assert_eq!(
            answer,
            json!({"ok": false, "ts": answer["ts"], "age_ms": 0, "hand1_sync_only": false,
                   "reason": "this hub has no mbc Live set"})
        );
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter().filter(|r| r["ev"] == "stream_proof").count() >= 3
            })
            .await;
        let refused: Vec<&Value> = records
            .iter()
            .filter(|r| r["ev"] == "stream_proof")
            .map(|r| &r["refused"])
            .collect();
        assert_eq!(refused, vec!["token", "token", "token"]);
        hub.stop().await;
    });
}

#[test]
fn a_hub_without_a_token_serves_no_proof_and_an_offline_mbc_is_no_proof() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start_site("mbc", &site(), 0, 0.0);
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let (code, answer) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(
            (code, &answer["code"]),
            (503, &json!("NO_AUTOMATION")),
            "{answer}"
        );
        hub.stop().await;
        let dir = tempfile::tempdir().unwrap();
        with_token(dir.path());
        let cfg = host.cfg();
        host.stop();
        let hub = TestHub::start(vec![cfg], dir.path()).await;
        let (code, answer) = proof(hub.addr, Some(TOKEN), &[]).await;
        assert_eq!(code, 200);
        assert_eq!(answer["ok"], json!(false));
        assert_eq!(answer["hand1_sync_only"], json!(false));
        assert_eq!(answer["reason"], json!("the mbc Live set is offline"));
        hub.stop().await;
    });
}
