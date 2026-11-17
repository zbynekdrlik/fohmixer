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
        lufs: Vec::new(),
        claps: None,
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
    /// A script before 1.13.0: every registration refused (`no such
    /// function: safe_writes`), so nothing is ever kept.
    old_script: bool,
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
            let slot = if self.old_script {
                json!({"ok": false, "error": "no such function: safe_writes",
                       "errorType": "OpError"})
            } else if gone > 0 {
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
        old_script: false,
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

/// The band's spread turn now (its timer's message handed over at once;
/// the timer then finds it stale): a test on tokio's real clock can still
/// walk a spread.
fn next_turn(router: &mut Router) {
    let generation = router.autotune.spreads["band"].generation;
    router.autotune_spread_turn("band", generation);
}

/// [`ready`], then AUTOTUNE AUT on and Live playing: both tracks tune, the
/// switch-on spread one selector per [`SPREAD`] (#190, 2026-11-17), its
/// second turn handed over at once ([`next_turn`]: not every caller
/// pauses the clock).
async fn tuning(dir: &std::path::Path) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    let (mut router, mut live, rx) = ready(dir, HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0)])]);
    next_turn(&mut router);
    live.settle(&mut router).await;
    assert!(router.autotune.spreads.is_empty());
    assert_eq!(live.written(), vec![batch(&[("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
    let _ = steps(&rx);
    (router, live, rx)
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

/// AUTOTUNE AUT saved on, Live playing at 10 in song 1 (D), both racks
/// found with their values heard and on DRY, a script that refuses every
/// registration (one before 1.13.0: nothing is ever kept), then the song
/// map read and the keys written and read back: every track stays DRY,
/// `unarmed`. A Max map holds the `held` key parameters (their writes
/// refused).
async fn unarmed_in_a_song(dir: &std::path::Path, held: &[&str]) -> (Router, Live) {
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: true,
        autotune_off: Vec::new(),
    };
    let (mut router, mut live, _rx) = scripted_router(dir, state);
    live.old_script = true;
    live.held.extend(held.iter().map(|id| id.to_string()));
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
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
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

/// The stop's spread of its DRY writes (#140, the DECIDED comment of
/// 2026-11-09): one selector per this many ms.
const SPREAD: std::time::Duration = std::time::Duration::from_millis(40);

/// `by` of Live's time passes; every message the hub's own timers or Live's
/// answers sent meanwhile is handled (a walk's is dropped: each test
/// answers the walks itself), every call answered.
async fn pass(router: &mut Router, live: &mut Live, by: std::time::Duration) {
    tokio::time::advance(by).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    while let Ok(msg) = live.own.try_recv() {
        if !matches!(
            msg,
            RouterMsg::AutotuneFound { .. }
                | RouterMsg::AutotuneWalkFailed { .. }
                | RouterMsg::AutotuneWalkRetry { .. }
        ) {
            router.handle(msg);
        }
    }
    live.settle(router).await;
}

mod guard;
mod hold;
mod pages;
mod refusal;
mod transport;

/// A play start's wait of its selector-on writes (#190).
#[path = "../engage_tests.rs"]
mod engage;
