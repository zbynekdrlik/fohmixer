use std::collections::BTreeMap;

use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use fohmixer_proto::client::{SafeState, SafeStatus};

use super::super::super::retries::{CONNECT_RETRIES, RETRY_MS};
use super::super::super::tests::{next_of, records, status};
use super::super::super::{Router, RouterIo, RouterMsg};
use super::super::AfterWalk;
use super::{
    REF_GONE, SAFE_WRITES, Safe, does_not_resolve, failed_problem, gone_refs, kept_after,
    probe_commands, safe_commands, safe_news, safe_problem,
};
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
        singer: None,
        two_singers: false,
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
        lufs: Vec::new(),
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
    let held = |list: Vec<(String, u8)>, state: SafeState| Safe {
        list,
        seq: 1,
        state,
        problem: None,
        failures: 0,
        kept: None,
        probes: 0,
        lost: false,
    };
    let both = list(&[("p6", 127), ("p7", 0)]);
    // Since its connect the script holds nothing: only a list with writes
    // is news.
    assert!(!safe_news(None, &[]));
    assert!(safe_news(None, &both));
    // The list sent last (on its way or kept) is no news; another one is,
    // a disarm too.
    for state in [SafeState::Sent, SafeState::Kept] {
        assert!(!safe_news(Some(&held(both.clone(), state)), &both));
        assert!(safe_news(
            Some(&held(both.clone(), state)),
            &list(&[("p6", 127)])
        ));
        assert!(safe_news(
            Some(&held(both.clone(), state)),
            &list(&[("p6", 127), ("p7", 127)])
        ));
        assert!(safe_news(Some(&held(both.clone(), state)), &[]));
        assert!(!safe_news(Some(&held(Vec::new(), state)), &[]));
    }
    // After a failure what the script holds is not known: any list is
    // news, the same one and an empty one too.
    assert!(safe_news(
        Some(&held(both.clone(), SafeState::Failed)),
        &both
    ));
    assert!(safe_news(Some(&held(Vec::new(), SafeState::Failed)), &[]));
}

#[test]
fn a_list_on_its_way_or_unanswered_narrows_the_kept_one() {
    // PR #147 re-review R4.
    let list = |pairs: &[(&str, u8)]| -> Vec<(String, u8)> {
        pairs.iter().map(|(id, v)| (id.to_string(), *v)).collect()
    };
    let safe = |list: Vec<(String, u8)>, state, kept, lost| Safe {
        list,
        seq: 1,
        state,
        problem: None,
        failures: 0,
        kept,
        probes: 0,
        lost,
    };
    let both = list(&[("p6", 127), ("p7", 0)]);
    let one = list(&[("p6", 127)]);
    assert_eq!(kept_after(None), None);
    assert_eq!(
        kept_after(Some(&safe(one.clone(), SafeState::Sent, None, false))),
        None
    );
    // On its way, or Live did not answer it: the shared pairs only.
    assert_eq!(
        kept_after(Some(&safe(
            one.clone(),
            SafeState::Sent,
            Some(both.clone()),
            false
        ))),
        Some(one.clone())
    );
    assert_eq!(
        kept_after(Some(&safe(
            one.clone(),
            SafeState::Failed,
            Some(both.clone()),
            true
        ))),
        Some(one.clone())
    );
    // Refused, or kept: the kept list as it is.
    assert_eq!(
        kept_after(Some(&safe(
            one.clone(),
            SafeState::Failed,
            Some(both.clone()),
            false
        ))),
        Some(both.clone())
    );
    assert_eq!(
        kept_after(Some(&safe(
            both.clone(),
            SafeState::Kept,
            Some(both.clone()),
            false
        ))),
        Some(both.clone())
    );
    // One that adds a pair keeps every kept pair.
    let more = list(&[("p6", 127), ("p7", 0), ("p8", 0)]);
    assert_eq!(
        kept_after(Some(&safe(
            more,
            SafeState::Sent,
            Some(both.clone()),
            false
        ))),
        Some(both)
    );
}

#[test]
fn a_failed_registration_says_when_it_gave_up() {
    assert_eq!(
        failed_problem("instance offline", AfterWalk::Retry, 3),
        "instance offline"
    );
    assert_eq!(
        failed_problem("no such function: safe_writes", AfterWalk::Wait, 1),
        "no such function: safe_writes"
    );
    assert_eq!(
        failed_problem("instance offline", AfterWalk::GiveUp, 25),
        "instance offline (gave up after 25 tries, until the next trigger)"
    );
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
        vec![("p6".to_string(), 127), ("p7".to_string(), 0)]
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
    assert_eq!(router.autotune.safe["band"].list, Vec::new());
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
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
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

#[tokio::test]
async fn a_found_target_whose_walk_live_did_not_answer_keeps_its_ref_and_its_dry_write() {
    // PR #146 review I3: the script holds the object, so a walk Live did not
    // answer (busy, a timeout) leaves the target's ref and its DRY write as
    // they were; a definite refusal or a definite answer drops them.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    answered(&mut router, &mut own, asked, kept(2)).await;
    router.autotune_walk(STREAM);
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_walk_failed(STREAM, walk, "no result within 3 s".into());
    let target = &router.autotune.targets[STREAM];
    assert_eq!(
        target.found.as_ref().map(|f| f.id.as_str()),
        Some("p7"),
        "the hub keeps its ref"
    );
    assert_eq!(target.problem, None);
    assert_eq!(target.walk_failures, 1, "and walks it again");
    assert!(next_safe(&mut script).is_none(), "the registration stays");
    // A walk Live refused is a definite answer: the target leaves the list.
    router.autotune_walk(STREAM);
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_walk_failed(STREAM, walk, "live error: busy".into());
    assert!(router.autotune.targets[STREAM].found.is_none());
    let (shrunk, _) = registration(&mut script);
    assert_eq!(shrunk, writes(&[("p6", 127.0)]));
}

/// Every message the router's own tasks send within `limit_ms` (on tokio's
/// virtual clock), handled.
async fn pump(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>, limit_ms: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(limit_ms);
    while let Ok(Some(msg)) = tokio::time::timeout_at(deadline, own.recv()).await {
        router.handle(msg);
    }
}

/// The band's registration in `/api/status`.
fn safe_status(router: &mut Router) -> Vec<SafeStatus> {
    status(router).autotune.safe
}

fn band(state: SafeState, problem: Option<&str>) -> Vec<SafeStatus> {
    vec![SafeStatus {
        instance: "band".into(),
        state,
        targets: 2,
        problem: problem.map(str::to_string),
    }]
}

#[tokio::test]
async fn a_registration_is_kept_only_on_the_scripts_ok() {
    // PR #146 review I2: the hub tells a registration on its way from one
    // the script kept, so #140 PR 3b can switch on only under a kept one.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    assert_eq!(safe_status(&mut router), Vec::new());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    assert_eq!(safe_status(&mut router), band(SafeState::Sent, None));
    answered(&mut router, &mut own, asked, kept(2)).await;
    assert_eq!(safe_status(&mut router), band(SafeState::Kept, None));
    // A new list is on its way again until its answer.
    router.handle(layout(2, vec![repro()]));
    let (_, asked) = registration(&mut script);
    let only = vec![SafeStatus {
        instance: "band".into(),
        state: SafeState::Sent,
        targets: 1,
        problem: None,
    }];
    assert_eq!(safe_status(&mut router), only);
    answered(&mut router, &mut own, asked, kept(1)).await;
    assert_eq!(safe_status(&mut router)[0].state, SafeState::Kept);
}

#[tokio::test(start_paused = true)]
async fn a_registration_live_did_not_answer_is_retried_every_retry_ms() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    answered(&mut router, &mut own, asked, None).await;
    assert_eq!(
        safe_status(&mut router),
        band(SafeState::Failed, Some("instance offline"))
    );
    assert_eq!(router.autotune.safe["band"].failures, 1);
    assert_eq!(
        safe_steps(&rx),
        vec![safe_step(2, Some("instance offline"))]
    );
    // Not before RETRY_MS, then once.
    pump(&mut router, &mut own, RETRY_MS - 1).await;
    assert!(next_safe(&mut script).is_none(), "not before RETRY_MS");
    pump(&mut router, &mut own, RETRY_MS / 2).await;
    let (again, asked) = registration(&mut script);
    assert_eq!(again, writes(&[("p6", 127.0), ("p7", 0.0)]));
    assert!(next_safe(&mut script).is_none(), "once");
    answered(&mut router, &mut own, asked, kept(2)).await;
    assert_eq!(safe_status(&mut router), band(SafeState::Kept, None));
    assert_eq!(router.autotune.safe["band"].failures, 0);
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert!(next_safe(&mut script).is_none(), "kept: no more");
}

#[tokio::test(start_paused = true)]
async fn a_refused_registration_is_shown_and_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    let refused = json!({"ok": false,
        "error": "bad args: 1 safe write target(s) do not resolve", "errorType": "OpError"});
    answered(&mut router, &mut own, asked, Some(refused)).await;
    assert_eq!(
        safe_status(&mut router),
        band(
            SafeState::Failed,
            Some("bad args: 1 safe write target(s) do not resolve")
        )
    );
    assert_eq!(router.autotune.safe["band"].failures, 0, "no retry counted");
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert!(next_safe(&mut script).is_none(), "never retried");
}

#[tokio::test(start_paused = true)]
async fn a_registration_live_never_answers_gives_up_after_the_connect_retries() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let mut asked = both_found(&mut router, &mut own, &mut script).await;
    for n in 1..=CONNECT_RETRIES {
        answered(&mut router, &mut own, asked, None).await;
        pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
        asked = next_safe(&mut script)
            .unwrap_or_else(|| panic!("retry {n}"))
            .1;
    }
    answered(&mut router, &mut own, asked, None).await;
    assert_eq!(
        safe_status(&mut router),
        band(
            SafeState::Failed,
            Some("instance offline (gave up after 25 tries, until the next trigger)")
        )
    );
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert!(next_safe(&mut script).is_none(), "no retry after the last");
    // The next change point sends it again, its count over.
    router.autotune_release("band", "settled");
    let (_, asked) = registration(&mut script);
    assert_eq!(router.autotune.safe["band"].failures, 0);
    answered(&mut router, &mut own, asked, None).await;
    pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
    assert!(next_safe(&mut script).is_some(), "retried again");
}

#[tokio::test(start_paused = true)]
async fn a_retry_is_void_after_a_newer_registration_or_a_disconnect_and_waits_while_busy() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    answered(&mut router, &mut own, asked, None).await;
    // A change point sends the list again before the retry is due: the
    // retry is void.
    router.autotune_release("band", "settled");
    let (_, newer) = registration(&mut script);
    // (Under the call's 3 s answer timeout: `newer` waits unanswered.)
    pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 4).await;
    assert!(next_safe(&mut script).is_none(), "the old retry is void");
    // While Live is busy a due retry waits, uncounted.
    answered(&mut router, &mut own, newer, None).await;
    router.states.get_mut("band").unwrap().busy = true;
    pump(&mut router, &mut own, 3 * RETRY_MS + RETRY_MS / 2).await;
    assert!(next_safe(&mut script).is_none(), "no retry while busy");
    assert_eq!(router.autotune.safe["band"].failures, 1);
    router.states.get_mut("band").unwrap().busy = false;
    pump(&mut router, &mut own, RETRY_MS).await;
    let (_, asked) = registration(&mut script);
    // A disconnect voids a waiting retry.
    answered(&mut router, &mut own, asked, None).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert!(
        next_safe(&mut script).is_none(),
        "void after the disconnect"
    );
    assert_eq!(safe_status(&mut router), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn an_old_retry_is_void_when_the_newer_registration_failed_too() {
    // The gate's mutant of the retry's check (`&&` -> `||`): a retry of
    // an older registration must not send a newer failed one early, beside
    // that one's own retry.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    answered(&mut router, &mut own, asked, None).await;
    // Half a retry later a change point sends the list again, and Live
    // does not answer that one either: its retry is due RETRY_MS later.
    pump(&mut router, &mut own, RETRY_MS / 2).await;
    assert!(next_safe(&mut script).is_none(), "not yet");
    router.autotune_release("band", "settled");
    let (_, newer) = registration(&mut script);
    answered(&mut router, &mut own, newer, None).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
    // The first one's retry falls due first: void.
    pump(&mut router, &mut own, RETRY_MS * 3 / 4).await;
    assert!(next_safe(&mut script).is_none(), "the old retry is void");
    // The newer one's own retry sends it, once.
    pump(&mut router, &mut own, RETRY_MS).await;
    let (again, _asked) = registration(&mut script);
    assert_eq!(again, writes(&[("p6", 127.0), ("p7", 0.0)]));
    assert!(next_safe(&mut script).is_none(), "once");
    assert_eq!(router.autotune.safe["band"].failures, 1);
}

#[test]
fn the_probe_asks_each_ref_and_names_the_ones_gone() {
    // PR #146 review N1.
    assert!(does_not_resolve(
        "bad args: 1 safe write target(s) do not resolve"
    ));
    assert!(!does_not_resolve("no such function: safe_writes"));
    assert!(!does_not_resolve(
        "bad args: 1 safe write target(s) named twice"
    ));
    let ids: Vec<String> = ["p6", "p7", "p8"].map(String::from).to_vec();
    assert_eq!(
        probe_commands(&ids[..2]),
        vec![
            json!({"target": {"$ref": "p6", "class": "DeviceParameter"}, "name": "get_prop",
                   "args": {"prop": "value"}}),
            json!({"target": {"$ref": "p7", "class": "DeviceParameter"}, "name": "get_prop",
                   "args": {"prop": "value"}}),
        ]
    );
    let stale = json!({"ok": false, "error": "deleted", "errorType": "StaleRef"});
    let unknown = json!({"ok": false, "error": "unknown id", "errorType": "UnknownRef"});
    let busy = json!({"ok": false, "error": "busy", "errorType": "RuntimeError"});
    let value = json!({"ok": true, "data": 0.0});
    assert_eq!(
        gone_refs(&ids, &Ok(vec![stale.clone(), value.clone(), unknown])),
        ["p6", "p8"]
    );
    assert_eq!(
        gone_refs(&ids, &Ok(vec![busy, value.clone(), value])),
        Vec::<String>::new()
    );
    assert_eq!(
        gone_refs(&ids, &Err("instance offline".into())),
        Vec::<String>::new(),
        "nothing known"
    );
    assert_eq!(gone_refs(&ids, &Ok(vec![stale])), ["p6"], "a short answer");
    assert_eq!(REF_GONE, "its rack's selector is gone from Live");
}

/// The next probe of the refs the router asked the scripted Live for (a
/// walk before it is passed over).
fn next_probe(script: &mut Script) -> Asked {
    while let Some(asked) = script.try_call() {
        if asked
            .commands
            .iter()
            .all(|c| c["name"] == "get_prop" && c["target"]["$ref"].is_string())
        {
            return asked;
        }
    }
    panic!("no probe of the refs");
}

#[tokio::test]
async fn a_ref_gone_is_dropped_and_the_rest_registered_again() {
    // PR #146 review N1: the stream's rack was deleted after a walk kept its
    // ref; the script refuses the whole list. The hub asks which ref is
    // gone, drops that target and registers the rest.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    let refused = json!({"ok": false,
        "error": "bad args: 1 safe write target(s) do not resolve", "errorType": "OpError"});
    answered(&mut router, &mut own, asked, Some(refused)).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
    let seq = router.autotune.safe["band"].seq;
    let probe = next_probe(&mut script);
    assert_eq!(
        probe.commands,
        probe_commands(&["p6".to_string(), "p7".to_string()])
    );
    // An answer for another registration is void.
    let stale = json!({"ok": false, "error": "deleted", "errorType": "StaleRef"});
    let gone = Ok(vec![json!({"ok": true, "data": 127.0}), stale.clone()]);
    let ids = vec!["p6".to_string(), "p7".to_string()];
    router.autotune_safe_probed("band", seq + 1, &ids, &gone);
    assert!(router.autotune.targets[STREAM].found.is_some());
    // Its own answer: the stream target dropped (missing, walked again),
    // the PA's DRY write registered alone.
    let walk = router.autotune.targets[STREAM].walk;
    probe.answer(vec![json!({"ok": true, "data": 127.0}), stale]);
    let msg = next_of(&mut own, "the probe's answer", |m| {
        matches!(m, RouterMsg::AutotuneSafeProbed { .. })
    })
    .await;
    router.handle(msg);
    let target = &router.autotune.targets[STREAM];
    assert!(target.found.is_none());
    assert_eq!(target.problem.as_deref(), Some(REF_GONE));
    assert!(target.walk > walk, "walked again");
    assert!(
        router.autotune.targets[REPRO].found.is_some(),
        "the PA kept"
    );
    let (rest, asked) = registration(&mut script);
    assert_eq!(rest, writes(&[("p6", 127.0)]));
    answered(&mut router, &mut own, asked, kept(1)).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    let missing: Vec<Value> = records(&rx)
        .into_iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == "missing")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect();
    assert_eq!(
        missing,
        vec![
            json!({"ev": "autotune", "step": "missing", "instance": "band",
                    "target": "live_set tracks 7", "problem": REF_GONE})
        ]
    );
    // A probe answer once the set is kept again is void.
    let seq = router.autotune.safe["band"].seq;
    router.autotune_safe_probed(
        "band",
        seq,
        &["p6".to_string()],
        &Ok(vec![json!({"ok": false,
        "error": "deleted", "errorType": "StaleRef"})]),
    );
    assert!(router.autotune.targets[REPRO].found.is_some());
    assert!(next_safe(&mut script).is_none());
}

#[tokio::test]
async fn a_probe_that_finds_nothing_gone_drops_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    let refused = json!({"ok": false,
        "error": "bad args: 1 safe write target(s) do not resolve", "errorType": "OpError"});
    answered(&mut router, &mut own, asked, Some(refused)).await;
    let seq = router.autotune.safe["band"].seq;
    let ids = vec!["p6".to_string(), "p7".to_string()];
    router.autotune_safe_probed("band", seq, &ids, &Err("instance offline".into()));
    assert!(router.autotune.targets[STREAM].found.is_some());
    assert!(router.autotune.targets[REPRO].found.is_some());
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
    let _ = next_probe(&mut script);
    assert!(next_safe(&mut script).is_none(), "nothing registered again");
}

#[tokio::test(start_paused = true)]
async fn a_probe_live_did_not_answer_is_asked_again_then_gives_up() {
    // PR #147 review N1: Live is often busy right after a deletion; the
    // probe is retried as the walks are, up to the connect-time bound.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    let asked = both_found(&mut router, &mut own, &mut script).await;
    let refused = json!({"ok": false,
        "error": "bad args: 1 safe write target(s) do not resolve", "errorType": "OpError"});
    answered(&mut router, &mut own, asked, Some(refused)).await;
    for _ in 0..=CONNECT_RETRIES {
        // Live does not answer it (the call dropped: offline).
        drop(next_probe(&mut script));
        pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
    }
    // The last one gave up: no probe more, the status says so.
    while let Some(asked) = script.try_call() {
        let probe = asked
            .commands
            .iter()
            .all(|c| c["name"] == "get_prop" && c["target"]["$ref"].is_string());
        assert!(!probe, "a probe after the give-up: {:?}", asked.commands);
    }
    let safe = &router.autotune.safe["band"];
    assert_eq!(safe.state, SafeState::Failed);
    assert_eq!(safe.probes, CONNECT_RETRIES + 1);
    assert_eq!(
        safe.problem.as_deref(),
        Some("instance offline (gave up after 25 tries, until the next trigger)")
    );
    // Both targets kept: nothing is known gone.
    assert!(router.autotune.targets[STREAM].found.is_some());
    assert!(router.autotune.targets[REPRO].found.is_some());
}
