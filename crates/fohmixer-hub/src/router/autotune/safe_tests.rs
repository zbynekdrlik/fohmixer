use std::collections::BTreeMap;

use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::super::tests::{next_of, records};
use super::super::super::{Router, RouterIo, RouterMsg};
use super::{SAFE_WRITES, Safe, safe_commands, safe_news, safe_problem};
use crate::live::client::{Asked, LiveEvent, Script, scripted};
use crate::rules::HubState;
use crate::rules::autotune::find::Found;
use crate::rules::autotune::keys::KeyMissing;

/// The repro target (on 0, off 127) and the stream target (on 127, off 0)
/// of the band.
const REPRO: &str = "band|live_set tracks 6";
const STREAM: &str = "band|live_set tracks 7";

fn target(index: u32, label: &str, on: u8, off: u8) -> AutotuneTarget {
    AutotuneTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: None,
        },
        label: label.into(),
        on,
        off,
    }
}

fn repro() -> AutotuneTarget {
    target(6, "Vox 2", 0, 127)
}

fn stream() -> AutotuneTarget {
    target(7, "#8", 127, 0)
}

fn layout(rev: u64, autotune: Vec<AutotuneTarget>) -> RouterMsg {
    RouterMsg::Layout {
        rev,
        stage: None,
        targets: BTreeMap::new(),
        strips: Vec::new(),
        reverb: Vec::new(),
        fader_leaders: Vec::new(),
        autotune,
    }
}

/// A target's newest walk answers with the macro on band track `index`,
/// `$ref` `id`.
fn find(router: &mut Router, key: &str, index: u32, id: &str) {
    let walk = router.autotune.targets[key].walk;
    router.autotune_found(
        key,
        walk,
        Ok(Found {
            path: format!("live_set tracks {index} devices 0 parameters 1"),
            id: id.into(),
            key: Err(KeyMissing::NoChain),
        }),
    );
}

/// A router whose band is a scripted Live: every call comes to the test.
fn scripted_router(
    dir: &std::path::Path,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
    Script,
) {
    let (live, script) = scripted("band");
    let (tx, own) = tokio::sync::mpsc::unbounded_channel();
    let (events, records) = crate::events::EventLog::channel(1024);
    let router = Router::new(
        BTreeMap::from([("band".to_string(), live)]),
        HubState::default(),
        dir.to_path_buf(),
        RouterIo { tx, events },
    );
    (router, own, records, script)
}

/// A registration's writes as the script gets them: each `$ref` and value.
type Writes = Vec<(String, f64)>;

/// The next registration the router asked the scripted Live for (any other
/// call, a walk, is passed over, unanswered): its writes and the call.
fn next_safe(script: &mut Script) -> Option<(Writes, Asked)> {
    while let Some(asked) = script.try_call() {
        if asked.commands.len() == 1 && asked.commands[0]["name"] == SAFE_WRITES {
            let writes = asked.commands[0]["args"]["writes"]
                .as_array()
                .expect("a list of writes")
                .iter()
                .map(|w| {
                    (
                        w["target"]["$ref"].as_str().unwrap().to_string(),
                        w["value"].as_f64().unwrap(),
                    )
                })
                .collect();
            return Some((writes, asked));
        }
    }
    None
}

fn registration(script: &mut Script) -> (Writes, Asked) {
    next_safe(script).expect("a registration of the DRY writes")
}

fn writes(pairs: &[(&str, f64)]) -> Writes {
    pairs.iter().map(|(id, v)| (id.to_string(), *v)).collect()
}

/// The script's answer to a registration (`slot`; none: the call dropped,
/// as when Live is offline), and its message handled.
async fn answered(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    asked: Asked,
    slot: Option<Value>,
) {
    match slot {
        Some(slot) => asked.answer(vec![slot]),
        None => drop(asked),
    }
    let msg = next_of(own, "the registration's answer", |m| {
        matches!(m, RouterMsg::AutotuneSafe { .. })
    })
    .await;
    router.handle(msg);
}

fn kept(n: usize) -> Option<Value> {
    Some(json!({"ok": true, "data": {"armed": n}}))
}

/// The `safe` records waiting, without `ev` and `ts`.
fn safe_steps(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == "safe")
        .map(|mut r| {
            let fields = r.as_object_mut().unwrap();
            fields.remove("ev");
            fields.remove("ts");
            r
        })
        .collect()
}

fn safe_step(targets: usize, problem: Option<&str>) -> Value {
    json!({"step": "safe", "instance": "band", "targets": targets, "problem": problem})
}

/// The band's layout with both targets, both racks found (`p6`, `p7`): the
/// call of their registration, unanswered (the first one's, of the repro
/// rack alone, answered: void).
async fn both_found(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    script: &mut Script,
) -> Asked {
    router.handle(layout(1, vec![repro(), stream()]));
    find(router, REPRO, 6, "p6");
    find(router, STREAM, 7, "p7");
    let (one, first) = registration(script);
    assert_eq!(one, writes(&[("p6", 127.0)]));
    let (both, asked) = registration(script);
    assert_eq!(both, writes(&[("p6", 127.0), ("p7", 0.0)]));
    answered(router, own, first, kept(1)).await;
    asked
}

/// The band connected (a set loaded, a Live started).
fn connect(router: &mut Router) {
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "1.13.0".into(),
            live_version: "12".into(),
        }),
    });
}

#[test]
fn safe_commands_register_each_ref_with_its_off_value_in_one_command() {
    assert_eq!(SAFE_WRITES, "safe_writes");
    assert_eq!(
        safe_commands(&[("p6", 127), ("p7", 0)]),
        vec![
            json!({"target": "live_set", "name": "safe_writes", "args": {"writes": [
                {"target": {"$ref": "p6", "class": "DeviceParameter"}, "value": 127.0},
                {"target": {"$ref": "p7", "class": "DeviceParameter"}, "value": 0.0},
            ]}})
        ]
    );
    assert_eq!(
        safe_commands(&[]),
        vec![json!({"target": "live_set", "name": "safe_writes", "args": {"writes": []}})]
    );
}

#[test]
fn a_list_is_news_unless_the_script_holds_it() {
    let list = |pairs: &[(&str, u8)]| -> Vec<(String, u8)> {
        pairs.iter().map(|(id, v)| (id.to_string(), *v)).collect()
    };
    let held = |list: Option<Vec<(String, u8)>>| Safe { list, seq: 1 };
    let both = list(&[("p6", 127), ("p7", 0)]);
    // Since its connect the script holds nothing: only a list with writes
    // is news.
    assert!(!safe_news(None, &[]));
    assert!(safe_news(None, &both));
    // The list sent last is no news; another one is, a disarm too.
    assert!(!safe_news(Some(&held(Some(both.clone()))), &both));
    assert!(safe_news(
        Some(&held(Some(both.clone()))),
        &list(&[("p6", 127)])
    ));
    assert!(safe_news(
        Some(&held(Some(both.clone()))),
        &list(&[("p6", 127), ("p7", 127)])
    ));
    assert!(safe_news(Some(&held(Some(both.clone()))), &[]));
    assert!(!safe_news(Some(&held(Some(Vec::new()))), &[]));
    // After a failure what the script holds is not known: any list is
    // news, an empty one too.
    assert!(safe_news(Some(&held(None)), &both));
    assert!(safe_news(Some(&held(None)), &[]));
}

#[test]
fn a_registration_fails_on_a_call_error_or_a_refused_slot() {
    assert_eq!(safe_problem(&Ok(vec![kept(2).unwrap()])), None);
    assert_eq!(
        safe_problem(&Err("instance offline".into())),
        Some("instance offline".to_string())
    );
    assert_eq!(
        safe_problem(&Ok(vec![json!({"ok": false,
            "error": "no such function: safe_writes", "errorType": "OpError"})])),
        Some("no such function: safe_writes".to_string())
    );
    assert_eq!(safe_problem(&Ok(vec![])), Some("no result".to_string()));
}

#[tokio::test]
async fn every_found_selector_is_registered_with_its_off_value() {
    // Whatever AUT is (off here): each found target's macro with its off
    // value, in the layout's order.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![repro(), stream()]));
    assert!(next_safe(&mut script).is_none(), "nothing found yet");
    assert!(!router.autotune.state().aut);
    find(&mut router, STREAM, 7, "p7");
    let (one, first) = registration(&mut script);
    assert_eq!(one, writes(&[("p7", 0.0)]));
    find(&mut router, REPRO, 6, "p6");
    let (both, second) = registration(&mut script);
    assert_eq!(both, writes(&[("p6", 127.0), ("p7", 0.0)]), "Live's order");
    assert_eq!(
        router.autotune.safe["band"].list,
        Some(vec![("p6".to_string(), 127), ("p7".to_string(), 0)])
    );
    // The first one's answer is void: the second one replaced it.
    answered(&mut router, &mut own, first, kept(1)).await;
    assert_eq!(safe_steps(&rx), Vec::<Value>::new());
    answered(&mut router, &mut own, second, kept(2)).await;
    assert_eq!(safe_steps(&rx), vec![safe_step(2, None)]);
    assert!(next_safe(&mut script).is_none());
}

#[tokio::test]
async fn an_unchanged_list_is_not_sent_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    answered(&mut router, &mut own, asked, kept(2)).await;
    assert_eq!(safe_steps(&rx), vec![safe_step(2, None)]);
    // The same macros found again, the same layout served again: nothing.
    find(&mut router, STREAM, 7, "p7");
    find(&mut router, REPRO, 6, "p6");
    router.handle(layout(2, vec![repro(), stream()]));
    router.autotune_release("band", "settled");
    assert!(next_safe(&mut script).is_none());
    // A rack replaced in place: a new `$ref` at the same path is news.
    find(&mut router, STREAM, 7, "p7b");
    let (again, _) = registration(&mut script);
    assert_eq!(again, writes(&[("p6", 127.0), ("p7b", 0.0)]));
}

#[tokio::test]
async fn a_lost_rack_shrinks_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let _asked = both_found(&mut router, &mut own, &mut script).await;
    let walk = router.autotune.targets[REPRO].walk;
    router.autotune_found(REPRO, walk, Err("no rack named Latencies".into()));
    let (shrunk, _) = registration(&mut script);
    assert_eq!(shrunk, writes(&[("p7", 0.0)]));
    // A target the layout drops leaves the list too; with none left an
    // empty list disarms the script.
    router.handle(layout(2, vec![repro()]));
    let (none, _) = registration(&mut script);
    assert_eq!(none, Writes::new());
    assert_eq!(router.autotune.safe["band"].list, Some(Vec::new()));
}

#[tokio::test]
async fn a_disconnect_forgets_the_registration() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    let old = both_found(&mut router, &mut own, &mut script).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert!(!router.autotune.safe.contains_key("band"));
    // An answer from the old connection is void.
    answered(&mut router, &mut own, old, kept(2)).await;
    assert_eq!(safe_steps(&rx), Vec::<Value>::new());
    connect(&mut router);
    // The connect's walks answer while the band is held (`offline`):
    // nothing is registered.
    find(&mut router, STREAM, 7, "q7");
    find(&mut router, REPRO, 6, "q6");
    assert!(next_safe(&mut script).is_none(), "held");
    // The hold ends (here with the next layout): the full list again, by
    // the new connection's refs.
    router.handle(layout(2, vec![repro(), stream()]));
    let (again, asked) = registration(&mut script);
    assert_eq!(again, writes(&[("q6", 127.0), ("q7", 0.0)]));
    answered(&mut router, &mut own, asked, kept(2)).await;
    assert_eq!(safe_steps(&rx), vec![safe_step(2, None)]);
}

#[tokio::test]
async fn a_failed_registration_is_sent_again_at_the_next_change() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    // A script older than 1.13.0 has no such command.
    let refused = json!({"ok": false, "error": "no such function: safe_writes",
                         "errorType": "OpError"});
    answered(&mut router, &mut own, asked, Some(refused)).await;
    assert_eq!(
        safe_steps(&rx),
        vec![safe_step(2, Some("no such function: safe_writes"))]
    );
    assert_eq!(router.autotune.safe["band"].list, None);
    // What the script holds is not known: the same list goes again where
    // the next registration would (the markers settled, no hold).
    router.autotune_release("band", "settled");
    let (again, asked) = registration(&mut script);
    assert_eq!(again, writes(&[("p6", 127.0), ("p7", 0.0)]));
    // Live did not answer it: again.
    answered(&mut router, &mut own, asked, None).await;
    assert_eq!(
        safe_steps(&rx),
        vec![safe_step(2, Some("instance offline"))]
    );
    router.autotune_release("band", "settled");
    let (_, asked) = registration(&mut script);
    answered(&mut router, &mut own, asked, kept(2)).await;
    assert_eq!(safe_steps(&rx), vec![safe_step(2, None)]);
    // Kept: no more.
    router.autotune_release("band", "settled");
    assert!(next_safe(&mut script).is_none());
}
