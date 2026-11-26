//! The scene automation end to end (#207): `POST /api/scene` with the
//! automation token learns the scene and moves the faders with a rule for
//! it on SimLive (a volume glides to its dB in about its time), a page's
//! write of a fader under a glide stops it (the hand wins), and the
//! refusals: a wrong or missing token 401, an internet request 403, a bad
//! body 400, a hub without a token 503.
#![cfg(unix)]

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::support::{Client, Host, TestHub, runtime, serial};
use fohmixer_proto::client::{ClientMsg, ServerMsg, hub_key};
use fohmixer_proto::scenes::{SceneMute, SceneRule};
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::volume::{value2db, volume_of};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SECS_5: Duration = Duration::from_secs(5);
/// An invented automation token (the CLI's shape: 32+ visible characters).
const TOKEN: &str = "test-automation-token-0123456789abcdef";
const HAND1: &str = "live_set tracks[name=Hand1 #] mixer_device volume";

/// One page with Hand1 # and Hand2 # by name.
fn layout(dir: &Path) {
    let layout = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [{"kind": "group", "id": "g", "title": "G", "controls": [
            {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Hand1 #"}},
             "strip_kind": "standard"},
            {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Hand2 #"}},
             "strip_kind": "standard"}]}]}]}]});
    std::fs::write(dir.join("layout.json"), layout.to_string()).unwrap();
}

/// The automation token stored as the CLI stores it.
fn with_token(dir: &Path) {
    let token = fohmixer_hub::automation::AutomationToken::of(TOKEN).unwrap();
    fohmixer_hub::automation::store(fohmixer_hub::automation::AUTOMATION, dir, &token).unwrap();
}

/// `POST /api/scene` with `body`, `bearer` and the `extra` headers: the
/// status code and the JSON body.
async fn post_scene(
    addr: SocketAddr,
    bearer: Option<&str>,
    body: &str,
    extra: &[(&str, &str)],
) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "POST /api/scene HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(token) = bearer {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
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

async fn scene(addr: SocketAddr, name: &str) -> (u16, Value) {
    post_scene(addr, Some(TOKEN), &json!({"scene": name}).to_string(), &[]).await
}

fn hand1() -> StripRef {
    StripRef {
        instance: "band".into(),
        label: "Hand1 #".into(),
    }
}

/// A page's rule for Hand1 # and `scene`, answered `result`.
async fn set_rule(c: &mut Client, id: &str, scene: &str, rule: Option<SceneRule>) {
    c.send(&ClientMsg::SetSceneRule {
        id: id.into(),
        strip: hand1(),
        scene: scene.into(),
        rule,
    })
    .await;
    let id = id.to_string();
    c.wait(SECS_5, |m| match m {
        ServerMsg::Result { id: got, .. } if *got == id => Some(()),
        ServerMsg::Error {
            id: Some(got),
            message,
        } if *got == id => {
            panic!("the rule was refused: {message}")
        }
        _ => None,
    })
    .await;
}

/// Hand1 #'s volume, read with a command.
async fn volume(c: &mut Client) -> f64 {
    c.get("band", HAND1, "value").await.as_f64().unwrap()
}

/// Waits until the hub holds Hand1 #'s volume (the rule follows the same
/// entry).
async fn heard(c: &mut Client) {
    if let ServerMsg::Subbed { value: Some(_), .. } = c.sub("band", HAND1, "value", false).await {
        return;
    }
    let sub = hub_key("band", HAND1, "value", false);
    c.value_until(&sub, SECS_5, |i| i.value.is_some()).await;
}

/// The `scene_glide` records of Hand1 #, in order.
fn glides(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "scene_glide")
        .cloned()
        .collect()
}

#[test]
fn a_scene_glides_its_fader_in_about_its_time_and_the_hand_wins() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        with_token(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online && s.scenes.on)
            .await;
        let mut page = hub.client().await;
        // A page attached gets the document: no scene heard yet.
        page.wait(SECS_5, |m| match m {
            ServerMsg::Scenes { doc } if doc.known.is_empty() => Some(()),
            _ => None,
        })
        .await;
        // A scene no rule names: learned, nothing moves.
        assert_eq!(
            scene(hub.addr, "intro").await,
            (200, json!({"scene": "intro", "moved": 0}))
        );
        page.wait(SECS_5, |m| match m {
            ServerMsg::Scene { name, moved: 0, .. } if name == "intro" => Some(()),
            _ => None,
        })
        .await;
        // −10 dB over 1 s from Live's 0 dB.
        set_rule(
            &mut page,
            "r1",
            "intro",
            Some(SceneRule {
                db: Some(-10.0),
                mute: None,
                time_ms: 1000,
            }),
        )
        .await;
        heard(&mut page).await;
        assert!(
            (value2db(volume(&mut page).await)).abs() < 0.05,
            "starts at 0 dB"
        );
        let sent = Instant::now();
        assert_eq!(
            scene(hub.addr, "intro").await,
            (200, json!({"scene": "intro", "moved": 1}))
        );
        let records = hub
            .events_until(SECS_5, |r| {
                glides(r)
                    .iter()
                    .any(|g| g["what"] == "stop" && g["why"] == "done")
            })
            .await;
        let g = glides(&records);
        assert_eq!(g[0]["what"], "start");
        assert_eq!(g[0]["to_db"], json!(-10.0));
        assert_eq!(g[0]["ms"], json!(1000));
        // The hub's own timing, on its records (#186): about its time.
        let took = g[1]["ts"].as_i64().unwrap() - g[0]["ts"].as_i64().unwrap();
        assert!((950..=1600).contains(&took), "the glide took {took} ms");
        assert!(sent.elapsed() < Duration::from_secs(4));
        let v = volume(&mut page).await;
        assert!(
            (value2db(v) + 10.0).abs() < 0.01,
            "lands at −10 dB: {}",
            value2db(v)
        );
        assert_eq!(v, volume_of(-10.0), "the last step is exact");
        // The scene record.
        assert!(records.iter().any(|r| r["ev"] == "scene"
            && r["scene"] == "intro"
            && r["moved"] == 1
            && r["source"] == "lan"));

        // A long glide back up; the page's write stops it at once.
        assert_eq!(scene(hub.addr, "talk").await.0, 200);
        set_rule(
            &mut page,
            "r2",
            "talk",
            Some(SceneRule {
                db: Some(0.0),
                mute: Some(SceneMute::Mute),
                time_ms: 8000,
            }),
        )
        .await;
        assert_eq!(
            scene(hub.addr, "talk").await,
            (200, json!({"scene": "talk", "moved": 1}))
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        page.write("band", HAND1, "value", json!(0.3), 1, true)
            .await;
        let records = hub
            .events_until(SECS_5, |r| glides(r).iter().any(|g| g["why"] == "hand"))
            .await;
        let stop = glides(&records)
            .into_iter()
            .find(|g| g["why"] == "hand")
            .unwrap();
        assert_eq!(stop["label"], "Hand1 #");
        assert_eq!(stop["scene"], "talk");
        // The mute went once at the start.
        let muted = page
            .get("band", "live_set tracks[name=Hand1 #]", "mute")
            .await;
        assert!(muted == json!(true) || muted == json!(1), "{muted}");
        // The hand's value stays: nothing more is written by the rule.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(volume(&mut page).await, 0.3);
        let status = hub.status().await.scenes;
        assert_eq!(status.glides, 0);
        assert_eq!(status.last.as_deref(), Some("talk"));
        assert_eq!(status.known, 2);
        hub.stop().await;
        drop(host);
    });
}

#[test]
fn a_scene_request_is_refused_without_the_token_from_the_internet_or_with_a_bad_body() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        with_token(dir.path());
        let hub = TestHub::start(Vec::new(), dir.path()).await;
        let body = json!({"scene": "intro"}).to_string();
        let (code, answer) = post_scene(hub.addr, None, &body, &[]).await;
        assert_eq!(
            (code, &answer["code"]),
            (401, &json!("UNAUTHORIZED")),
            "{answer}"
        );
        let (code, _) = post_scene(
            hub.addr,
            Some("wrong-token-0123456789abcdef0123456789"),
            &body,
            &[],
        )
        .await;
        assert_eq!(code, 401);
        // The PIN login's token is no automation token.
        let (code, _) = post_scene(hub.addr, Some(&hub.token), &body, &[]).await;
        assert_eq!(code, 401);
        // Through a proxy (the internet path): refused before the token (on
        // a hub without `[access]` by the Access check itself, before the
        // handler; with one, an Access login still meets the handler's
        // `LAN_ONLY`, `routes::scene` unit tests).
        let (code, _) = post_scene(
            hub.addr,
            Some(TOKEN),
            &body,
            &[("cf-connecting-ip", "203.0.113.9")],
        )
        .await;
        assert_eq!(code, 403);
        for bad in [
            r#"{"scene": ""}"#,
            r#"{"name": "intro"}"#,
            "intro",
            r#"{"scene": "a\u0001b"}"#,
        ] {
            let (code, answer) = post_scene(hub.addr, Some(TOKEN), bad, &[]).await;
            assert_eq!(
                (code, &answer["code"]),
                (400, &json!("BAD_SCENE")),
                "{bad}: {answer}"
            );
        }
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter().filter(|r| r["ev"] == "scene").count() >= 7
            })
            .await;
        let refused: Vec<&Value> = records
            .iter()
            .filter(|r| r["ev"] == "scene")
            .map(|r| &r["refused"])
            .collect();
        assert_eq!(
            refused,
            vec!["token", "token", "token", "body", "body", "body", "body"]
        );
        // Nothing was learned.
        assert_eq!(hub.status().await.scenes.known, 0);
        hub.stop().await;
    });
}

#[test]
fn a_hub_without_a_token_takes_no_scene_and_tells_no_page() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let hub = TestHub::start(Vec::new(), dir.path()).await;
        let (code, answer) = scene(hub.addr, "intro").await;
        assert_eq!(
            (code, &answer["code"]),
            (503, &json!("NO_AUTOMATION")),
            "{answer}"
        );
        assert!(
            answer["message"]
                .as_str()
                .unwrap()
                .contains("fohmixer-hub automation set-token")
        );
        assert!(!hub.status().await.scenes.on);
        let mut page = hub.client().await;
        // A ping's pong comes after everything the attach sent.
        page.send(&ClientMsg::Ping {
            n: 1,
            t: 1.0,
            rtt: None,
            rtt_n: None,
        })
        .await;
        let mut seen = Vec::new();
        while let Some(m) = page.next_message(SECS_5).await {
            if matches!(m, ServerMsg::Pong { .. }) {
                break;
            }
            seen.push(m);
        }
        assert!(
            !seen
                .iter()
                .any(|m| matches!(m, ServerMsg::Scenes { .. } | ServerMsg::Scene { .. })),
            "{seen:?}"
        );
        hub.stop().await;
    });
}
