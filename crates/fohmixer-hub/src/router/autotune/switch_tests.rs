use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{
    Decision, HUB_AUTOTUNE, HUB_AUTOTUNE_AUT, HUB_AUTOTUNE_SINGER, SafeState, ServerMsg,
    SingerState, SwitchWhy, TargetState, TrackState,
};
use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding};
use fohmixer_proto::songmap::{ClipName, Cue, Key, SectionsState};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::super::songmap::Listing;
use super::super::super::tests::{attach, next_of, records, status};
use super::super::super::{Router, RouterIo, RouterMsg};
use super::super::safe::{SAFE_WRITES, Safe};
use super::{
    NO_SINGER, armed, checked, checking, has_problem, key_why, part, singer_label, singer_value,
    track_state, trouble, waiting_words,
};
use crate::live::client::{Asked, LiveEvent, Script, scripted};
use crate::rules::HubState;
use crate::rules::autotune::find::{Found, KeyFound, Param};
use crate::rules::autotune::keys::{KeyDevice, KeyMissing, KeyParams, KeyState};
use crate::rules::autotune::switch::{KeyCheck, Refusal};

/// The PA target (an Auto-Tune Artist, on 0, off 127) and the stream target
/// (an AudioGridder, on 127, off 0) of one singer, VOX1.
const PA: &str = "band|live_set tracks 6";
const STREAM: &str = "band|live_set tracks 7";
/// The band's playhead, tempo and transport (hub keys).
const TIME: &str = "band|live_set|current_song_time|false";
const TEMPO_KEY: &str = "band|live_set|tempo|false";
const TRANSPORT: &str = "band|live_set|is_playing|false";
/// Live's words for a parameter a Max map holds.
const DISABLED: &str = "live error: Value cannot be set, the parameter is disabled";

fn target(index: u32, label: &str, (on, off): (u8, u8), singer: Option<&str>) -> AutotuneTarget {
    AutotuneTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: None,
        },
        label: label.into(),
        on,
        off,
        singer: singer.map(str::to_string),
        two_singers: false,
    }
}

fn pa() -> AutotuneTarget {
    target(6, "Vox 2", (0, 127), Some("VOX1"))
}

fn stream() -> AutotuneTarget {
    target(7, "#8", (127, 0), Some("VOX1"))
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

/// Band track `index`'s key device: the Auto-Tune Artist on track 6,
/// AudioGridder on the others; Key and Scale `$ref`s `k<index>`, `s<index>`.
fn device(index: u32) -> KeyFound {
    let plugin = format!("live_set tracks {index} devices 0 chains 0 devices 0");
    let kind = if index == 6 {
        KeyDevice::AutoTune
    } else {
        KeyDevice::Gridder
    };
    KeyFound {
        params: KeyParams {
            device: kind,
            key: 1,
            scale: 2,
        },
        device: plugin.clone(),
        key: Param {
            path: format!("{plugin} parameters 1"),
            id: format!("k{index}"),
        },
        scale: Param {
            path: format!("{plugin} parameters 2"),
            id: format!("s{index}"),
        },
    }
}

/// Band track `index`'s `Latencies` rack: its selector `p<index>`.
fn rack(index: u32) -> Found {
    Found {
        path: format!("live_set tracks {index} devices 0 parameters 1"),
        id: format!("p{index}"),
        key: Ok(device(index)),
    }
}

/// Song 1 (`001 Test One {D 120}`) from 0 to 100 with a key clip to G at
/// 50, song 2 in D too from 120 to 200, song 3 with no key from 210 to 300
/// (invented titles).
fn songs() -> Listing {
    let cue = |name: &str, time: f64| Cue {
        name: name.to_string(),
        time: Some(time),
    };
    Listing {
        cues: vec![
            cue("001 Test One {D 120}", 0.0),
            cue("SONG END", 100.0),
            cue("002 Test Two {D 90}", 120.0),
            cue("SONG END", 200.0),
            cue("003 Test Three", 210.0),
            cue("SONG END", 300.0),
        ],
        clips: vec![ClipName {
            name: "Key {G}".into(),
            start: Some(50.0),
        }],
        sections: SectionsState::One,
        track: None,
        sections_error: None,
    }
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

/// A command's `$ref`, if it goes through one.
fn ref_of(command: &Value) -> Option<&str> {
    command["target"]["$ref"].as_str()
}

/// Whether a command sets or reads Live's bar.
fn is_bar(command: &Value) -> bool {
    command["target"] == "live_set"
        && (command["args"]["prop"] == "root_note" || command["args"]["prop"] == "scale_name")
}

/// Whether a call writes selectors: every command a `set_prop` through a
/// selector's `$ref` (`p…`) with class `DeviceParameter`, never a path.
fn is_selector_write(commands: &[Value]) -> bool {
    !commands.is_empty()
        && commands.iter().all(|c| {
            c["name"] == "set_prop"
                && ref_of(c).is_some_and(|id| id.starts_with('p'))
                && c["target"]["class"] == "DeviceParameter"
        })
}

/// A scripted Live that answers as SimLive would: the DRY registrations
/// kept, the key writes taken and read back, the selector writes taken
/// with their echo to the hub; a walk's call is dropped (each test answers
/// the walks itself).
struct Live {
    script: Script,
    own: UnboundedReceiver<RouterMsg>,
    /// Each parameter's value by `$ref`, and Live's bar by property.
    params: BTreeMap<String, Value>,
    /// Each selector batch: its writes (`$ref`, value).
    batches: Vec<Vec<(String, f64)>>,
    /// The refs a Max map holds (a set is refused).
    held: BTreeSet<String>,
    /// The refs whose object was deleted (answered stale).
    gone: BTreeSet<String>,
}

impl Live {
    /// Its answer to one command.
    fn slot(&mut self, command: &Value, echoes: &mut Vec<(String, f64)>) -> Value {
        let id = ref_of(command)
            .map(str::to_string)
            .unwrap_or_else(|| command["args"]["prop"].as_str().unwrap_or("").to_string());
        if self.gone.contains(&id) {
            return json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"});
        }
        if command["name"] == "set_prop" {
            if self.held.contains(&id) {
                return json!({"ok": false, "error": DISABLED, "errorType": "RuntimeError"});
            }
            let value = command["args"]["value"].clone();
            if id.starts_with('p') {
                echoes.push((id.clone(), value.as_f64().unwrap()));
            }
            self.params.insert(id, value);
            return ok(Value::Null);
        }
        ok(self.params.get(&id).cloned().unwrap_or_else(|| json!(0.0)))
    }

    /// Answers one call as Live would.
    fn answer(&mut self, router: &mut Router, asked: Asked) {
        let commands = asked.commands.clone();
        if commands.len() == 1 && commands[0]["name"] == SAFE_WRITES {
            let writes = commands[0]["args"]["writes"].as_array().unwrap();
            let gone = writes
                .iter()
                .filter(|w| self.gone.contains(w["target"]["$ref"].as_str().unwrap()))
                .count();
            let slot = if gone > 0 {
                json!({"ok": false, "error": format!("bad args: {gone} safe write target(s) do not resolve"),
                       "errorType": "OpError"})
            } else {
                ok(json!({"armed": writes.len()}))
            };
            asked.answer(vec![slot]);
            return;
        }
        let ours = commands.iter().all(|c| ref_of(c).is_some() || is_bar(c));
        if !ours {
            // A walk: dropped.
            return;
        }
        if is_selector_write(&commands) {
            self.batches.push(
                commands
                    .iter()
                    .map(|c| {
                        (
                            ref_of(c).unwrap().to_string(),
                            c["args"]["value"].as_f64().unwrap(),
                        )
                    })
                    .collect(),
            );
        }
        let mut echoes = Vec::new();
        let slots: Vec<Value> = commands.iter().map(|c| self.slot(c, &mut echoes)).collect();
        asked.answer(slots);
        for (id, value) in echoes {
            deliver_macro(router, &id, value);
        }
    }

    /// Answers every call and handles every answer until nothing moves.
    async fn settle(&mut self, router: &mut Router) {
        for _ in 0..100 {
            let mut moved = false;
            while let Some(asked) = self.script.try_call() {
                moved = true;
                self.answer(router, asked);
            }
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            while let Ok(msg) = self.own.try_recv() {
                if matches!(
                    msg,
                    RouterMsg::AutotuneSafe { .. }
                        | RouterMsg::AutotuneKeysWritten { .. }
                        | RouterMsg::AutotuneKeysRead { .. }
                        | RouterMsg::AutotuneWritten { .. }
                        | RouterMsg::AutotuneSafeProbed { .. }
                ) {
                    moved = true;
                    router.handle(msg);
                }
            }
            if !moved {
                return;
            }
        }
        panic!("Live never went quiet");
    }

    /// The next call that `want` takes (a walk before it is dropped; any
    /// other call fails the test).
    fn next(&mut self, what: &str, want: impl Fn(&[Value]) -> bool) -> Asked {
        while let Some(asked) = self.script.try_call() {
            if want(asked.commands.as_slice()) {
                return asked;
            }
            let walk = asked
                .commands
                .iter()
                .all(|c| ref_of(c).is_none() && !is_bar(c) && c["name"] == "get_prop");
            assert!(walk, "{what}: another call first: {:?}", asked.commands);
        }
        panic!("no {what}");
    }

    /// Answers every call waiting but handles none of the answers (a
    /// selector write among them fails the test).
    fn quiet(&mut self) {
        while let Some(asked) = self.script.try_call() {
            assert!(
                !is_selector_write(&asked.commands),
                "a selector write: {:?}",
                asked.commands
            );
            self.answer_later(asked);
        }
    }

    /// Answers a call `quiet` took (a walk is dropped).
    fn answer_later(&mut self, asked: Asked) {
        let commands = asked.commands.clone();
        let mut echoes = Vec::new();
        if commands.iter().all(|c| ref_of(c).is_some() || is_bar(c)) {
            let slots: Vec<Value> = commands.iter().map(|c| self.slot(c, &mut echoes)).collect();
            asked.answer(slots);
        } else if commands.len() == 1 && commands[0]["name"] == SAFE_WRITES {
            let n = commands[0]["args"]["writes"].as_array().map_or(0, Vec::len);
            asked.answer(vec![ok(json!({"armed": n}))]);
        }
    }

    /// The selector batches written since the last look.
    fn written(&mut self) -> Vec<Vec<(String, f64)>> {
        std::mem::take(&mut self.batches)
    }
}

/// Live's value of the selector `id` reaches the hub (its echo, or a hand
/// on it in Live), as the flush hands it over.
fn deliver_macro(router: &mut Router, id: &str, value: f64) {
    let sub = router
        .autotune
        .targets
        .values()
        .find(|t| t.found.as_ref().is_some_and(|f| f.id == id))
        .and_then(|t| t.sub.clone());
    if let Some(sub) = sub {
        router.autotune_delivery(&sub, Some(json!(value)), None);
    }
}

/// Another writer sets the selector `id` in Live.
fn by_hand(router: &mut Router, live: &mut Live, id: &str, value: f64) {
    live.params.insert(id.to_string(), json!(value));
    deliver_macro(router, id, value);
}

/// A batch of selector writes.
fn batch(writes: &[(&str, f64)]) -> Vec<(String, f64)> {
    writes.iter().map(|(id, v)| (id.to_string(), *v)).collect()
}

/// A delivery of one of AUTOTUNE's subscriptions, as the flush hands it.
fn deliver(router: &mut Router, sub: &str, value: f64) {
    router.autotune_delivery(sub, Some(json!(value)), None);
}

/// A target's key device's Key and Scale values heard.
fn heard(router: &mut Router, key: &str, values: (f64, f64)) {
    let target = &router.autotune.targets[key];
    let (key_sub, scale_sub) = (
        target.key_sub.clone().unwrap(),
        target.scale_sub.clone().unwrap(),
    );
    deliver(router, &key_sub, values.0);
    deliver(router, &scale_sub, values.1);
}

/// A target's newest walk answers `found`.
fn find(router: &mut Router, key: &str, found: Found) {
    let walk = router.autotune.targets[key].walk;
    router.autotune_found(key, walk, Ok(found));
}

/// The band's transport heard, as the flush hands it over.
fn transport(router: &mut Router, playing: bool) {
    router.on_transport(TRANSPORT, &json!(playing));
}

/// AUTOTUNE AUT on, as page 1's hold sets it (#140 PR 3c).
fn aut_on(router: &mut Router) {
    router.autotune_flag(1, true);
}

fn set_hub(router: &mut Router, key: &str, value: Value) {
    router.handle(RouterMsg::SetHub {
        client: 1,
        key: key.into(),
        value,
    });
}

/// The `autotune` records waiting, without `ev` and `ts`.
fn steps(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "autotune")
        .map(|mut r| {
            let fields = r.as_object_mut().unwrap();
            fields.remove("ev");
            fields.remove("ts");
            r
        })
        .collect()
}

/// The records of `step` among `got`.
fn of(got: &[Value], step: &str) -> Vec<Value> {
    got.iter().filter(|r| r["step"] == step).cloned().collect()
}

/// A `select` record of the band's target on track `index`.
fn select(index: u32, on: bool, why: &str, cause: &str) -> Value {
    json!({"step": "select", "instance": "band", "target": format!("live_set tracks {index}"),
           "on": on, "why": why, "cause": cause})
}

/// Each target's decision in `/api/status`, in Live's order.
fn decisions(router: &mut Router) -> Vec<Option<Decision>> {
    status(router)
        .autotune
        .targets
        .iter()
        .map(|t| t.decision)
        .collect()
}

fn on() -> Option<Decision> {
    Some(Decision {
        on: true,
        why: SwitchWhy::Play,
    })
}

fn off(why: SwitchWhy) -> Option<Decision> {
    Some(Decision { on: false, why })
}

/// A router whose band is a scripted Live (`state` as loaded), its Live.
fn scripted_router(
    dir: &std::path::Path,
    state: HubState,
) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    let (handle, script) = scripted("band");
    let (tx, own) = tokio::sync::mpsc::unbounded_channel();
    let (events, records) = crate::events::EventLog::channel(4096);
    let router = Router::new(
        BTreeMap::from([("band".to_string(), handle)]),
        state,
        dir.to_path_buf(),
        RouterIo { tx, events },
    );
    let live = Live {
        script,
        own,
        params: BTreeMap::new(),
        batches: Vec::new(),
        held: BTreeSet::new(),
        gone: BTreeSet::new(),
    };
    (router, live, records)
}

/// The band's song map read `read_no` answers `listing`.
fn map_read(router: &mut Router, read_no: u64, listing: Listing) {
    router.songmap.reading.insert("band".into(), read_no);
    router.handle(RouterMsg::SongMapRead {
        instance: "band".into(),
        read_no,
        result: Ok(listing),
        ms: 5,
    });
}

/// Both racks found and their key devices' values heard, each selector on
/// DRY in Live (the PA's at 127, the stream's at 0), the song map read and
/// Live standing at 10 in song 1 (D): the keys written and confirmed, the
/// DRY writes kept. AUTOTUNE AUT as `state` has it.
async fn ready(
    dir: &std::path::Path,
    state: HubState,
) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    ready_with(dir, state, songs()).await
}

/// [`ready`] on the song map `listing`.
async fn ready_with(
    dir: &std::path::Path,
    state: HubState,
    listing: Listing,
) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    let (mut router, mut live, rx) = scripted_router(dir, state);
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    heard(&mut router, PA, (0.75, 0.0));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    by_hand(&mut router, &mut live, "p6", 127.0);
    by_hand(&mut router, &mut live, "p7", 0.0);
    transport(&mut router, false);
    map_read(&mut router, 1, listing);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    let keys: Vec<Option<KeyState>> = status(&mut router)
        .autotune
        .targets
        .iter()
        .map(|t| t.key.as_ref().and_then(|k| k.state))
        .collect();
    assert_eq!(keys, [Some(KeyState::Confirmed), Some(KeyState::Confirmed)]);
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "both on DRY"
    );
    let _ = steps(&rx);
    (router, live, rx)
}

/// [`ready`], then AUTOTUNE AUT on and Live playing: both tracks tune.
async fn tuning(dir: &std::path::Path) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    let (mut router, mut live, rx) = ready(dir, HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
    let _ = steps(&rx);
    (router, live, rx)
}

#[test]
fn the_glues_small_decisions() {
    // The fall back is armed for a selector only while both the list the
    // script last kept and the newest one sent have it with its off value
    // (PR #146 review I2, the PR #147 ruling on I2).
    let both = vec![("p6".to_string(), 127), ("p7".to_string(), 0)];
    let safe = |list: &[(String, u8)], kept: Option<&[(String, u8)]>, state| Safe {
        list: list.to_vec(),
        seq: 3,
        state,
        problem: None,
        failures: 0,
        kept: kept.map(<[(String, u8)]>::to_vec),
        probes: 0,
        lost: false,
    };
    let kept = safe(both.as_slice(), Some(both.as_slice()), SafeState::Kept);
    assert!(armed(Some(&kept), "p6", 127));
    assert!(armed(Some(&kept), "p7", 0));
    assert!(!armed(Some(&kept), "p7", 127), "another off value");
    assert!(!armed(Some(&kept), "p8", 0), "not in the list");
    assert!(!armed(None, "p6", 127), "nothing registered");
    // A list that adds a target, on its way or refused: the two it shares
    // with the kept one stay armed, the new one is not.
    let more = vec![both[0].clone(), both[1].clone(), ("p8".to_string(), 0)];
    for state in [SafeState::Sent, SafeState::Failed] {
        let adding = safe(more.as_slice(), Some(both.as_slice()), state);
        assert!(armed(Some(&adding), "p6", 127), "{state:?}");
        assert!(armed(Some(&adding), "p7", 0), "{state:?}");
        assert!(!armed(Some(&adding), "p8", 0), "{state:?}");
    }
    // A list that drops a selector's write: no longer armed, whatever the
    // script holds; nothing kept yet: none armed.
    let fewer = safe(&both[..1], Some(both.as_slice()), SafeState::Sent);
    assert!(armed(Some(&fewer), "p6", 127));
    assert!(!armed(Some(&fewer), "p7", 0));
    let first = safe(both.as_slice(), None, SafeState::Sent);
    assert!(!armed(Some(&first), "p6", 127));
    // A singer's off by hand: `{id, off}`, nothing more.
    assert_eq!(
        singer_value(&json!({"id": "VOX1", "off": true})),
        Some(("VOX1".to_string(), true))
    );
    assert_eq!(
        singer_value(&json!({"off": false, "id": "#Vox 7"})),
        Some(("#Vox 7".to_string(), false))
    );
    for bad in [
        json!({"id": "VOX1"}),
        json!({"id": "VOX1", "off": 1}),
        json!({"id": 1, "off": true}),
        json!({"id": "VOX1", "off": true, "more": 1}),
        json!({"name": "VOX1", "off": true}),
        json!({"id": "VOX1", "on": true}),
        json!(true),
        json!(["VOX1", true]),
    ] {
        assert_eq!(singer_value(&bad), None, "{bad}");
    }
    // A key being checked: AUT on, Live playing in a keyed song, a key on
    // its way.
    let d = Some(Key {
        root: 2,
        minor: false,
    });
    let pending = || [KeyCheck::Confirmed, KeyCheck::Pending].into_iter();
    assert!(checking(true, Some(true), d, pending()));
    assert!(!checking(false, Some(true), d, pending()), "AUT off");
    assert!(!checking(true, Some(false), d, pending()), "stopped");
    assert!(!checking(true, None, d, pending()), "not heard");
    assert!(!checking(true, Some(true), None, pending()), "no key");
    assert!(!checking(
        true,
        Some(true),
        d,
        [KeyCheck::Confirmed, KeyCheck::Bad].into_iter()
    ));
    // A singer's tracks, part and label.
    assert_eq!(track_state(None), TrackState::None);
    assert_eq!(track_state(Some(TargetState::On)), TrackState::Tune);
    for dry in [TargetState::Off, TargetState::Waiting, TargetState::Missing] {
        assert_eq!(track_state(Some(dry)), TrackState::Dry, "{dry:?}");
    }
    assert_eq!((part(0), part(127), part(64)), ("PA", "stream", "stream"));
    assert_eq!(singer_label(&["#8", "Vox 2"]), "Vox 2");
    assert_eq!(singer_label(&["Vox 1", "Vox 2"]), "Vox 1");
    assert_eq!(singer_label(&["#8", "#9"]), "#8");
    assert_eq!(singer_label(&[]), "");
    // Why a key is wrong.
    let f = Key {
        root: 5,
        minor: false,
    };
    let g = Key {
        root: 7,
        minor: false,
    };
    assert_eq!(
        key_why("stream", KeyState::Mismatch, Some(f), Some(g)),
        "stream Key reads G, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::AnotherWriter, Some(f), None),
        "PA Key reads no key, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::Refused, Some(f), Some(g)),
        "PA Key refused, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::Mismatch, None, Some(g)),
        "PA Key reads G, wrote no key"
    );
}

#[tokio::test]
async fn a_targets_problems_are_its_walks_its_tuner_its_key_device_and_another_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, rack(7));
    let fine = router.autotune.targets[STREAM].clone();
    assert!(!has_problem(&fine));
    assert_eq!(trouble(&fine), None);
    let mut walks = fine.clone();
    walks.problem = Some("gave up".into());
    assert!(has_problem(&walks));
    assert_eq!(trouble(&walks).as_deref(), Some("gave up"));
    let mut two = fine.clone();
    two.spec.two_singers = true;
    assert!(has_problem(&two));
    assert_eq!(
        trouble(&two).as_deref(),
        Some("its Tuner names two singers")
    );
    let mut keyless = fine.clone();
    keyless.key = Some(Err("chain 0 holds 2 plug-ins, not one".into()));
    assert!(has_problem(&keyless));
    assert_eq!(
        trouble(&keyless).as_deref(),
        Some("chain 0 holds 2 plug-ins, not one")
    );
    let mut lost = fine.clone();
    lost.key = None;
    assert!(!has_problem(&lost), "a key device looked for again");
    // Only a found target with a key device and no problem counts for the
    // pages' `checking` (PR #147 review N3).
    assert!(checked(&fine));
    assert!(!checked(&walks) && !checked(&keyless) && !checked(&two));
    assert!(!checked(&lost), "no key device followed now");
    let mut unfound = fine.clone();
    unfound.found = None;
    assert!(!checked(&unfound));
    // No id and a placeholder label: no singer, a problem (PR #147 review
    // N6); a label of its own is a singer.
    let mut lone = fine.clone();
    lone.spec.singer = None;
    assert!(has_problem(&lone));
    assert_eq!(trouble(&lone).as_deref(), Some(NO_SINGER));
    lone.spec.label = "Vox 9".into();
    assert!(!has_problem(&lone));
    assert_eq!(trouble(&lone), None);
    // Live refused its selector for good (PR #147 review I5): shown, its
    // decision reads it on its own (no problem input).
    let mut refused = fine.clone();
    refused.refusal = Some(Refusal::after(
        Some(&Refusal::after(None, "p7", true, "held")),
        "p7",
        true,
        "held",
    ));
    assert!(!has_problem(&refused));
    assert_eq!(
        trouble(&refused).as_deref(),
        Some("Live refuses its selector: held")
    );
    let mut once = fine.clone();
    once.refusal = Some(Refusal::after(None, "p7", true, "held"));
    assert_eq!(trouble(&once), None, "its retry goes");
    let mut other_ref = refused.clone();
    other_ref.found = Some(rack(8));
    other_ref.problem = None;
    assert_eq!(trouble(&other_ref), None, "a new ref");
    let mut writer = fine;
    writer.guard.left();
    assert!(!has_problem(&writer), "the guard is its own input");
    assert_eq!(
        trouble(&writer).as_deref(),
        Some("another writer moves its selector")
    );
}

#[tokio::test]
async fn aut_off_writes_dry_to_a_selector_that_shows_the_chain() {
    // Spec §2.2: AUT off, every singer on DRY, always; one write per
    // change.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    // The PA's selector on its chain (0), the stream's on DRY.
    deliver_macro(&mut router, "p6", 0.0);
    deliver_macro(&mut router, "p7", 0.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "found", "on": [],
                    "off": ["Vox 2"]})
        ]
    );
    assert_eq!(of(&got, "select"), Vec::<Value>::new(), "no switch: DRY");
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)]
    );
    // Its echo: nothing more; found again: nothing more.
    find(&mut router, PA, rack(6));
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(status(&mut router).autotune.writes, 1);
}

#[tokio::test]
async fn aut_on_and_playing_in_a_keyed_song_switches_on_after_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // Out of every song, AUT on, Live plays: DRY (span).
    deliver(&mut router, TIME, 105.0);
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    // Song 2 starts: its key goes first; no selector moves before its
    // read-back confirms it.
    deliver(&mut router, TIME, 125.0);
    let write = live.next("the song start's key write", |c| {
        c.iter().all(|c| c["name"] == "set_prop") && !is_selector_write(c)
    });
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Mismatch), off(SwitchWhy::Mismatch)]
    );
    let n = write.commands.len();
    let commands = write.commands.clone();
    for c in &commands {
        live.params.insert(
            ref_of(c)
                .map(str::to_string)
                .unwrap_or_else(|| c["args"]["prop"].as_str().unwrap().to_string()),
            c["args"]["value"].clone(),
        );
    }
    write.answer(vec![ok(Value::Null); n]);
    let msg = next_of(&mut live.own, "the key write's answer", |m| {
        matches!(m, RouterMsg::AutotuneKeysWritten { .. })
    })
    .await;
    router.handle(msg);
    live.quiet();
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "still checking"
    );
    assert!(router.autotune.state().checking);
    // The read-back confirms both: one batch, each its own on value.
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert!(!router.autotune.state().checking);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, true, "play", "key"),
            select(7, true, "play", "key")
        ]
    );
    assert_eq!(
        of(&got, "write"),
        vec![json!({"step": "write", "instance": "band", "cause": "key",
                    "on": ["Vox 2", "#8"], "off": []})]
    );
    assert_eq!(router.autotune.state().targets.len(), 2);
}

#[tokio::test]
async fn aut_on_with_the_keys_confirmed_switches_on_at_once() {
    // The keys follow the song position with AUT off too (spec §2.2), so
    // AUT on in a keyed song tunes at once.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "AUT off");
    aut_on(&mut router);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, true, "play", "flag"),
            select(7, true, "play", "flag")
        ]
    );
    assert_eq!(
        of(&got, "flag"),
        vec![json!({"step": "flag", "on": true, "client": 1, "peer": null})]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_aut"], json!(true), "saved on");
    // AUT off: DRY at once.
    set_hub(&mut router, HUB_AUTOTUNE_AUT, json!(false));
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)]
    );
}

#[tokio::test]
async fn a_stop_mid_song_writes_dry_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    // At once, before any read-back: the call is on its way already.
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        vec![
            json!({"target": {"$ref": "p6", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 127.0}}),
            json!({"target": {"$ref": "p7", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 0.0}}),
        ]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, false, "stop", "transport"),
            select(7, false, "stop", "transport")
        ]
    );
    // A lost transport is no play either.
    drop(dry);
    let lost = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(lost.path()).await;
    router.transport_failed(TRANSPORT, "not observable");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
}

#[tokio::test]
async fn out_of_a_song_and_a_keyless_song_are_dry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // Song 1 ends: DRY (span).
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    // Song 3 has no key: DRY stays (no_key), nothing written again.
    deliver(&mut router, TIME, 215.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::NoKey), off(SwitchWhy::NoKey)]
    );
    let state = router.autotune.state();
    assert_eq!((state.song, state.key), (Some(3), None));
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, false, "span", "position"),
            select(7, false, "span", "position")
        ]
    );
}

#[tokio::test]
async fn a_span_ends_and_starts_at_the_cues_own_value() {
    // The PC's cue times are the beat plus 5.2e-5 (hub-rust.md, the song
    // map): a playhead between the rounded beat and Live's value is still
    // in the song before an end, and still out of every song before a
    // start.
    let cue = |name: &str, time: f64| Cue {
        name: name.to_string(),
        time: Some(time),
    };
    let listing = Listing {
        cues: vec![
            cue("001 Test One {D 120}", 0.0),
            cue("SONG END", 100.000052291458),
            cue("002 Test Two {D 90}", 120.000052291458),
            cue("SONG END", 200.000052291458),
        ],
        clips: Vec::new(),
        sections: SectionsState::One,
        track: None,
        sections_error: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready_with(dir.path(), HubState::default(), listing).await;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    // Live plays: the playhead is judged one tick ahead.
    let ahead = crate::rules::autotune::keys::lookahead_beats(Some(120.0));
    assert!(ahead > 0.0);
    deliver(&mut router, TIME, 100.00002 - ahead);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "past the rounded end"
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
    deliver(&mut router, TIME, 100.00006 - ahead);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    deliver(&mut router, TIME, 120.00002 - ahead);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "past the rounded start"
    );
    assert_eq!(router.autotune.state().song, None);
    deliver(&mut router, TIME, 120.00006 - ahead);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(router.autotune.state().song, Some(2));
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_singer_off_covers_both_tracks_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let page = attach(&mut router, 1);
    page.take();
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX1", "off": true}),
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::SingerOff), off(SwitchWhy::SingerOff)]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_off"], json!(["VOX1"]));
    assert_eq!(status(&mut router).autotune.singers_off, ["VOX1"]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "singer"),
        vec![
            json!({"step": "singer", "id": "VOX1", "off": true, "client": 1,
                    "peer": "10.0.0.1"})
        ]
    );
    // The pages are told: the singer off.
    let told: Vec<_> = page
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Autotune(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(told.last().is_some_and(|s| s.singers[0].off), "{told:?}");
    // Back on: both tune again (their keys are confirmed).
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX1", "off": false}),
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert!(status(&mut router).autotune.singers_off.is_empty());
    // An id no target carries and a value that is no `{id, off}` get the
    // error reply; nothing changes.
    for value in [
        json!({"id": "VOX9", "off": true}),
        json!({"id": "VOX1"}),
        json!(true),
    ] {
        set_hub(&mut router, HUB_AUTOTUNE_SINGER, value.clone());
        let replies = page.take().unwrap_or_default();
        assert!(
            replies
                .iter()
                .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
            "{value}: {replies:?}"
        );
    }
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn a_singer_off_is_kept_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: true,
        autotune_off: vec!["VOX1".into()],
    };
    let (mut router, mut live, _rx) = ready(dir.path(), state).await;
    // A saved AUT on is honoured at the start (#140 PR 3c).
    assert!(router.autotune.state().aut);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::SingerOff), off(SwitchWhy::SingerOff)]
    );
    let state = router.autotune.state();
    assert_eq!(state.singers.len(), 1);
    assert!(state.singers[0].off);
}

#[tokio::test]
async fn a_key_change_keeps_tuning_and_a_bad_read_back_drops_only_that_track() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A Max map holds the stream's slots now: its key write is refused.
    live.held.insert("k7".into());
    live.held.insert("s7".into());
    // The key clip to G passed: both stay on through the write.
    deliver(&mut router, TIME, 55.0);
    assert_eq!(decisions(&mut router), [on(), on()]);
    live.quiet_until_written(&mut router).await;
    // The stream's write refused: that track alone goes DRY; the PA's
    // read-back confirms G and it stays on.
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Mismatch)]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![select(7, false, "mismatch", "key")]
    );
    let singer = &router.autotune.state().singers[0];
    assert_eq!(
        (singer.stream, singer.pa, singer.key_ok),
        (TrackState::Dry, TrackState::Tune, false)
    );
    assert_eq!(singer.why.as_deref(), Some("stream Key refused, wrote G"));
}

impl Live {
    /// The next key write's answer handled (its read-back not yet).
    async fn quiet_until_written(&mut self, router: &mut Router) {
        let write = self.next("a key write", |c| {
            c.iter().all(|c| c["name"] == "set_prop") && !is_selector_write(c)
        });
        let mut echoes = Vec::new();
        let slots: Vec<Value> = write
            .commands
            .clone()
            .iter()
            .map(|c| self.slot(c, &mut echoes))
            .collect();
        write.answer(slots);
        let msg = next_of(&mut self.own, "the key write's answer", |m| {
            matches!(m, RouterMsg::AutotuneKeysWritten { .. })
        })
        .await;
        router.handle(msg);
    }
}

#[tokio::test]
async fn a_read_back_that_disagrees_drops_its_track_before_the_retry() {
    // The stream's slot reads another key back: DRY at once, the one retry
    // after it; a confirmed retry turns it on again.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    deliver(&mut router, TIME, 55.0);
    live.quiet_until_written(&mut router).await;
    let read = live.next("the read-back", |c| {
        c.iter().all(|c| c["name"] == "get_prop")
    });
    let slots: Vec<Value> = read
        .commands
        .iter()
        .map(|c| match ref_of(c) {
            // The stream's Key reads C, not G.
            Some("k7") => ok(json!(0.0)),
            Some(id) => ok(live.params[id].clone()),
            None => ok(live.params[c["args"]["prop"].as_str().unwrap()].clone()),
        })
        .collect();
    read.answer(slots);
    let msg = next_of(&mut live.own, "the read-back", |m| {
        matches!(m, RouterMsg::AutotuneKeysRead { .. })
    })
    .await;
    router.handle(msg);
    // DRY before the retry goes: the batch first, then the retry's write.
    let dry = live.next("the stream's DRY", is_selector_write);
    assert_eq!(dry.commands.len(), 1);
    assert_eq!(ref_of(&dry.commands[0]), Some("p7"));
    live.answer(&mut router, dry);
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Mismatch));
    live.settle(&mut router).await;
    // The retry confirmed G: on again.
    assert_eq!(live.written(), vec![batch(&[("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn another_writer_is_shown_and_dry_wins() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // AUT off: the stream's selector moved to its chain by hand: DRY again.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    let writer = |act: &str| {
        json!({"step": "selector_writer", "instance": "band", "target": "live_set tracks 7",
               "act": act})
    };
    assert_eq!(
        of(&steps(&rx), "selector_writer"),
        vec![writer("dry_again")]
    );
    // Again within 10 s: the hub stops writing it, and says so.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(of(&steps(&rx), "selector_writer"), vec![writer("gave_up")]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)],
        "AUT off comes first"
    );
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::AnotherWriter));
    let singer = &router.autotune.state().singers[0];
    assert_eq!(
        singer.why.as_deref(),
        Some("stream: another writer moves its selector")
    );
    // The PA tunes meanwhile; the stream's selector left as it is.
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0)])]);
    // The next song start resets the guard: DRY again, then on once its
    // key is confirmed.
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    let _ = live.written();
    deliver(&mut router, TIME, 125.0);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7", 0.0)]), batch(&[("p6", 0.0), ("p7", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_selector_another_writer_takes_to_dry_is_left_so() {
    // The hub never fights to turn autotune on (spec §7).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    by_hand(&mut router, &mut live, "p6", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "selector_writer"),
        vec![json!({"step": "selector_writer", "instance": "band",
                    "target": "live_set tracks 6", "act": "left_dry"})]
    );
    assert_eq!(
        of(&got, "select"),
        vec![select(6, false, "another_writer", "guard")]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::AnotherWriter), on()]
    );
    // A key change after it writes it nothing.
    deliver(&mut router, TIME, 55.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn the_hubs_own_echo_is_no_other_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // The echoes of the ON batch came back during the settle: no
    // `selector_writer` and nothing written again.
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn a_hold_writes_dry_first_through_the_last_ref() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A track-list change while both tune: DRY by `$ref` first, then held.
    router.autotune_tracks("band");
    let dry = live.next("the hold's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        vec![
            json!({"target": {"$ref": "p6", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 127.0}}),
            json!({"target": {"$ref": "p7", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 0.0}}),
        ]
    );
    let got = steps(&rx);
    let order: Vec<&str> = got.iter().map(|r| r["step"].as_str().unwrap()).collect();
    let write_at = order.iter().position(|s| *s == "write").unwrap();
    let held_at = order.iter().position(|s| *s == "held").unwrap();
    assert!(write_at < held_at, "{order:?}");
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "hold", "on": [],
                    "off": ["Vox 2", "#8"]})
        ]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Held), off(SwitchWhy::Held)]
    );
    // A stale ref answers StaleRef: nothing more is written (its target is
    // walked again).
    let stale = json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"});
    dry.answer(vec![ok(Value::Null), stale]);
    let msg = next_of(&mut live.own, "the hold's answer", |m| {
        matches!(m, RouterMsg::AutotuneWritten { .. })
    })
    .await;
    router.handle(msg);
    // While held, a stop, a start and another writer write nothing, also
    // on a target its walk confirmed meanwhile (the set is still held).
    transport(&mut router, false);
    transport(&mut router, true);
    by_hand(&mut router, &mut live, "p6", 0.0);
    find(&mut router, PA, rack(6));
    assert!(router.autotune.targets[PA].sure);
    by_hand(&mut router, &mut live, "p6", 127.0);
    by_hand(&mut router, &mut live, "p6", 0.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(of(&got, "selector_writer"), Vec::<Value>::new());
    assert_eq!(
        of(&got, "failed"),
        vec![
            json!({"step": "failed", "instance": "band", "target": "live_set tracks 7",
                    "problem": "stale ref"})
        ]
    );
}

#[tokio::test]
async fn a_write_that_meets_the_markers_unsettled_holds_the_set_dry_first() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A composition waits (the markers not settled): the stop's DRY goes
    // through the refs, and the set is held.
    router.markers_pending = Some(Vec::new());
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "markers"})]
    );
    assert_eq!(of(&got, "write")[0]["cause"], "hold");
    // Settled again: the release walks every target again; each is DRY and
    // written nothing until its walk confirms it (PR #147 review I4).
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    // Its own reason, the set no longer held (PR #147 re-review R6).
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Unconfirmed), off(SwitchWhy::Unconfirmed)]
    );
    // The walks answer: each tunes again once its own walk confirmed it.
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 0.0)]), batch(&[("p7", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_reconnect_decides_again_and_switches_on_only_after_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    // The script made its DRY writes; the hub forgot its refs and
    // decisions.
    assert_eq!(decisions(&mut router), [None, None]);
    live.params.insert("p6".into(), json!(127.0));
    live.params.insert("p7".into(), json!(0.0));
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "1.14.0".into(),
            live_version: "12".into(),
        }),
    });
    router.autotune_release("band", "settled");
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 12.0);
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p6", 127.0);
    deliver_macro(&mut router, "p7", 0.0);
    heard(&mut router, PA, (2.0 / 11.0, 0.05 / 28.0));
    heard(&mut router, STREAM, (2.0 / 11.0, 1.0 / 28.0));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "no map yet"
    );
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    let _ = steps(&rx);
    // The connect's map: the keys first, then the selectors.
    map_read(&mut router, 2, songs());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Mismatch), off(SwitchWhy::Mismatch)]
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let got = steps(&rx);
    let order: Vec<&str> = got
        .iter()
        .map(|r| r["step"].as_str().unwrap())
        .filter(|s| ["key_write", "readback", "write"].contains(s))
        .collect();
    assert_eq!(order, ["key_write", "readback", "readback", "write"]);
}

#[tokio::test]
async fn a_registration_mid_song_takes_no_selector_dry_and_one_dropping_its_write_does() {
    // The PR #147 ruling on I2: a selector stays on while both the list the
    // script kept and the newest one sent have its DRY write.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A third target found mid-song: the list goes again (sent) with the
    // two tuning selectors in it: nothing flicks.
    let third = "band|live_set tracks 8";
    router.handle(layout(
        2,
        vec![pa(), stream(), target(8, "Vox 9", (127, 0), Some("VOX9"))],
    ));
    find(&mut router, third, rack(8));
    assert_eq!(router.autotune.safe["band"].state, SafeState::Sent);
    assert_eq!(decisions(&mut router)[..2], [on(), on()]);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "no flick");
    assert_eq!(decisions(&mut router)[..2], [on(), on()]);
    assert_eq!(of(&steps(&rx), "select"), Vec::<Value>::new());
    // A registration Live did not answer: both lists still have the two
    // selectors (the script holds one of them): they stay on.
    router.handle(layout(3, vec![pa(), stream()]));
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    drop(registration);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router), [on(), on()]);
    // The stream rack replaced in place: its new ref is in no list the
    // script kept, so it is DRY until the new list is kept, then on again.
    let replaced = Found {
        path: rack(7).path,
        id: "p7b".into(),
        key: Ok(device(7)),
    };
    find(&mut router, STREAM, replaced);
    deliver_macro(&mut router, "p7b", 127.0);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7b", 0.0)]), batch(&[("p7b", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_tap_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let page = attach(&mut router, 1);
    page.take();
    for value in [json!(true), json!(false), json!("on")] {
        set_hub(&mut router, HUB_AUTOTUNE, value.clone());
        assert_eq!(
            steps(&rx),
            vec![json!({"step": "refused", "what": "tap",
                        "why": "the AUTOTUNE tap is gone; a tap opens the singers' panel",
                        "client": 1, "peer": "10.0.0.1"})],
            "{value}"
        );
        assert_eq!(
            page.take().unwrap_or_default(),
            vec![ServerMsg::Autotune(router.autotune.state())],
            "told again, no error: {value}"
        );
    }
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let now = status(&mut router).autotune;
    assert_eq!((now.taps, now.aut), (0, true));
}

#[tokio::test]
async fn no_write_when_live_already_shows_the_decision() {
    // One write per change: the same decision again, a playhead that stays
    // in the song, the macro found again write nothing.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    deliver(&mut router, TIME, 11.0);
    deliver(&mut router, TIME, 12.0);
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(of(&got, "write"), Vec::<Value>::new());
    assert_eq!(of(&got, "select"), Vec::<Value>::new());
    assert_eq!(status(&mut router).autotune.writes, 1, "tuning's one batch");
}

/// AUTOTUNE AUT saved on, Live playing at 10 in song 1 (D), both racks
/// found with their values heard and on DRY, the first registration Live
/// never answers (so nothing is kept), then the song map read and the keys
/// written and read back: every track stays DRY, `unarmed`.
async fn unarmed_in_a_song(dir: &std::path::Path) -> (Router, Live) {
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: true,
        autotune_off: Vec::new(),
    };
    let (mut router, mut live, _rx) = scripted_router(dir, state);
    assert!(router.autotune.state().aut, "a saved AUT on is honoured");
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    heard(&mut router, PA, (0.75, 0.0));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    by_hand(&mut router, &mut live, "p6", 127.0);
    by_hand(&mut router, &mut live, "p7", 0.0);
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    let first = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    drop(first);
    live.settle(&mut router).await;
    assert_ne!(router.autotune.safe["band"].state, SafeState::Kept);
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "no map yet: nothing on"
    );
    map_read(&mut router, 1, songs());
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "not kept: nothing on"
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Unarmed), off(SwitchWhy::Unarmed)]
    );
    (router, live)
}

#[test]
fn a_decision_of_the_hubs_own_has_its_words_and_every_other_none() {
    // PR #150 review I2.
    assert_eq!(
        waiting_words(SwitchWhy::Unarmed, Some("no such function: safe_writes")).as_deref(),
        Some("its fall back to DRY is not kept (no such function: safe_writes)")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Unarmed, None).as_deref(),
        Some("its fall back to DRY is not kept yet")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Held, None).as_deref(),
        Some("waits for Live's track list")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Unconfirmed, None).as_deref(),
        Some("waits for its rack to be found again")
    );
    for why in [
        SwitchWhy::Aut,
        SwitchWhy::Stop,
        SwitchWhy::Span,
        SwitchWhy::NoKey,
        SwitchWhy::SingerOff,
        SwitchWhy::Problem,
        SwitchWhy::AnotherWriter,
        SwitchWhy::Mismatch,
        SwitchWhy::Play,
    ] {
        assert_eq!(waiting_words(why, Some("x")), None, "{why:?}");
    }
}

#[tokio::test]
async fn a_singer_kept_dry_by_its_sets_fall_back_not_kept_says_why() {
    // PR #150 review I2: the pages never read `tuning` with every track DRY
    // for a reason of the hub's own; the singer shows it.
    let dir = tempfile::tempdir().unwrap();
    let (router, _live) = unarmed_in_a_song(dir.path()).await;
    let state = router.autotune.state();
    let vox = &state.singers[0];
    assert!(!vox.key_ok, "{vox:?}");
    let why = vox.why.as_deref().unwrap_or("");
    assert!(why.contains("fall back to DRY is not kept"), "{why}");
}

#[tokio::test]
async fn a_singers_key_is_confirmed_only_once_written_and_read_back() {
    // PR #150 review M4: the panel's ✓ is a read-back, never a key on its
    // way.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    heard(&mut router, PA, (0.75, 0.0));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    transport(&mut router, false);
    map_read(&mut router, 1, songs());
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    // The keys written, not yet read back: no singer is confirmed.
    let state = router.autotune.state();
    assert!(
        state.singers.iter().all(|s| s.key_ok && !s.confirmed),
        "{state:?}"
    );
    live.settle(&mut router).await;
    let state = router.autotune.state();
    assert!(state.singers[0].confirmed, "{state:?}");
    // Out of every song: no key in force, nothing confirmed.
    deliver(&mut router, TIME, 110.0);
    assert!(!router.autotune.state().singers[0].confirmed);
}

#[tokio::test]
async fn the_pages_are_told_the_singers_and_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    router.handle(layout(
        2,
        vec![pa(), stream(), target(9, "Vox 7", (0, 127), None)],
    ));
    let state = router.autotune.state();
    assert_eq!((state.song, state.key.as_deref()), (Some(1), Some("D")));
    assert_eq!(
        (state.map_keyed, state.map_changes, state.map_problems),
        (2, 1, 0)
    );
    assert!(!state.checking, "AUT off");
    assert_eq!(
        state.singers,
        vec![
            SingerState {
                id: "VOX1".into(),
                label: "Vox 2".into(),
                stream: TrackState::Dry,
                pa: TrackState::Dry,
                off: false,
                key_ok: true,
                why: None,
                confirmed: true,
            },
            SingerState {
                id: "#Vox 7".into(),
                label: "Vox 7".into(),
                stream: TrackState::None,
                pa: TrackState::Dry,
                off: false,
                key_ok: true,
                why: None,
                confirmed: false,
            },
        ]
    );
    assert_eq!(state.bar.as_deref(), Some("D"));
    // Tuning, the stream's slot then read by another key.
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let state = router.autotune.state();
    assert_eq!(
        (state.singers[0].stream, state.singers[0].pa),
        (TrackState::Tune, TrackState::Tune)
    );
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 7.0 / 11.0);
    let state = router.autotune.state();
    assert!(!state.singers[0].key_ok);
    assert_eq!(
        state.singers[0].why.as_deref(),
        Some("stream Key reads G, wrote D")
    );
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7", 0.0)])],
        "another writer's key: DRY"
    );
}

impl Live {
    /// Answers a call as Live would but holds its echoes (each selector's
    /// value after it), for the test to deliver when Live's tick would.
    fn answer_quiet(&mut self, asked: Asked) -> Vec<(String, f64)> {
        let commands = asked.commands.clone();
        if is_selector_write(&commands) {
            self.batches.push(
                commands
                    .iter()
                    .map(|c| {
                        (
                            ref_of(c).unwrap().to_string(),
                            c["args"]["value"].as_f64().unwrap(),
                        )
                    })
                    .collect(),
            );
        }
        let mut echoes = Vec::new();
        let slots: Vec<Value> = commands.iter().map(|c| self.slot(c, &mut echoes)).collect();
        asked.answer(slots);
        echoes
    }
}

/// The next selector write's answer handled.
async fn written_handled(router: &mut Router, live: &mut Live) {
    let msg = next_of(&mut live.own, "a selector write's answer", |m| {
        matches!(m, RouterMsg::AutotuneWritten { .. })
    })
    .await;
    router.handle(msg);
}

/// A selector batch's commands, each through its `$ref`.
fn selector_commands(writes: &[(&str, f64)]) -> Vec<Value> {
    writes
        .iter()
        .map(|(id, value)| {
            json!({"target": {"$ref": id, "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": value}})
        })
        .collect()
}

#[tokio::test]
async fn a_stop_before_the_switch_on_landed_writes_dry_at_once() {
    // PR #147 review I1 (a): Live stopped before it took the switch-on, so
    // it still shows DRY while the chain's write is on its way; DRY is
    // written at once, and Live takes both in one tick (both answers, then
    // one value per selector, the last).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    let switch_on = live.next("the switch-on", is_selector_write);
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        selector_commands(&[("p6", 127.0), ("p7", 0.0)])
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    let _ = live.answer_quiet(switch_on);
    let last = live.answer_quiet(dry);
    written_handled(&mut router, &mut live).await;
    written_handled(&mut router, &mut live).await;
    for (id, value) in &last {
        deliver_macro(&mut router, id, *value);
    }
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0), ("p7", 127.0)]),
            batch(&[("p6", 127.0), ("p7", 0.0)])
        ]
    );
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
}

#[tokio::test]
async fn a_start_before_the_stops_dry_landed_writes_the_chain_at_once() {
    // PR #147 review I1, the other direction: Live plays again before it
    // took the stop's DRY; each write in its own tick, one echo after its
    // answer and the other before it. No echo is another writer.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    transport(&mut router, true);
    let chain = live.next("the start's chain", is_selector_write);
    assert_eq!(
        chain.commands,
        selector_commands(&[("p6", 0.0), ("p7", 127.0)])
    );
    // The DRY's tick: its answer, then its values.
    let echoes = live.answer_quiet(dry);
    written_handled(&mut router, &mut live).await;
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    // The chain's tick: its values first, then its answer.
    let echoes = live.answer_quiet(chain);
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    written_handled(&mut router, &mut live).await;
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 127.0), ("p7", 0.0)]),
            batch(&[("p6", 0.0), ("p7", 127.0)])
        ]
    );
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert!(router.autotune.targets[PA].writing.is_empty());
}

#[tokio::test]
async fn a_layout_without_a_tuning_target_writes_it_dry_first_then_registers_without_it() {
    // PR #147 review I3: its last ref is the hub's only way back to DRY;
    // the script keeps its DRY write until that write answered.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.handle(layout(2, vec![pa()]));
    let dry = live.next("the dropped target's DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p7", 0.0)]));
    assert!(!router.autotune.targets.contains_key(STREAM));
    assert!(live.script.try_call().is_none(), "no registration yet");
    assert_eq!(router.autotune.safe["band"].seq, seq);
    assert_eq!(
        of(&steps(&rx), "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "layout", "on": [],
                    "off": ["#8"]})
        ]
    );
    live.answer(&mut router, dry);
    written_handled(&mut router, &mut live).await;
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    assert_eq!(
        registration.commands[0]["args"]["writes"],
        json!([{"target": {"$ref": "p6", "class": "DeviceParameter"}, "value": 127.0}])
    );
    assert_eq!(decisions(&mut router), [on()], "the PA still tunes");
}

#[tokio::test]
async fn a_tuning_target_found_missing_is_written_dry_first() {
    // PR #147 review I3: a definite answer that its rack is gone (renamed,
    // or a second one: `Twice`).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("2 racks named Latencies".into()));
    let dry = live.next("its DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p7", 0.0)]));
    assert!(router.autotune.targets[STREAM].found.is_none());
    assert!(live.script.try_call().is_none(), "no registration yet");
    live.answer(&mut router, dry);
    written_handled(&mut router, &mut live).await;
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    assert_eq!(
        registration.commands[0]["args"]["writes"],
        json!([{"target": {"$ref": "p6", "class": "DeviceParameter"}, "value": 127.0}])
    );
}

#[tokio::test]
async fn a_layout_that_swaps_two_tracks_pairs_no_new_spec_with_an_old_selector() {
    // PR #147 review I4: a track-list change moved the stream track to
    // index 6 and the PA track to 7, so each kept key takes the other's
    // spec; the walks find each rack at its new index.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    let seq = router.autotune.safe["band"].seq;
    let _ = steps(&rx);
    let stream_at_6 = target(6, "#7", (127, 0), Some("VOX1"));
    let pa_at_7 = target(7, "Vox 2", (0, 127), Some("VOX1"));
    router.handle(layout(2, vec![stream_at_6, pa_at_7]));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "nothing by the old refs"
    );
    assert_eq!(router.autotune.safe["band"].seq, seq, "nothing registered");
    assert_eq!(decisions(&mut router), [None, None]);
    // The walks: the stream rack (p7) at 6, the PA rack (p6) at 7.
    let at = |index: u32, rack_of: u32| Found {
        path: format!("live_set tracks {index} devices 0 parameters 1"),
        id: format!("p{rack_of}"),
        key: Ok(device(rack_of)),
    };
    find(&mut router, PA, at(6, 7));
    find(&mut router, STREAM, at(7, 6));
    deliver_macro(&mut router, "p7", 0.0);
    deliver_macro(&mut router, "p6", 127.0);
    live.settle(&mut router).await;
    // Each ref with its own spec's DRY value; both DRY until their keys.
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p7".to_string(), 0), ("p6".to_string(), 127)]
    );
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert!(
        of(&steps(&rx), "select")
            .iter()
            .all(|r| r["on"] == json!(false))
    );
}

#[tokio::test]
async fn a_polarity_change_walks_before_any_write_or_registration() {
    // PR #147 review I4: the stream target's marker now says PA (on 0,
    // off 127) at the same track: its old ref's DRY value is the other
    // polarity's.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.handle(layout(
        2,
        vec![pa(), target(7, "#8", (0, 127), Some("VOX1"))],
    ));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "no write");
    assert_eq!(router.autotune.safe["band"].seq, seq, "no registration");
    assert!(router.autotune.targets[STREAM].found.is_none());
    assert_eq!(decisions(&mut router)[0], on(), "the PA still tunes");
    // Its walk: the same rack; Live at 127, its DRY now.
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p6".to_string(), 127), ("p7".to_string(), 127)]
    );
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router)[0], on());
}

#[tokio::test]
async fn a_selector_write_live_refuses_gets_one_retry_then_the_hub_stops() {
    // PR #147 review I5, ruled: a Max map holds the stream's selector.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    live.held.insert("p7".into());
    let walk = router.autotune.targets[STREAM].walk;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0), ("p7", 127.0)]),
            batch(&[("p7", 127.0)])
        ]
    );
    let failed = of(&steps(&rx), "failed");
    assert_eq!(failed.len(), 2, "{failed:?}");
    assert!(failed.iter().all(|r| r["problem"] == DISABLED));
    assert_eq!(router.autotune.targets[STREAM].walk, walk, "not walked");
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Problem)]);
    assert_eq!(
        router.autotune.state().singers[0].why.as_deref(),
        Some(format!("stream: Live refuses its selector: {DISABLED}").as_str())
    );
    // A key change's brief `mismatch` while its key is written is no new
    // decision (PR #147 re-review R7): the refused chain is not asked again.
    deliver(&mut router, TIME, 55.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Problem)]);
    // A stop is a new decision: the refusal ends (Live shows DRY there).
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    // The next start asks once more, and its retry, then stops again.
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0), ("p7", 127.0)]),
            batch(&[("p7", 127.0)])
        ]
    );
}

#[tokio::test]
async fn the_markers_settling_decides_again_without_a_hold() {
    // PR #147 review N2: judged held while a composition waits, with
    // nothing to write, no hold opens; their settling decides again.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    router.markers_pending = Some(Vec::new());
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Held), off(SwitchWhy::Held)]
    );
    assert_eq!(of(&steps(&rx), "held"), Vec::<Value>::new(), "no hold");
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_missing_target_is_a_problem_never_a_key_being_checked() {
    // PR #147 review N3.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    live.settle(&mut router).await;
    let state = router.autotune.state();
    assert!(!state.checking, "a missing rack is no key being checked");
    let vox1 = &state.singers[0];
    assert!(!vox1.key_ok);
    assert_eq!(vox1.why.as_deref(), Some("stream: no rack named Latencies"));
    assert_eq!(vox1.stream, TrackState::Dry);
    // A key device missing: a problem too, never checking.
    let mut keyless = rack(7);
    keyless.key = Err(KeyMissing::NoChain);
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Ok(keyless));
    let state = router.autotune.state();
    assert!(!state.checking);
    assert!(!state.singers[0].key_ok);
}

#[tokio::test]
async fn a_dry_decision_writes_a_stream_selector_anywhere_in_its_chains_zone() {
    // PR #147 review N4, ruled (DRY wins): 64 lies in a stream rack's
    // autotune zone (58 to 127 on the PC); the pages' window reads it off.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p7", 64.0);
    assert_eq!(router.autotune.state().targets[0].state, TargetState::Off);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
}

#[tokio::test]
async fn a_keyless_song_start_ends_the_guards_give_up() {
    // PR #147 review N5: a keyless song entered from out of every song is
    // a song start too.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // Another writer twice within 10 s (AUT off): the hub stops.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    let _ = live.written();
    assert_eq!(
        of(&steps(&rx), "selector_writer").last().unwrap()["act"],
        "gave_up"
    );
    // Out of every song, then into song 3 (no key): DRY again.
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "a span");
    deliver(&mut router, TIME, 215.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
}

#[tokio::test]
async fn a_singers_manual_off_is_cleared_for_an_id_no_target_carries() {
    // PR #147 review N6: an id a marker edit left behind.
    let dir = tempfile::tempdir().unwrap();
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: false,
        autotune_off: vec!["VOX1".into(), "VOX9".into()],
    };
    let (mut router, _live, rx) = ready(dir.path(), state).await;
    let page = attach(&mut router, 1);
    page.take();
    // Off again while off: no target carries it, the error reply.
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": true}),
    );
    let replies = page.take().unwrap_or_default();
    assert!(
        replies
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
        "{replies:?}"
    );
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": false}),
    );
    assert_eq!(
        of(&steps(&rx), "singer"),
        vec![
            json!({"step": "singer", "id": "VOX9", "off": false, "client": 1,
                    "peer": "10.0.0.1"})
        ]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_off"], json!(["VOX1"]));
    // Off again: no target carries it, the error reply.
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": true}),
    );
    let replies = page.take().unwrap_or_default();
    assert!(
        replies
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
        "{replies:?}"
    );
}

#[tokio::test]
async fn a_target_naming_no_singer_has_no_manual_off_and_never_tunes() {
    // PR #147 review N6: a placeholder label (`#10`) names a track index,
    // never a singer.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    let lone = "band|live_set tracks 9";
    router.handle(layout(
        2,
        vec![pa(), stream(), target(9, "#10", (0, 127), None)],
    ));
    find(&mut router, lone, rack(9));
    deliver_macro(&mut router, "p9", 127.0);
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(decisions(&mut router)[2], off(SwitchWhy::Problem));
    let state = router.autotune.state();
    let lone_singer = &state.singers[1];
    assert_eq!(lone_singer.id, "");
    assert!(!lone_singer.off);
    assert_eq!(
        lone_singer.why.as_deref(),
        Some(format!("PA: {}", super::NO_SINGER).as_str())
    );
    // No id to switch off by hand: `##10` is no singer.
    let page = attach(&mut router, 1);
    page.take();
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "##10", "off": true}),
    );
    assert!(
        page.take()
            .unwrap_or_default()
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { .. }))
    );
}

#[tokio::test]
async fn a_second_track_list_change_within_a_hold_unconfirms_a_walks_answer_again() {
    // PR #147 re-review R1: a walk answered between two track-list changes
    // in one hold found the rack the first change put at its index; the
    // markers' layout then keeps that index's polarity.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    find(
        &mut router,
        PA,
        Found {
            id: "p8".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    assert!(router.autotune.targets[PA].sure);
    // The second change, within the same hold.
    router.autotune_tracks("band");
    assert!(!router.autotune.targets[PA].sure);
    assert!(!router.autotune.targets[STREAM].sure);
    // The layout ends the hold: nothing by the walk's ref, nothing
    // registered with it, before the newest walks answer.
    router.handle(layout(
        2,
        vec![target(6, "Vox 5", (0, 127), Some("VOX1")), stream()],
    ));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(router.autotune.safe["band"].seq, seq, "nothing registered");
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Unconfirmed), off(SwitchWhy::Unconfirmed)]
    );
    // The newest walks answer: registered with their refs.
    find(
        &mut router,
        PA,
        Found {
            id: "p9".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p9".to_string(), 127), ("p7".to_string(), 0)]
    );
    assert!(live.written().iter().flatten().all(|(id, _)| id != "p8"));
}

#[tokio::test]
async fn a_target_let_go_while_the_markers_are_unsettled_is_held_dry_first() {
    // PR #147 re-review R2: no hold open yet, a composition waiting.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.markers_pending = Some(Vec::new());
    router.handle(layout(2, vec![pa()]));
    let dry = live.next("the hold's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        selector_commands(&[("p6", 127.0), ("p7", 0.0)])
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "markers"})]
    );
    assert_eq!(of(&got, "write")[0]["cause"], "hold");
    assert!(!router.autotune.targets.contains_key(STREAM));
    live.answer(&mut router, dry);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].seq, seq, "held: nothing yet");
    // Settled: once the PA's walk confirmed it, registered without the
    // stream's selector.
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    find(&mut router, PA, rack(6));
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p6".to_string(), 127)]
    );
}

#[tokio::test]
async fn a_stop_after_a_switch_on_live_did_not_answer_writes_dry_at_once() {
    // PR #147 re-review R3: Live may still take the switch-on.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    let switch_on = live.next("the switch-on", is_selector_write);
    let walk = router.autotune.targets[STREAM].walk;
    drop(switch_on);
    written_handled(&mut router, &mut live).await;
    let target = &router.autotune.targets[STREAM];
    assert_eq!(target.writing_now(), Some(true), "still on its way");
    assert!(target.sure);
    assert_eq!(target.walk, walk, "not walked");
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        selector_commands(&[("p6", 127.0), ("p7", 0.0)])
    );
    assert_eq!(of(&steps(&rx), "failed").len(), 2);
}

#[tokio::test]
async fn a_list_sent_over_one_still_on_its_way_narrows_what_the_script_may_hold() {
    // PR #147 re-review R4: the script may take either list.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    // The stream's rack gone for good (DRY: nothing to write first): a list
    // without it goes, unanswered yet.
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    let without = live.next("the list without it", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    // Found again with the same ref: the list with it goes after.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&[("p6".to_string(), 127)][..])
    );
    aut_on(&mut router);
    transport(&mut router, true);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
    drop(without);
}

#[tokio::test]
async fn a_list_live_did_not_answer_narrows_what_the_script_may_hold() {
    // PR #147 re-review R15: a registration Live did not answer
    // (`Safe.lost`) may still become the script's list, so the list after
    // it keeps only the pairs both share; one the script refused keeps the
    // kept list whole.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    let both = [("p6".to_string(), 127), ("p7".to_string(), 0)];
    let without_stream = |router: &mut Router| {
        // The stream's rack gone for good (DRY: nothing to write first).
        let walk = router.autotune.targets[STREAM].walk;
        router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    };
    let is_list = |c: &[Value]| c.len() == 1 && c[0]["name"] == SAFE_WRITES;
    // Refused: the list without the stream goes and the script refuses it.
    without_stream(&mut router);
    let refused = live.next("the list without it", is_list);
    refused.answer(vec![json!({"ok": false,
        "error": "no such function: safe_writes", "errorType": "OpError"})]);
    live.settle(&mut router).await;
    let safe = &router.autotune.safe["band"];
    assert_eq!(safe.state, SafeState::Failed);
    assert!(!safe.lost, "refused: the script did not take it");
    // Found again: the list with it goes after, the kept list whole.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&both[..]),
        "kept whole after a refusal"
    );
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    // Not answered: the list without the stream goes, Live never answers.
    without_stream(&mut router);
    let lost = live.next("the list without it, again", is_list);
    drop(lost);
    live.settle(&mut router).await;
    let safe = &router.autotune.safe["band"];
    assert_eq!(safe.state, SafeState::Failed);
    assert!(safe.lost, "Live did not answer it");
    assert_eq!(safe.kept.as_deref(), Some(&both[..]));
    // Found again: the script may hold the list without it, so only the
    // pair both share stays kept and the stream is not armed.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&both[..1])
    );
    aut_on(&mut router);
    transport(&mut router, true);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
}

#[tokio::test]
async fn a_walk_that_gives_up_unconfirmed_leaves_its_target_out_of_the_registered_list() {
    // PR #147 re-review R8: two track-list changes in one hold; the PA's
    // walk after the second gives up, keeping the ref the first change's
    // walk found, which an older layout named at its index.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    find(
        &mut router,
        PA,
        Found {
            id: "p8".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    router.autotune_tracks("band");
    find(&mut router, STREAM, rack(7));
    let walk = router.autotune.targets[PA].walk;
    for _ in 0..=super::super::super::retries::CONNECT_RETRIES {
        router.autotune_walk_failed(PA, walk, "no result within 3 s".into());
    }
    let pa = &router.autotune.targets[PA];
    assert!(pa.problem.as_deref().is_some_and(|p| p.contains("gave up")));
    assert_eq!(pa.found.as_ref().map(|f| f.id.as_str()), Some("p8"));
    assert!(!pa.sure);
    // The layout ends the hold: the stream registers, never the PA by
    // `p8`.
    router.handle(layout(
        2,
        vec![target(6, "Vox 5", (0, 127), Some("VOX1")), stream()],
    ));
    let safe = &router.autotune.safe["band"];
    assert!(safe.seq > seq, "registered");
    assert_eq!(safe.list, vec![("p7".to_string(), 0)]);
    live.settle(&mut router).await;
    assert!(live.written().iter().flatten().all(|(id, _)| id != "p8"));
}

#[tokio::test]
async fn a_stop_after_a_lost_write_and_an_answered_one_writes_dry_at_once() {
    // PR #147 re-review R9: the stop's DRY goes unanswered, the start's
    // chain is answered, and its value comes while the markers are
    // unsettled (no decision then); the next stop writes DRY.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    let lost = live.next("the stop's DRY", is_selector_write);
    drop(lost);
    written_handled(&mut router, &mut live).await;
    assert_eq!(router.autotune.targets[PA].writing_now(), Some(false));
    transport(&mut router, true);
    let chain = live.next("the start's chain", is_selector_write);
    assert_eq!(
        chain.commands,
        selector_commands(&[("p6", 0.0), ("p7", 127.0)])
    );
    let echoes = live.answer_quiet(chain);
    written_handled(&mut router, &mut live).await;
    router.markers_pending = Some(Vec::new());
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    router.markers_pending = None;
    transport(&mut router, false);
    let dry = live.next("the second stop's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        selector_commands(&[("p6", 127.0), ("p7", 0.0)])
    );
}
