use std::path::Path;
use std::sync::Arc;

use fohmixer_proto::client::{ServerMsg, set_key};
use fohmixer_proto::roles::Role;
use fohmixer_proto::scenes::{DEFAULT_TIME_MS, SceneMute, SceneRule, ScenesDoc, heard};
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::volume::volume_of;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::reverb::tests::{answer, slot};
use super::super::tests::{attach, attach_as, offline_router, records, set_msg, status};
use super::super::{Router, RouterMsg};
use super::*;
use crate::scenes::SCENES_FILE;

const VOX_VOLUME: &str = "live_set tracks[name=Vox 1] mixer_device volume";
const VOX_TRACK: &str = "live_set tracks[name=Vox 1]";

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn rule(db: Option<f64>, mute: Option<SceneMute>) -> SceneRule {
    SceneRule {
        db,
        mute,
        time_ms: DEFAULT_TIME_MS,
    }
}

/// A router over the offline band serving the synthetic sectors fixture
/// (Vox 1..4, Klavir, Bass, Keys 1, PA Out), taking scenes when `on`,
/// with `doc` as `scenes.json` when given.
fn served(
    dir: &Path,
    on: bool,
    doc: Option<&ScenesDoc>,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(doc) = doc {
        std::fs::write(dir.join(SCENES_FILE), serde_json::to_string(doc).unwrap()).unwrap();
    }
    let store = Arc::new(crate::layout::store_in(
        dir,
        Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, own, rx) = offline_router(dir);
    let mut router = router.with_layout(store).with_scenes(on);
    router.scenes_watch();
    (router, own, rx)
}

/// `intro` and `talk` heard, Vox 1 to −10 dB unmuted on `intro`.
fn ruled() -> ScenesDoc {
    let doc = heard(&heard(&ScenesDoc::empty(), "intro").unwrap(), "talk").unwrap();
    doc.with(
        &band("Vox 1"),
        "intro",
        Some(rule(Some(-10.0), Some(SceneMute::Unmute))),
    )
}

/// The messages waiting in `outbox`.
fn taken(outbox: &crate::outbox::Outbox) -> Vec<ServerMsg> {
    outbox.take().unwrap_or_default()
}

fn of_kind(records: &[Value], ev: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == ev)
        .map(|r| {
            let mut r = r.clone();
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect()
}

fn set_rule(router: &mut Router, client: ClientId, id: &str, scene: &str, rule: Option<SceneRule>) {
    router.handle(RouterMsg::SetSceneRule {
        client,
        id: id.into(),
        strip: band("Vox 1"),
        scene: scene.into(),
        rule,
    });
}

/// Live answers every followed value: Vox 1 at 0 dB and unmuted.
fn live_answers(router: &mut Router) {
    router.subs.connected("band");
    answer(router, |target, prop| match (target, prop) {
        (_, "value") => slot("live_1.value", json!(0.85)),
        (_, "mute") => slot("live_1.mute", json!(false)),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
}

#[test]
fn the_rules_client_is_after_the_recording_watchs() {
    assert_eq!(SCENES_CLIENT, u64::MAX - 7);
    assert_eq!(SCENES_PEER, "rule:scenes");
    assert_eq!(OFF, "scenes are off: no automation token");
}

#[test]
fn the_lines_say_what_changed() {
    assert_eq!(
        rule_line(
            &band("Vox 1"),
            "intro",
            Some(&rule(Some(-10.0), Some(SceneMute::Mute)))
        ),
        "a scene rule changed: band|Vox 1 in scene \"intro\": -10.0 dB, MUTE, 2000 ms"
    );
    assert_eq!(
        rule_line(&band("Vox 1"), "intro", Some(&rule(None, None))),
        "a scene rule changed: band|Vox 1 in scene \"intro\": -, -, 2000 ms"
    );
    assert_eq!(
        rule_line(&band("Vox 1"), "intro", None),
        "a scene rule changed: band|Vox 1 in scene \"intro\": removed"
    );
    assert_eq!(
        scene_line("talk", "lan", "10.0.0.5", 2),
        "scene \"talk\" came from 10.0.0.5 (lan): 2 faders move"
    );
    assert!(full_line("x").contains("keeps 128 names"));
    let e = std::io::Error::other("disk full");
    assert_eq!(
        write_problem(&e),
        "cannot write the scene rules (scenes.json): disk full"
    );
    assert_eq!(
        rule_fields("set", Some(3), Some("10.0.0.5"), json!({"scene": "a"})),
        json!({"what": "set", "client": 3, "peer": "10.0.0.5", "scene": "a"})
    );
}

#[test]
fn a_mute_value_is_a_bool_or_a_number() {
    assert_eq!(mute_on(&json!(true)), Some(true));
    assert_eq!(mute_on(&json!(false)), Some(false));
    assert_eq!(mute_on(&json!(1)), Some(true));
    assert_eq!(mute_on(&json!(0)), Some(false));
    assert_eq!(mute_on(&json!("on")), None);
}

#[test]
fn followed_is_the_volume_and_the_strips_own_mute() {
    let targets = FaderTargets {
        volume: Some(("band".into(), VOX_VOLUME.into())),
        mutes: vec![
            ("band".into(), VOX_TRACK.into()),
            ("band".into(), "live_set tracks 7".into()),
        ],
    };
    assert_eq!(
        followed(&targets),
        vec![
            ("band".to_string(), VOX_VOLUME.to_string(), "value"),
            ("band".to_string(), VOX_TRACK.to_string(), "mute"),
        ]
    );
}

#[tokio::test]
async fn a_hub_without_the_token_tells_no_page_and_refuses_a_rule() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), false, Some(&ruled()));
    let page = attach(&mut router, 1);
    let told = taken(&page);
    assert!(
        !told
            .iter()
            .any(|m| matches!(m, ServerMsg::Scenes { .. } | ServerMsg::Scene { .. })),
        "{told:?}"
    );
    set_rule(&mut router, 1, "r1", "talk", Some(rule(Some(0.0), None)));
    assert_eq!(
        taken(&page),
        vec![ServerMsg::Error {
            id: Some("r1".into()),
            message: OFF.into()
        }]
    );
    assert!(of_kind(&records(&rx), "scene_rule").is_empty());
    assert!(!status(&mut router).scenes.on);
    // It follows nothing.
    assert!(router.scenes.watched.is_empty());
}

#[tokio::test]
async fn a_rule_is_set_recorded_told_and_followed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, Some(&ruled()));
    // Vox 1's volume and mute followed from the start.
    assert_eq!(
        router.scenes.watched.keys().cloned().collect::<Vec<_>>(),
        vec![
            set_key("band", VOX_VOLUME, "value"),
            set_key("band", VOX_TRACK, "mute"),
        ]
    );
    let page = attach(&mut router, 1);
    assert!(
        taken(&page).contains(&ServerMsg::Scenes { doc: ruled() }),
        "the document at attach"
    );
    let talk = rule(None, Some(SceneMute::Mute));
    set_rule(&mut router, 1, "r1", "talk", Some(talk));
    let after = ruled().with(&band("Vox 1"), "talk", Some(talk));
    assert_eq!(
        taken(&page),
        vec![
            ServerMsg::Result {
                id: "r1".into(),
                data: Vec::new()
            },
            ServerMsg::Scenes { doc: after.clone() },
        ]
    );
    let on_disk: ScenesDoc =
        serde_json::from_slice(&std::fs::read(dir.path().join(SCENES_FILE)).unwrap()).unwrap();
    assert_eq!(on_disk, after);
    let recs = of_kind(&records(&rx), "scene_rule");
    assert_eq!(
        recs,
        vec![
            json!({"ev": "scene_rule", "what": "set", "client": 1, "peer": "10.0.0.1",
                    "strip": {"instance": "band", "label": "Vox 1"}, "scene": "talk",
                    "rule": {"db": null, "mute": "mute", "time_ms": 2000}})
        ]
    );
    // The same again: only `result`, no record.
    set_rule(&mut router, 1, "r2", "talk", Some(talk));
    assert_eq!(
        taken(&page),
        vec![ServerMsg::Result {
            id: "r2".into(),
            data: Vec::new()
        }]
    );
    assert!(of_kind(&records(&rx), "scene_rule").is_empty());
    // A refusal: an unknown scene, the asker alone.
    set_rule(&mut router, 1, "r3", "song", Some(talk));
    assert_eq!(
        taken(&page),
        vec![ServerMsg::Error {
            id: Some("r3".into()),
            message: "unknown scene".into()
        }]
    );
    // The last rule removed: nothing is followed any more.
    set_rule(&mut router, 1, "r4", "talk", None);
    set_rule(&mut router, 1, "r5", "intro", None);
    assert!(router.scenes.watched.is_empty());
    assert_eq!(status(&mut router).scenes.strips, 0);
}

#[tokio::test]
async fn an_mbc_login_cannot_set_a_band_rule() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(dir.path(), true, Some(&ruled()));
    let page = attach_as(&mut router, 1, Role::Mbc);
    taken(&page);
    set_rule(&mut router, 1, "r1", "talk", Some(rule(Some(0.0), None)));
    assert_eq!(
        taken(&page),
        vec![ServerMsg::Error {
            id: Some("r1".into()),
            message: crate::ws::role::REFUSED.into()
        }]
    );
}

#[tokio::test]
async fn a_scene_learns_its_name_records_tells_and_answers_what_moved() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, Some(&ruled()));
    live_answers(&mut router);
    let page = attach(&mut router, 1);
    taken(&page);
    // A new name: learned and told; no fader has a rule for it.
    assert_eq!(router.scene_came("song", "lan", "10.0.0.5"), 0);
    let told = taken(&page);
    assert!(matches!(&told[0], ServerMsg::Scenes { doc } if doc.knows("song")));
    assert!(
        matches!(&told[1], ServerMsg::Scene { name, moved: 0, at_ms } if name == "song" && *at_ms > 0.0),
        "{told:?}"
    );
    assert_eq!(
        of_kind(&records(&rx), "scene"),
        vec![
            json!({"ev": "scene", "scene": "song", "source": "lan", "peer": "10.0.0.5", "moved": 0})
        ]
    );
    // `intro`: Vox 1 glides from 0 dB to −10 dB; already unmuted.
    assert_eq!(router.scene_came("intro", "lan", "10.0.0.5"), 1);
    let glides = of_kind(&records(&rx), "scene_glide");
    assert_eq!(
        glides,
        vec![
            json!({"ev": "scene_glide", "what": "start", "instance": "band", "label": "Vox 1",
                    "scene": "intro", "from_db": 0.0, "to_db": -10.0, "ms": 2000})
        ]
    );
    let now = status(&mut router).scenes;
    assert_eq!(now.glides, 1);
    assert_eq!(now.last.as_deref(), Some("intro"));
    assert_eq!(now.known, 3);
    // A later page is told the last scene.
    let later = attach(&mut router, 2);
    assert!(
        taken(&later)
            .iter()
            .any(|m| matches!(m, ServerMsg::Scene { name, moved: 1, .. } if name == "intro"))
    );
}

#[tokio::test]
async fn a_glide_steps_through_the_setter_and_a_pages_set_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, Some(&ruled()));
    live_answers(&mut router);
    attach(&mut router, 1);
    router.scene_came("intro", "lan", "10.0.0.5");
    // The wake: a step through the setter as the rule's own client.
    let generation = router.scenes.generation;
    router.handle(RouterMsg::ScenesWake { generation });
    let sets: Vec<Value> = of_kind(&records(&rx), "set");
    assert!(
        sets.iter().any(|s| s["client"] == json!(SCENES_CLIENT)
            && s["key"] == set_key("band", VOX_VOLUME, "value")),
        "{sets:?}"
    );
    assert!(router.scenes.rule.writes() >= 1);
    // A stale wake does nothing.
    let writes = router.scenes.rule.writes();
    router.handle(RouterMsg::ScenesWake {
        generation: generation.wrapping_sub(1),
    });
    assert_eq!(router.scenes.rule.writes(), writes);
    // The page's set of Vox 1's volume: the hand wins.
    router.handle(set_msg_for(1, VOX_VOLUME, 9, 0.5));
    let stops = of_kind(&records(&rx), "scene_glide");
    assert_eq!(
        stops,
        vec![
            json!({"ev": "scene_glide", "what": "stop", "instance": "band", "label": "Vox 1",
                    "scene": "intro", "why": "hand"})
        ]
    );
    assert_eq!(router.scenes.rule.glides(), 0);
}

#[tokio::test]
async fn an_instance_going_offline_ends_its_glides() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, Some(&ruled()));
    live_answers(&mut router);
    router.scene_came("intro", "lan", "10.0.0.5");
    records(&rx);
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: crate::LiveEvent::Disconnected,
    });
    let stops = of_kind(&records(&rx), "scene_glide");
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["why"], "offline");
}

#[tokio::test]
async fn a_scene_with_no_value_heard_writes_the_target_and_the_unmute_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, Some(&ruled()));
    assert_eq!(router.scene_came("intro", "lan", "10.0.0.5"), 1);
    let sets = of_kind(&records(&rx), "set");
    let written: Vec<(Value, Value)> = sets
        .iter()
        .map(|s| (s["key"].clone(), s["value"].clone()))
        .collect();
    assert_eq!(
        written,
        vec![
            (json!(set_key("band", VOX_TRACK, "mute")), json!(false)),
            (
                json!(set_key("band", VOX_VOLUME, "value")),
                json!(volume_of(-10.0))
            ),
        ]
    );
    assert_eq!(router.scenes.rule.glides(), 0);
}

#[tokio::test]
async fn a_hand_edit_is_taken_recorded_and_followed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), true, None);
    assert!(router.scenes.watched.is_empty());
    let page = attach(&mut router, 1);
    taken(&page);
    std::fs::write(
        dir.path().join(SCENES_FILE),
        serde_json::to_string(&ruled()).unwrap(),
    )
    .unwrap();
    router.handle(RouterMsg::ScenesPoll);
    assert_eq!(taken(&page), vec![ServerMsg::Scenes { doc: ruled() }]);
    assert_eq!(router.scenes.watched.len(), 2);
    assert_eq!(
        of_kind(&records(&rx), "scene_rule"),
        vec![json!({"ev": "scene_rule", "what": "file", "client": null, "peer": null})]
    );
    // Removed: written back and recorded.
    std::fs::remove_file(dir.path().join(SCENES_FILE)).unwrap();
    router.handle(RouterMsg::ScenesPoll);
    assert!(dir.path().join(SCENES_FILE).exists());
    assert_eq!(
        of_kind(&records(&rx), "scene_rule"),
        vec![
            json!({"ev": "scene_rule", "what": "file", "client": null, "peer": null, "why": "deleted"})
        ]
    );
}

#[tokio::test]
async fn the_router_answers_a_scene_message_with_what_moved() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(dir.path(), true, Some(&ruled()));
    let (reply, answer) = tokio::sync::oneshot::channel();
    router.handle(RouterMsg::Scene {
        name: "intro".into(),
        source: "lan".into(),
        peer: "10.0.0.5".into(),
        reply,
    });
    assert_eq!(answer.await.unwrap(), 1);
}

/// A page's `set` of `prop` value of `target` in band.
fn set_msg_for(client: ClientId, target: &str, seq: u64, value: f64) -> RouterMsg {
    let mut msg = set_msg(client, "band", seq, value);
    if let RouterMsg::Set { target: t, .. } = &mut msg {
        *t = target.to_string();
    }
    msg
}
