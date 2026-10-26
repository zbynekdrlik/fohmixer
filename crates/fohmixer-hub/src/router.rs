//! The hub's router: one task that owns the subscription table, the client
//! outboxes, the setters and the STAGE AUT rule (S3 design note §3, §4, §6).
//! It follows every instance's transport from its start (#104, #130, D18:
//! `RULES_CLIENT`), logs each change (`transport`) and hands it to the
//! rules: STAGE AUT here, REVERB in `router/reverb.rs` (#130, D18: its own
//! subscriber `REVERB_CLIENT`, its writes through the setters, its wakes).
//!
//! Everything reaches it as a message, in order: the instances' events (a
//! subscription's result stays ordered with the value pushes of the same
//! connection), the clients' requests, layout changes. It never waits: it
//! writes into outboxes and queues requests to the instances' tasks. It also
//! runs the layout's check for unresolved names (spec §2.5 D4) when a layout
//! is accepted and when an instance connects.
//!
//! The clients' writes (#43, `set`) go through one setter per instance
//! (`setter.rs`, driven by `writes.rs`): one batch in flight, the latest
//! want per key, acks per client. Every hop is an event-log record
//! (`events.rs`); Live's health goes to every client as `link`.
//!
//! The Stream Deck (#52, `deck.rs`): the router owns its state too; the
//! Companion task's events come as messages, and `router/deck.rs` carries
//! the decisions out.
//!
//! The Tuner markers (#68, `router/markers.rs`): the router's marker keeper
//! finds them; `router/markers_io.rs` hands each new set to the layout
//! store, whose new composition goes out as a layout change.
//!
//! The Pro-Q 4 screen (#71 PR E, `eq.rs`): the router owns who holds which
//! editor; `router/eq.rs` carries the state's acts out through the window
//! worker (`plugwin.rs`) and the instances.
//!
//! AbleSet's page (#90 B3, `router/ableset.rs`): every page is told of it
//! at attach and at each change of the hub's link to AbleSet.
//!
//! AUTOTUNE (#104, D19, `router/autotune.rs`): the `+AUTOTUNE` tracks'
//! `Latencies` racks found by a walk, their first macro written through its
//! `$ref` by the rule's transport, the pages' flag and taps (its own
//! subscriber `AUTOTUNE_CLIENT`).
//!
//! The song map (#140, `router/songmap.rs`): every Live set's locators and
//! Sections key clips read into a song map at a connect, a list change and
//! every transport change, served at `GET /api/songmap` (its own subscriber
//! `SONGMAP_CLIENT`; it writes nothing to Live).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use fohmixer_proto::client::{
    AutotuneStatus, FaderLinkStatus, HUB_STAGE_AUT, ReverbStatus, ServerMsg, SetItem,
    SongMapReport, SongMapStatus, StageAutStatus, Unresolved, ValueItem,
};
use fohmixer_proto::layout::{AutotuneTarget, Binding, Layout, LinkId, RuleFader};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::events::EventLog;
use crate::live::client::{Beat, LiveError, LiveEvent, LiveHandle};
use crate::live::names::NameCheck;
use crate::live::subs::{Cached, ClientId, Outgoing, Subs};
use crate::outbox::Outbox;
use crate::rules::links::FaderLinks;
use crate::rules::{HubState, StageAut};
use crate::setter::Setter;

#[path = "router/writes.rs"]
mod writes;

mod ableset;
#[path = "router/autotune.rs"]
mod autotune;
#[path = "router/deck.rs"]
mod deck;
mod eq;
#[path = "router/links.rs"]
mod links;
mod markers;
#[path = "router/markers_io.rs"]
mod markers_io;
mod retries;
#[path = "router/reverb.rs"]
mod reverb;
#[path = "router/songmap.rs"]
mod songmap;
mod unfold;

pub use songmap::Missing as SongMapMissing;

/// The router's own subscriber for the rules (#130, D18): every instance's
/// transport, which STAGE AUT and REVERB AUT follow and the event log
/// records (clients start at 1).
pub const RULES_CLIENT: ClientId = 0;
/// The router's subscriber that keeps the strips' groups unfolded (#58;
/// client numbers count up from 1 and never reach it).
pub const UNFOLD_CLIENT: ClientId = ClientId::MAX;
/// The router's subscriber that follows the Tuner markers (#68).
pub const MARKERS_CLIENT: ClientId = ClientId::MAX - 1;

/// A message to the router.
pub enum RouterMsg {
    /// An event of an instance's task.
    Live {
        instance: String,
        event: LiveEvent,
    },
    /// A client connected: its outbox, its peer's address and whether its
    /// socket came over the LAN (the Access check's class,
    /// `ws::Opener::is_lan`; AbleSet's page tells it so, #90).
    Attach {
        client: ClientId,
        outbox: Arc<Outbox>,
        peer: String,
        lan: bool,
    },
    Sub {
        client: ClientId,
        instance: String,
        target: String,
        prop: String,
        display: bool,
    },
    Unsub {
        client: ClientId,
        sub: String,
    },
    SetHub {
        client: ClientId,
        key: String,
        value: Value,
    },
    /// A client's write (#43, `set`), with when it reached the hub (hub UTC
    /// ms) and the page clock's offset as of its socket's last ping.
    Set {
        client: ClientId,
        instance: String,
        target: String,
        prop: String,
        value: Value,
        seq: u64,
        t: f64,
        is_final: bool,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// A client's writes Live applies together (#127, `sets`: a linked
    /// mute's tracks), each taken as a `set` before any batch is written;
    /// at most `MAX_SETS`, the rest acked failed (`writes::on_sets`).
    Sets {
        client: ClientId,
        sets: Vec<SetItem>,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// The result of an instance's batch of writes (`Err`: why it has
    /// none: offline, a timeout, refused).
    Applied {
        instance: String,
        batch: u64,
        outcome: Result<Vec<Value>, String>,
    },
    /// The answer to a read of the unfold keeper (#58).
    UnfoldRead {
        instance: String,
        seq: u64,
        outcome: Result<Vec<Value>, String>,
    },
    /// A failed read of the unfold keeper is due again (#58).
    UnfoldRetry {
        instance: String,
    },
    /// The answer to a batch of a read of the marker keeper (#68).
    MarkersRead {
        instance: String,
        seq: u64,
        step: markers::Step,
        outcome: Result<Vec<Value>, String>,
    },
    /// A failed read of the marker keeper is due again (#68).
    MarkersRetry {
        instance: String,
    },
    /// The markers' change `generation` has had its quiet time (#68): served
    /// when no newer change came.
    MarkersSettle {
        generation: u64,
    },
    /// A client's connection ended.
    Detach {
        client: ClientId,
    },
    /// A new layout is served: its revision, its STAGE AUT binding and its
    /// check targets per instance (for the unresolved-names check: the
    /// bindings, `live::names::layout_targets`), its strips' tracks and its
    /// reverb faders.
    Layout {
        rev: u64,
        stage: Option<Binding>,
        targets: BTreeMap<String, Vec<String>>,
        /// The strips' tracks (`Layout::strip_tracks`, #58).
        strips: Vec<(String, String)>,
        /// The REVERB rule's faders (`Layout::reverb_faders`, #130).
        reverb: Vec<RuleFader>,
        /// Its fader links' strip leaders (`Layout::fader_leaders`, #130).
        fader_leaders: Vec<LinkId>,
        /// The AUTOTUNE rule's tracks (`Layout::autotune_targets`, #104).
        autotune: Vec<AutotuneTarget>,
    },
    /// The router's part of `/api/status`.
    Status {
        reply: oneshot::Sender<RouterStatus>,
    },
    /// `/api/markers/migration` (#68 PR C): the planned tracks as the
    /// marker keeper finds them; with `apply`, the ready ones' Tuners are
    /// renamed.
    Migration {
        apply: bool,
        reply: oneshot::Sender<fohmixer_proto::markers::migrate::MigrationStatus>,
    },
    /// Each instance's lists read afresh for a `Migration` (the answers of
    /// `get_prop tracks` and `get_prop return_tracks`, or why not).
    MigrationLists {
        apply: bool,
        reply: oneshot::Sender<fohmixer_proto::markers::migrate::MigrationStatus>,
        lists: Vec<(String, Result<Vec<Value>, String>)>,
    },
    /// A page opened or closed the Stream Deck tab (#52).
    DeckView {
        client: ClientId,
        on: bool,
    },
    /// A page's Stream Deck press (#52), with its arrival (hub UTC ms) and
    /// the page clock's offset as of its socket's last ping.
    DeckPress {
        client: ClientId,
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        hold_ms: Option<f64>,
        why: Option<String>,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// Something came from a client (#52: a holding page silent for 2 s is
    /// released).
    Heard {
        client: ClientId,
    },
    /// An event of the Companion task (#52).
    Deck {
        event: crate::companion::CompanionEvent,
    },
    /// The Stream Deck's and the Pro-Q contacts' clock (#52, #71 PR E:
    /// every `crate::deck::TICK` with `[companion]` or the Pro-Q screen).
    Tick,
    /// A page asks for the Pro-Q 4 instances of a strip's track (#71 PR E).
    EqList {
        client: ClientId,
        binding: Binding,
    },
    /// The walk of a strip's track ended (#71 PR E).
    EqListed {
        client: ClientId,
        binding: Binding,
        outcome: Result<Vec<crate::eq::walk::Found>, String>,
    },
    /// A page opens a Pro-Q 4 editor (#71 PR E), with its picture area
    /// (PR G).
    EqOpen {
        client: ClientId,
        instance: String,
        path: String,
        area: Option<fohmixer_proto::eq::Area>,
    },
    /// A page's picture area changed (#71 PR G).
    EqArea {
        client: ClientId,
        area: fohmixer_proto::eq::Area,
    },
    /// An editor's open sequence ended: its picture's size, or why not, and
    /// the device's `$ref` the open read (its close turns it off through
    /// it), of the instance's connection the open started on; how the
    /// editor's resize to its page's area settled (PR G).
    EqOpened {
        key: crate::eq::EditorKey,
        session: u32,
        outcome: Result<(u32, u32), String>,
        reference: Option<Value>,
        connection: u32,
        resized: Option<crate::plugwin::Resized>,
    },
    /// An open editor's resize to its page's new area settled (#71 PR G;
    /// none: nothing was asked).
    EqResized {
        key: crate::eq::EditorKey,
        session: u32,
        resized: Option<crate::plugwin::Resized>,
    },
    /// A numbered finger on a page's editor (PR J).
    EqInput {
        client: ClientId,
        input: crate::eq::Input,
    },
    /// The part of its editor's picture a page shows (PR L).
    EqView {
        client: ClientId,
        part: Option<fohmixer_proto::eq::Part>,
    },
    /// A page leaves its editor.
    EqClose {
        client: ClientId,
    },
    /// A page's undo of its open editor, its input `n` (#71 PR P2a).
    EqUndo {
        client: ClientId,
        n: u32,
    },
    /// The window worker answered a page's undo (#71 PR P2a).
    EqUndone {
        undid: crate::eq::Undid,
    },
    /// An editor's close sequence ended: what it left in Live.
    EqClosed {
        key: crate::eq::EditorKey,
        session: u32,
        outcome: crate::eq::close::Shut,
    },
    /// An event of the window worker (#71 PR E).
    EqWorker {
        event: crate::plugwin::PlugwinEvent,
    },
    /// The hub's link to AbleSet went up or down (#90 B3).
    AbleSetLink {
        online: bool,
    },
    /// REVERB's wake `generation` came (#130, D18: the stop's wait ended or
    /// a glide's next step is due); a newer armed wake makes it stale.
    ReverbWake {
        generation: u64,
    },
    /// An AUTOTUNE target's walk `walk` ended (#104): its rack's first
    /// macro, or why not in words (a newer walk makes it void).
    AutotuneFound {
        key: String,
        walk: u64,
        outcome: Result<crate::rules::autotune::find::Found, String>,
    },
    /// An AUTOTUNE target's walk `walk` failed (#140): a read Live did not
    /// answer (offline, a timeout) or refused; worth another walk.
    AutotuneWalkFailed {
        key: String,
        walk: u64,
        why: String,
    },
    /// A failed walk `walk`'s retry is due (#140); void once the target was
    /// walked since (a trigger, a disconnect).
    AutotuneWalkRetry {
        key: String,
        walk: u64,
    },
    /// The answer to AUTOTUNE's selector write `write` of `instance` (#104,
    /// #140 PR 3b): each target's key, the `$ref` written and the value's
    /// side (`true`: the autotune chain), in the slots' order (`Err`: why
    /// none was written: offline, a timeout, refused).
    AutotuneWritten {
        instance: String,
        write: u64,
        selectors: Vec<(String, String, bool)>,
        outcome: Result<Vec<Value>, String>,
    },
    /// The answer to AUTOTUNE's registration `seq` of `instance`'s DRY
    /// writes with its script, `targets` of them (#140 PR 3a): the one
    /// command's slot, or why there is none.
    AutotuneSafe {
        instance: String,
        seq: u64,
        targets: usize,
        outcome: Result<Vec<Value>, String>,
    },
    /// The script's answer to the probe of `instance`'s refused
    /// registration `seq` (#140 PR 3b, PR #146 review N1): each listed
    /// ref's `value` (`ids` in that order), or why there is none.
    AutotuneSafeProbed {
        instance: String,
        seq: u64,
        ids: Vec<String>,
        outcome: Result<Vec<Value>, String>,
    },
    /// The retry of the probe of `instance`'s refused registration `seq` is
    /// due (PR #147 review N1: a probe Live did not answer); void once the
    /// set registered since or went offline.
    AutotuneSafeProbeRetry {
        instance: String,
        seq: u64,
    },
    /// A beat (PR #146 review I1, every `live::client::ROUTER_BEAT_MS`):
    /// nothing to do; the loop's turn stamps the router's `Beat`.
    Beat,
    /// The retry of `instance`'s failed registration `seq` is due (#140 PR
    /// 3a, PR #146 review I2); void once the set registered since or went
    /// offline.
    AutotuneSafeRetry {
        instance: String,
        seq: u64,
    },
    /// The answer to key write `write` of `instance`'s targets `keys` (#140
    /// PR 2b; their slots in that order, then Live's bar's when it held it)
    /// and its round trip; a newer write of a target makes it void there.
    AutotuneKeysWritten {
        instance: String,
        write: u64,
        keys: Vec<String>,
        outcome: Result<Vec<Value>, String>,
        rtt_ms: f64,
    },
    /// The read-back of key write `write` of `instance`'s targets `keys`
    /// (#140 PR 2b; as [`RouterMsg::AutotuneKeysWritten`]).
    AutotuneKeysRead {
        instance: String,
        write: u64,
        keys: Vec<String>,
        outcome: Result<Vec<Value>, String>,
    },
    /// A read of `instance`'s song map ended (#140): what it found, or why
    /// not, and how long it took (a newer read or a disconnect makes it
    /// void).
    SongMapRead {
        instance: String,
        read_no: u64,
        result: Result<songmap::Listing, String>,
        ms: u64,
    },
    /// `GET /api/songmap` (#140): `instance`'s newest song map.
    SongMap {
        instance: String,
        reply: oneshot::Sender<Result<SongMapReport, SongMapMissing>>,
    },
    /// A failed song map read of `instance` is due again (#140 review M3):
    /// `read_no` is the failed read's; void unless it is still the set's
    /// retry.
    SongMapRetry {
        instance: String,
        read_no: u64,
    },
    /// The hub stops: the Stream Deck's held keys released and its task
    /// told to stop (#52), then every client closed.
    Stop,
}

/// The router's part of `/api/status`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouterStatus {
    pub subscriptions: BTreeMap<String, usize>,
    /// The groups the unfold keeper holds, per instance (#58).
    pub unfolded: BTreeMap<String, Vec<String>>,
    pub listeners: BTreeMap<String, usize>,
    /// Each instance's transport as the router last heard it (#104, D18;
    /// none before the first value, and again after a disconnect).
    pub playing: BTreeMap<String, Option<bool>>,
    pub stage_aut: StageAutStatus,
    /// REVERB (#130, D18).
    pub reverb: ReverbStatus,
    /// Each strip leader's fader link switch (#130).
    pub fader_links: Vec<FaderLinkStatus>,
    /// AUTOTUNE (#104, D19).
    pub autotune: AutotuneStatus,
    /// Each Live set's song map, once read (#140).
    pub songmap: Vec<SongMapStatus>,
    pub clients: usize,
    pub unresolved: Vec<Unresolved>,
}

#[derive(Debug, Clone, Default)]
struct InstanceState {
    online: bool,
    busy: bool,
    set_name: String,
}

/// Where STAGE AUT acts: the stage-mic track's mute (its instance's
/// transport drives it, `Router::transport`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct StageTarget {
    instance: String,
    target: String,
}

/// One instance's transport as the router follows it: its subscription's
/// hub key and the latest `is_playing` heard (none before the first, and
/// again after a disconnect).
#[derive(Debug, Clone)]
struct TransportWatch {
    key: String,
    playing: Option<bool>,
}

/// The `transport` record's fields for a value `playing` after `prev`; none
/// when it changed nothing.
fn transport_change(prev: Option<bool>, playing: bool) -> Option<Value> {
    (prev != Some(playing)).then(|| json!({"playing": playing, "prev": prev}))
}

/// What a delivery of the rules' transport subscription says: its error
/// first (the listener failed: the rules are blind, #130 review M6), else
/// its value; none when it holds neither.
fn transport_heard(value: Option<Value>, error: Option<String>) -> Option<Result<Value, String>> {
    error.map(Err).or_else(|| value.map(Ok))
}

/// What the router needs besides its instances: its own sender (a batch's
/// result comes back as a message) and the event log.
pub struct RouterIo {
    pub tx: mpsc::UnboundedSender<RouterMsg>,
    pub events: EventLog,
}

/// The router task's state.
pub struct Router {
    subs: Subs,
    names: NameCheck,
    clients: HashMap<ClientId, Arc<Outbox>>,
    /// Each client's peer address (event-log records).
    peers: HashMap<ClientId, String>,
    /// Whether each client's socket came over the LAN (#90: AbleSet's page
    /// message names it).
    lan: HashMap<ClientId, bool>,
    live: BTreeMap<String, LiveHandle>,
    states: BTreeMap<String, InstanceState>,
    setters: BTreeMap<String, Setter>,
    /// The persisted hub values (`hub-state.json`), kept whole and saved
    /// whole (#130).
    state: HubState,
    stage: StageAut,
    stage_target: Option<StageTarget>,
    /// REVERB (#130, D18).
    reverb: reverb::ReverbIo,
    /// The fader links' switches (#130, D18).
    links: FaderLinks,
    /// AUTOTUNE (#104, D19).
    autotune: autotune::AutotuneIo,
    /// The song map (#140).
    songmap: songmap::SongMapIo,
    /// Every instance's transport (#104, D18), by instance.
    transport: BTreeMap<String, TransportWatch>,
    layout_rev: u64,
    data_dir: PathBuf,
    io: RouterIo,
    /// The setters' clock (monotonic, ms since the router started).
    started: std::time::Instant,
    /// The Stream Deck (#52); none without `[companion]`.
    deck: Option<deck::DeckIo>,
    /// The Pro-Q 4 screen (#71 PR E); none with `[eq] backend = "off"`.
    eq: Option<eq::EqIo>,
    /// AbleSet's page (#90 B3); none without `[ableset]`.
    ableset: Option<ableset::AbleSetIo>,
    /// Keeps the strips' groups unfolded (#58).
    unfold: unfold::Keeper,
    /// Finds the Tuner markers (#68).
    markers: markers::Keeper,
    /// The layout store the markers go to (#68); none in tests that need
    /// no markers.
    layout_store: Option<Arc<crate::layout::LayoutStore>>,
    /// The latest markers waiting for their quiet time, and its number.
    markers_pending: Option<Vec<fohmixer_proto::markers::Found>>,
    markers_generation: u64,
    /// Stamped at every turn of the loop (PR #146 review I1): the Live
    /// clients' writers ping only while it is fresh.
    beat: Beat,
}

/// The `link` record of a script's close with a reason (PR #146 review
/// M5): its instance and the close's words (`1001 silent`).
pub(crate) fn closed_fields(instance: &str, why: &str) -> Value {
    json!({"instance": instance, "closed": why})
}

/// The router's beat (PR #146 review I1): a [`RouterMsg::Beat`] every
/// `period`, so an idle loop still turns and stamps its `Beat`; it ends
/// with the router's channel.
pub(crate) async fn beat_ticks(
    router: mpsc::UnboundedSender<RouterMsg>,
    period: std::time::Duration,
) {
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if router.send(RouterMsg::Beat).is_err() {
            return;
        }
    }
}

impl Router {
    /// A router over these instances, with the persisted hub values
    /// (`hub-state.json`: STAGE AUT, REVERB, the fader links' switches,
    /// AUTOTUNE AUT and the singers off by hand, #140 PR 3b; a saved
    /// AUTOTUNE AUT on is honoured since #140 PR 3c).
    pub fn new(
        live: BTreeMap<String, LiveHandle>,
        state: HubState,
        data_dir: PathBuf,
        io: RouterIo,
    ) -> Self {
        // The rules follow every instance's transport from the start (the
        // table resolves it at the connect).
        let mut subs = Subs::new(live.keys().cloned());
        let transport: BTreeMap<String, TransportWatch> = live
            .keys()
            .filter_map(|name| {
                subs.subscribe(RULES_CLIENT, name, "live_set", "is_playing", false)
                    .ok()
                    .map(|reply| {
                        let watch = TransportWatch {
                            key: reply.key,
                            playing: None,
                        };
                        (name.clone(), watch)
                    })
            })
            .collect();
        Self {
            subs,
            names: NameCheck::default(),
            clients: HashMap::new(),
            // The REVERB rule's writes name it in the event log (#130).
            peers: HashMap::from([(reverb::REVERB_CLIENT, reverb::REVERB_PEER.to_string())]),
            lan: HashMap::new(),
            states: live
                .keys()
                .map(|k| (k.clone(), InstanceState::default()))
                .collect(),
            setters: live
                .keys()
                .map(|k| (k.clone(), Setter::default()))
                .collect(),
            live,
            stage: StageAut::new(state.stage_aut),
            stage_target: None,
            reverb: reverb::ReverbIo::new(&state.reverb),
            links: FaderLinks::new(&state.fader_links),
            autotune: autotune::AutotuneIo::new(state.autotune_aut, state.autotune_off.clone()),
            songmap: songmap::SongMapIo::default(),
            state,
            transport,
            layout_rev: 0,
            data_dir,
            io,
            started: std::time::Instant::now(),
            deck: None,
            eq: None,
            ableset: None,
            unfold: unfold::Keeper::default(),
            markers: markers::Keeper::default(),
            layout_store: None,
            markers_pending: None,
            markers_generation: 0,
            beat: Beat::steady(),
        }
    }

    /// Stamps `beat` at every turn of its loop (the hub's; the Live
    /// clients' writers read it).
    pub fn with_beat(mut self, beat: Beat) -> Self {
        self.beat = beat;
        self
    }

    /// Serves messages until [`RouterMsg::Stop`], then releases the Stream
    /// Deck's held keys and tells its task to stop (#52), hands the Pro-Q 4
    /// editors' windows back (#71 PR E), and closes every client (it holds a
    /// sender of its own for its batches' results, so the channel never ends
    /// by itself: the hub's stop sends `Stop`).
    pub async fn run(mut self, mut rx: mpsc::UnboundedReceiver<RouterMsg>) {
        while let Some(msg) = rx.recv().await {
            let go_on = self.handle(msg);
            self.flush();
            // The loop turned: the Live clients' writers may ping (PR #146
            // review I1).
            self.beat.stamp();
            if !go_on {
                break;
            }
        }
        self.deck_stop();
        self.eq_stop();
        for outbox in self.clients.values() {
            outbox.close();
        }
        tracing::info!("router stopped");
    }

    fn handle(&mut self, msg: RouterMsg) -> bool {
        match msg {
            RouterMsg::Live { instance, event } => self.live_event(&instance, event),
            RouterMsg::Attach {
                client,
                outbox,
                peer,
                lan,
            } => {
                for name in self.states.keys() {
                    self.send_instance(&outbox, name);
                }
                outbox.hub(HUB_STAGE_AUT, json!(self.stage.is_on()));
                self.reverb_attach(&outbox);
                self.autotune_attach(&outbox);
                self.links_attach(&outbox);
                if self.layout_rev > 0 {
                    outbox.layout(self.layout_rev);
                }
                self.deck_attach(client, &outbox);
                self.eq_attach(client, &outbox);
                self.ableset_attach(&outbox, lan);
                self.clients.insert(client, outbox);
                self.peers.insert(client, peer);
                self.lan.insert(client, lan);
            }
            RouterMsg::Sub {
                client,
                instance,
                target,
                prop,
                display,
            } => {
                let reply = match self
                    .subs
                    .subscribe(client, &instance, &target, &prop, display)
                {
                    Ok(reply) => subbed(&reply.key, reply.cached.as_ref()),
                    Err((key, error)) => subbed(&key, Some(&Cached::Error(error))),
                };
                if let Some(outbox) = self.clients.get(&client) {
                    outbox.reply(reply);
                }
            }
            RouterMsg::Unsub { client, sub } => {
                self.subs.unsubscribe(client, &sub);
                if let Some(outbox) = self.clients.get(&client) {
                    outbox.forget(&sub);
                }
            }
            RouterMsg::SetHub { client, key, value } => self.set_hub(client, &key, &value),
            RouterMsg::Set {
                client,
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
                hub_ms,
                offset_ms,
            } => self.on_set(writes::SetMsg {
                client,
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
                hub_ms,
                offset_ms,
            }),
            RouterMsg::Sets {
                client,
                sets,
                hub_ms,
                offset_ms,
            } => self.on_sets(
                sets.into_iter()
                    .map(|s| writes::SetMsg {
                        client,
                        instance: s.instance,
                        target: s.target,
                        prop: s.prop,
                        value: s.value,
                        seq: s.seq,
                        t: s.t,
                        is_final: s.is_final,
                        hub_ms,
                        offset_ms,
                    })
                    .collect(),
            ),
            RouterMsg::UnfoldRead {
                instance,
                seq,
                outcome,
            } => {
                if let Err(error) = &outcome {
                    tracing::warn!(instance = %instance, error = %error, "the hub could not read the groups of the strips' tracks");
                }
                let actions = self.unfold.read_done(&instance, seq, &outcome);
                self.unfold_apply(actions);
            }
            RouterMsg::UnfoldRetry { instance } => {
                let actions = self.unfold.retry(&instance);
                self.unfold_apply(actions);
            }
            RouterMsg::MarkersRead {
                instance,
                seq,
                step,
                outcome,
            } => {
                if let Err(error) = &outcome {
                    tracing::warn!(instance = %instance, error = %error, "the hub could not read the Tuner markers");
                }
                let actions = self.markers.read_done(&instance, seq, step, &outcome);
                self.markers_apply(actions);
                // A read that leaves every strip where it was ends REVERB's
                // track-list window (#130 review W1, M1) and AUTOTUNE's
                // hold (#104, PR #139 review I1).
                if self.reverb_settled(&instance) {
                    self.reverb_tracks_known(&instance);
                }
                self.autotune_release(&instance, "settled");
            }
            RouterMsg::MarkersRetry { instance } => self.markers_retried(instance),
            RouterMsg::MarkersSettle { generation } => self.markers_settled(generation),
            RouterMsg::Applied {
                instance,
                batch,
                outcome,
            } => self.on_applied(&instance, batch, &outcome),
            RouterMsg::Detach { client } => {
                self.deck_detach(client);
                self.eq_detach(client);
                self.subs.drop_client(client);
                self.clients.remove(&client);
                self.peers.remove(&client);
                self.lan.remove(&client);
                for setter in self.setters.values_mut() {
                    setter.drop_client(client);
                }
            }
            RouterMsg::Layout {
                rev,
                stage,
                targets,
                strips,
                reverb,
                fader_leaders,
                autotune,
            } => {
                // Two producers (the file's poll and the markers, #68): an
                // older revision arriving late never replaces a newer one.
                if rev <= self.layout_rev {
                    return true;
                }
                self.layout_rev = rev;
                for outbox in self.clients.values() {
                    outbox.layout(rev);
                }
                self.set_stage_binding(stage.as_ref());
                self.reverb_layout(reverb);
                self.autotune_layout(autotune);
                self.links_layout(fader_leaders);
                let actions = self.unfold.set_strips(strips);
                self.unfold_apply(actions);
                self.names.set_targets(targets);
                let online: Vec<String> = self
                    .states
                    .iter()
                    .filter(|(_, state)| state.online)
                    .map(|(name, _)| name.clone())
                    .collect();
                for name in online {
                    self.names.start(&name);
                }
            }
            RouterMsg::Migration { apply, reply } => self.markers_migration(apply, reply),
            RouterMsg::MigrationLists {
                apply,
                reply,
                lists,
            } => {
                let _ = reply.send(self.markers_migration_lists(apply, &lists));
            }
            RouterMsg::Status { reply } => {
                let names = self.live.keys();
                let _ = reply.send(RouterStatus {
                    subscriptions: names
                        .clone()
                        .map(|n| {
                            let own = [
                                UNFOLD_CLIENT,
                                MARKERS_CLIENT,
                                RULES_CLIENT,
                                reverb::REVERB_CLIENT,
                                autotune::AUTOTUNE_CLIENT,
                                songmap::SONGMAP_CLIENT,
                            ];
                            (n.clone(), self.subs.subscriptions_besides(n, &own))
                        })
                        .collect(),
                    unfolded: unfold::by_instance(self.unfold.held()),
                    listeners: names
                        .clone()
                        .map(|n| {
                            let own = [
                                UNFOLD_CLIENT,
                                MARKERS_CLIENT,
                                RULES_CLIENT,
                                reverb::REVERB_CLIENT,
                                autotune::AUTOTUNE_CLIENT,
                                songmap::SONGMAP_CLIENT,
                            ];
                            (n.clone(), self.subs.listeners_besides(n, &own))
                        })
                        .collect(),
                    playing: names.map(|n| (n.clone(), self.transport_now(n))).collect(),
                    stage_aut: StageAutStatus {
                        on: self.stage.is_on(),
                        writes: self.stage.writes(),
                    },
                    reverb: self.reverb.rule.status(),
                    fader_links: self.links.status(),
                    autotune: self.autotune.status(),
                    songmap: self.songmap.status(),
                    clients: self.clients.len(),
                    unresolved: self.names.unresolved(),
                });
            }
            RouterMsg::DeckView { client, on } => self.deck_view(client, on),
            RouterMsg::DeckPress {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            } => self.deck_press(deck::PressMsg {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            }),
            RouterMsg::Heard { client } => {
                self.deck_heard(client);
                self.eq_heard(client);
            }
            RouterMsg::Deck { event } => self.deck_event(event),
            RouterMsg::Tick => {
                self.deck_tick();
                self.eq_tick();
            }
            RouterMsg::EqList { client, binding } => self.eq_list(client, binding),
            RouterMsg::EqListed {
                client,
                binding,
                outcome,
            } => self.eq_listed(client, binding, outcome),
            RouterMsg::EqOpen {
                client,
                instance,
                path,
                area,
            } => self.eq_open(client, &instance, &path, area),
            RouterMsg::EqArea { client, area } => self.eq_area(client, area),
            RouterMsg::EqOpened {
                key,
                session,
                outcome,
                reference,
                connection,
                resized,
            } => {
                self.eq_resized(&key, session, resized.as_ref());
                self.eq_opened(&key, session, outcome, reference, connection);
            }
            RouterMsg::EqResized {
                key,
                session,
                resized,
            } => self.eq_resized(&key, session, resized.as_ref()),
            RouterMsg::EqInput { client, input } => self.eq_input(client, input),
            RouterMsg::EqView { client, part } => self.eq_view(client, part),
            RouterMsg::EqClose { client } => self.eq_close(client),
            RouterMsg::EqUndo { client, n } => self.eq_undo(client, n),
            RouterMsg::EqUndone { undid } => self.eq_undone(&undid),
            RouterMsg::EqClosed {
                key,
                session,
                outcome,
            } => self.eq_closed(&key, session, outcome),
            RouterMsg::EqWorker { event } => self.eq_worker(event),
            RouterMsg::AbleSetLink { online } => self.ableset_link(online),
            RouterMsg::ReverbWake { generation } => self.reverb_wake(generation),
            RouterMsg::AutotuneFound { key, walk, outcome } => {
                self.autotune_found(&key, walk, outcome);
            }
            RouterMsg::AutotuneWalkFailed { key, walk, why } => {
                self.autotune_walk_failed(&key, walk, why);
            }
            RouterMsg::AutotuneWalkRetry { key, walk } => self.autotune_walk_retry(&key, walk),
            RouterMsg::AutotuneWritten {
                instance,
                write,
                selectors,
                outcome,
            } => self.autotune_written(&instance, write, &selectors, &outcome),
            RouterMsg::AutotuneSafe {
                instance,
                seq,
                targets,
                outcome,
            } => self.autotune_safe_answered(&instance, seq, targets, &outcome),
            RouterMsg::AutotuneSafeRetry { instance, seq } => {
                self.autotune_safe_retry(&instance, seq);
            }
            RouterMsg::AutotuneSafeProbed {
                instance,
                seq,
                ids,
                outcome,
            } => self.autotune_safe_probed(&instance, seq, &ids, &outcome),
            RouterMsg::AutotuneSafeProbeRetry { instance, seq } => {
                self.autotune_safe_probe_retry(&instance, seq);
            }
            RouterMsg::Beat => {}
            RouterMsg::AutotuneKeysWritten {
                instance,
                write,
                keys,
                outcome,
                rtt_ms,
            } => self.autotune_keys_written(&instance, write, &keys, &outcome, rtt_ms),
            RouterMsg::AutotuneKeysRead {
                instance,
                write,
                keys,
                outcome,
            } => self.autotune_keys_read(&instance, write, &keys, &outcome),
            RouterMsg::SongMapRead {
                instance,
                read_no,
                result,
                ms,
            } => self.songmap_read(&instance, read_no, result, ms),
            RouterMsg::SongMap { instance, reply } => {
                let _ = reply.send(self.songmap_report(&instance));
            }
            RouterMsg::SongMapRetry { instance, read_no } => self.songmap_retry(&instance, read_no),
            RouterMsg::Stop => return false,
        }
        true
    }

    fn live_event(&mut self, instance: &str, event: LiveEvent) {
        match event {
            LiveEvent::Connected(info) => {
                if let Some(state) = self.states.get_mut(instance) {
                    *state = InstanceState {
                        online: true,
                        busy: false,
                        set_name: info.set_name,
                    };
                }
                self.subs.connected(instance);
                self.names.start(instance);
                // The script's object refs start over with each connection:
                // the marker keeper reads the Tuners (and their refs) again,
                // and the Pro-Q 4 screen forgets its lists and refs.
                let actions = self.markers.connected(instance);
                self.markers_apply(actions);
                self.eq_connected(instance);
                // AUTOTUNE's refs too: its targets are walked again.
                self.autotune_connected(instance);
                // The song map follows the set's lists (#140).
                self.songmap_connected(instance);
                self.broadcast_instance(instance);
            }
            LiveEvent::Closed { why } => {
                // The script's reason (PR #146 review M5), before its
                // disconnect: a silent hub's close tells from a crash.
                self.io.events.record("link", closed_fields(instance, &why));
            }
            LiveEvent::Disconnected => {
                if let Some(state) = self.states.get_mut(instance) {
                    *state = InstanceState::default();
                }
                if let Some(setter) = self.setters.get_mut(instance) {
                    setter.on_disconnect();
                }
                self.subs.disconnected(instance);
                self.names.stop(instance);
                if let Some(watch) = self.transport.get_mut(instance) {
                    watch.playing = None;
                }
                if self
                    .stage_target
                    .as_ref()
                    .is_some_and(|t| t.instance == instance)
                {
                    self.stage.on_disconnect();
                }
                // Until a read after the next connect, the strips' places are
                // unknown (a set loaded meanwhile): REVERB's window opens.
                self.markers.disconnected(instance);
                self.reverb_offline(instance);
                self.autotune_offline(instance);
                self.songmap_offline(instance);
                self.broadcast_instance(instance);
            }
            LiveEvent::Busy {
                busy,
                changed,
                tick_age_ms,
                reason,
            } => {
                if changed {
                    if let Some(state) = self.states.get_mut(instance) {
                        state.busy = busy;
                    }
                    self.broadcast_instance(instance);
                    self.io.events.record(
                        "link",
                        json!({"instance": instance, "busy": busy,
                               "tick_age_ms": tick_age_ms, "reason": reason}),
                    );
                }
                for outbox in self.clients.values() {
                    outbox.link(ServerMsg::Link {
                        instance: instance.to_string(),
                        tick_age_ms,
                        busy,
                    });
                }
            }
            LiveEvent::Result { uuid, data } => {
                if !self.subs.on_result(instance, &uuid, &data) {
                    self.names.on_result(instance, &uuid, &data);
                }
            }
            LiveEvent::Values(items) => self.subs.on_values(instance, &items),
        }
    }

    /// An instance's state for one client.
    fn send_instance(&self, outbox: &Outbox, name: &str) {
        let state = self.states.get(name).cloned().unwrap_or_default();
        outbox.instance(
            name,
            state.online,
            ServerMsg::Instance {
                name: name.to_string(),
                online: state.online,
                busy: state.busy,
                set_name: state.set_name,
            },
        );
    }

    fn broadcast_instance(&self, name: &str) {
        for outbox in self.clients.values() {
            self.send_instance(outbox, name);
        }
    }

    fn set_hub(&mut self, client: ClientId, key: &str, value: &Value) {
        // The fader links' switches first (#130), then REVERB's keys, then
        // AUTOTUNE's (#104), then STAGE AUT.
        if self.links_set_hub(client, key, value) {
            return;
        }
        if self.reverb_set_hub(client, key, value) {
            return;
        }
        if self.autotune_set_hub(client, key, value) {
            return;
        }
        let (true, Some(on)) = (key == HUB_STAGE_AUT, value.as_bool()) else {
            if let Some(outbox) = self.clients.get(&client) {
                outbox.reply(ServerMsg::Error {
                    id: None,
                    message: format!("no hub value {key:?} taking {value}"),
                });
            }
            return;
        };
        let changed = on != self.stage.is_on();
        self.io.events.record(
            "stage_aut",
            json!({"step": "flag", "on": on, "client": client, "peer": self.peers.get(&client)}),
        );
        let write = self.stage.set_flag(on);
        if changed {
            tracing::info!(on, "STAGE AUT turned {}", if on { "on" } else { "off" });
            self.state.stage_aut = on;
            self.save_state();
        }
        for outbox in self.clients.values() {
            outbox.hub(HUB_STAGE_AUT, json!(on));
        }
        if let Some(mute) = write {
            self.write_stage(mute);
        }
    }

    /// Writes the hub values whole (#130: a save of one value keeps the
    /// others).
    fn save_state(&self) {
        if let Err(e) = self.state.save(&self.data_dir) {
            tracing::error!(error = %e, "cannot save the hub's values (hub-state.json)");
        }
    }

    /// Follows a new layout's STAGE AUT binding: the rule starts afresh on
    /// the new target, with its instance's transport as last heard.
    fn set_stage_binding(&mut self, binding: Option<&Binding>) {
        let wanted = binding.and_then(|b| b.target().ok().map(|t| (b.instance.clone(), t)));
        let current = self
            .stage_target
            .as_ref()
            .map(|t| (t.instance.clone(), t.target.clone()));
        if wanted == current {
            return;
        }
        self.stage_target = None;
        self.stage.on_disconnect();
        let Some((instance, target)) = wanted else {
            tracing::info!("the layout has no STAGE AUT binding");
            return;
        };
        tracing::info!(instance = %instance, target = %target, "STAGE AUT follows the transport");
        let playing = self.transport_now(&instance);
        self.stage_target = Some(StageTarget { instance, target });
        if let Some(playing) = playing {
            self.stage_value(playing);
        }
    }

    /// The latest `is_playing` of `instance` (none before one was heard).
    pub(super) fn transport_now(&self, instance: &str) -> Option<bool> {
        self.transport.get(instance).and_then(|w| w.playing)
    }

    /// A transport value of the hub key `sub`: recorded on a change, then
    /// handed to the rules.
    fn on_transport(&mut self, sub: &str, value: &Value) {
        let Some(playing) = value.as_bool() else {
            return;
        };
        let Some((instance, watch)) = self.transport.iter_mut().find(|(_, w)| w.key == sub) else {
            return;
        };
        let instance = instance.clone();
        let prev = watch.playing;
        let Some(mut fields) = transport_change(prev, playing) else {
            return;
        };
        watch.playing = Some(playing);
        fields["instance"] = json!(instance);
        tracing::info!(instance = %instance, playing, "the transport {}", if playing { "started" } else { "stopped" });
        self.io.events.record("transport", fields);
        if self
            .stage_target
            .as_ref()
            .is_some_and(|t| t.instance == instance)
        {
            self.stage_value(playing);
        }
        self.reverb_transport(&instance, playing);
        self.autotune_transport(&instance, playing);
        self.songmap_transport(&instance, prev);
    }

    /// The rules' transport subscription of the hub key `sub` failed: the
    /// transport is not known any more (a later value is a first one),
    /// logged once per failed delivery and recorded (warn class: an error).
    fn transport_failed(&mut self, sub: &str, error: &str) {
        let Some((instance, watch)) = self.transport.iter_mut().find(|(_, w)| w.key == sub) else {
            return;
        };
        let instance = instance.clone();
        watch.playing = None;
        tracing::warn!(instance = %instance, error = ?error, "the rules cannot follow the transport");
        self.io
            .events
            .record("transport", json!({"instance": instance, "error": error}));
        self.reverb_transport_lost(&instance);
        self.autotune_transport_lost(&instance);
    }

    /// A transport value for STAGE AUT.
    fn stage_value(&mut self, playing: bool) {
        if let Some(mute) = self.stage.on_playing(playing) {
            self.write_stage(mute);
        }
    }

    /// Writes the stage mics' mute (logged and recorded; a failure is
    /// logged and recorded, never retried — spec I6).
    fn write_stage(&self, mute: bool) {
        let Some(target) = &self.stage_target else {
            return;
        };
        let Some(live) = self.live.get(&target.instance) else {
            return;
        };
        tracing::info!(instance = %target.instance, target = %target.target, mute, "STAGE AUT writes the stage mics' mute");
        self.io.events.record(
            "stage_aut",
            json!({"step": "write", "instance": target.instance, "mute": mute}),
        );
        let result = live.call(vec![json!({
            "target": target.target,
            "name": "set_prop",
            "args": {"prop": "mute", "value": mute},
        })]);
        let what = target.target.clone();
        let events = self.io.events.clone();
        tokio::spawn(async move {
            if let Some(problem) = write_failure(&result.await) {
                tracing::warn!(target = %what, problem = %problem, "STAGE AUT write failed");
                events.record(
                    "stage_aut",
                    json!({"step": "failed", "target": what, "problem": problem}),
                );
            }
        });
    }

    /// Carries out the unfold keeper's actions (#58): its subscriptions
    /// first, then the values they already held. Those ask only for reads
    /// and writes (`Keeper::value`), so two steps do it, with no loop a
    /// mutant could make endless.
    fn unfold_apply(&mut self, actions: Vec<unfold::Action>) {
        let cached = self.unfold_do(actions);
        let mut next = Vec::new();
        for (key, state) in cached {
            next.extend(self.unfold.value(&key, cached_value(&state)));
        }
        self.unfold_do(next);
    }

    /// Carries out actions; the subscriptions' cached states, by key.
    fn unfold_do(&mut self, actions: Vec<unfold::Action>) -> Vec<(String, Cached)> {
        let mut cached: Vec<(String, Cached)> = Vec::new();
        for action in actions {
            match action {
                unfold::Action::Sub(watch) => {
                    let (instance, target, prop) = watch.target();
                    let instance = instance.to_string();
                    match self
                        .subs
                        .subscribe(UNFOLD_CLIENT, &instance, &target, prop, false)
                    {
                        Ok(reply) => {
                            self.unfold.subscribed(watch, reply.key.clone());
                            if let Some(state) = reply.cached {
                                cached.push((reply.key, state));
                            }
                        }
                        Err((_, error)) => {
                            tracing::warn!(instance = %instance, target = %target, error = %error, "the hub cannot keep the strips' groups unfolded there");
                        }
                    }
                }
                unfold::Action::Unsub { key } => {
                    self.subs.unsubscribe(UNFOLD_CLIENT, &key);
                }
                unfold::Action::Read {
                    instance,
                    seq,
                    commands,
                } => self.unfold_read(instance, seq, commands),
                unfold::Action::Unfold {
                    instance,
                    target,
                    group,
                } => self.unfold_group(&instance, target, &group),
                unfold::Action::Retry { instance } => self.unfold_retry(instance),
            }
        }
        cached
    }

    /// Reads the groups the strips' tracks sit in; the answer comes back as
    /// [`RouterMsg::UnfoldRead`].
    fn unfold_read(&self, instance: String, seq: u64, commands: Vec<Value>) {
        let Some(live) = self.live.get(&instance) else {
            return;
        };
        let result = live.call(commands);
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::UnfoldRead {
                instance,
                seq,
                outcome,
            });
        });
    }

    /// Asks the keeper to read `instance` again after `RETRY_MS`.
    fn unfold_retry(&self, instance: String) {
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(unfold::RETRY_MS)).await;
            let _ = tx.send(RouterMsg::UnfoldRetry { instance });
        });
    }

    /// Unfolds a group a strip's track sits in, through the path it was
    /// read by (logged; a failure is logged, and the next read tries again).
    fn unfold_group(&self, instance: &str, target: String, group: &str) {
        let Some(live) = self.live.get(instance) else {
            return;
        };
        tracing::info!(instance = %instance, group = %group, "the hub unfolds a group a strip's track sits in (Live meters no track inside a folded group)");
        let result = live.call(vec![json!({
            "target": target,
            "name": "set_prop",
            "args": {"prop": unfold::FOLD, "value": false},
        })]);
        tokio::spawn(async move {
            if let Some(problem) = write_failure(&result.await) {
                tracing::warn!(target = %target, problem = %problem, "the unfold failed");
            }
        });
    }

    /// Sends the table's and the name check's requests.
    fn send_requests(&mut self) {
        let requests: Vec<Outgoing> = self
            .subs
            .drain_outgoing()
            .into_iter()
            .chain(self.names.drain_outgoing())
            .collect();
        for out in requests {
            if let Some(live) = self.live.get(&out.instance) {
                live.send(out.uuid, out.commands);
            }
        }
    }

    /// Sends the requests and hands out the table's values (the unfold
    /// keeper's to the keeper: they ask only for reads and writes, which go
    /// out on their own). REVERB (#130) gets its faders' volumes, then the
    /// track lists the marker keeper follows, then AUTOTUNE (#104) its
    /// macros' values, then the song map (#140) its lists, before the
    /// transport: a reconnect's values are judged before its first transport
    /// value.
    fn flush(&mut self) {
        self.send_requests();
        let mut unfold = Vec::new();
        let mut marker_items: Vec<(String, Option<Value>)> = Vec::new();
        let mut transport: Vec<(String, Result<Value, String>)> = Vec::new();
        let mut reverb_values: Vec<(String, f64)> = Vec::new();
        let mut reverb_lists: Vec<(String, Value)> = Vec::new();
        let mut autotune_items: Vec<ValueItem> = Vec::new();
        let mut songmap_items: Vec<ValueItem> = Vec::new();
        for (client, item) in self.subs.take_deliveries() {
            if client == RULES_CLIENT {
                if let Some(heard) = transport_heard(item.value, item.error) {
                    transport.push((item.sub, heard));
                }
            } else if client == UNFOLD_CLIENT {
                if let Some(error) = &item.error {
                    tracing::warn!(key = %item.sub, error = %error, "a list the hub's unfold keeper listens to failed");
                }
                unfold.extend(self.unfold.value(&item.sub, item.value.as_ref()));
            } else if client == MARKERS_CLIENT {
                if let Some(error) = &item.error {
                    tracing::warn!(key = %item.sub, error = %error, "a list or name the hub's marker keeper listens to failed");
                }
                if reverb::list_instance(&item.sub).is_some()
                    && let Some(value) = &item.value
                {
                    reverb_lists.push((item.sub.clone(), value.clone()));
                }
                marker_items.push((item.sub, item.value));
            } else if client == reverb::REVERB_CLIENT {
                if let Some(v) = item.value.as_ref().and_then(Value::as_f64) {
                    reverb_values.push((item.sub, v));
                }
            } else if client == autotune::AUTOTUNE_CLIENT {
                autotune_items.push(item);
            } else if client == songmap::SONGMAP_CLIENT {
                songmap_items.push(item);
            } else if let Some(outbox) = self.clients.get(&client) {
                outbox.value(item);
            }
        }
        for (sub, value) in reverb_values {
            self.reverb_value(&sub, value);
        }
        for (sub, value) in reverb_lists {
            self.reverb_list(&sub, &value);
        }
        for item in autotune_items {
            self.autotune_delivery(&item.sub, item.value, item.error);
        }
        self.songmap_delivery(songmap_items);
        for (sub, heard) in transport {
            match heard {
                Ok(value) => self.on_transport(&sub, &value),
                Err(error) => self.transport_failed(&sub, &error),
            }
        }
        self.unfold_apply(unfold);
        let marker_actions = self
            .markers
            .values(marker_items.iter().map(|(k, v)| (k.as_str(), v.as_ref())));
        self.markers_apply(marker_actions);
    }
}

/// The `Layout` message of the layout served at revision `rev`: the file
/// changed, or the markers did (#68).
pub fn layout_msg(rev: u64, served: Option<&Layout>) -> RouterMsg {
    RouterMsg::Layout {
        rev,
        stage: served.and_then(|l| l.stage_aut_binding().cloned()),
        targets: served
            .map(crate::live::names::layout_targets)
            .unwrap_or_default(),
        strips: served.map(Layout::strip_tracks).unwrap_or_default(),
        reverb: served.map(Layout::reverb_faders).unwrap_or_default(),
        fader_leaders: served.map(Layout::fader_leaders).unwrap_or_default(),
        autotune: served.map(Layout::autotune_targets).unwrap_or_default(),
    }
}

/// A cached state's value for the unfold keeper (none: an error).
fn cached_value(state: &Cached) -> Option<&Value> {
    match state {
        Cached::Value { value, .. } => Some(value),
        Cached::Error(_) => None,
    }
}

/// Why a STAGE AUT write did not happen, if it did not.
fn write_failure(outcome: &Result<Vec<Value>, LiveError>) -> Option<String> {
    match outcome {
        Ok(slots) if slots.first().and_then(|s| s.get("ok")) == Some(&json!(true)) => None,
        Ok(slots) => Some(Value::from(slots.clone()).to_string()),
        Err(e) => Some(e.to_string()),
    }
}

/// The `subbed` reply for a key in a state.
fn subbed(key: &str, cached: Option<&Cached>) -> ServerMsg {
    let item = cached.map_or_else(
        || ValueItem {
            sub: key.to_string(),
            value: None,
            display: None,
            error: None,
        },
        |c| c.item(key),
    );
    ServerMsg::Subbed {
        sub: item.sub,
        value: item.value,
        display: item.display,
        error: item.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::AckItem;

    #[tokio::test(start_paused = true)]
    async fn a_stalled_router_stops_its_writers_pings() {
        // PR #146 review I1: the pings say the router decides, not only that
        // the process lives. While its loop turns (a beat comes every 200
        // ms) its writers ping; once it no longer turns (here it stopped;
        // a hang or a panic is the same to the beat) the pings stop within
        // 600 ms, and the script falls back 1.5 s after the last one.
        use crate::live::client::{BEAT_FRESH_MS, Beat, ROUTER_BEAT_MS, write_requests};
        use tokio_tungstenite::tungstenite::Message;
        let dir = tempfile::tempdir().unwrap();
        let (router, own, _records) = offline_router(dir.path());
        let tx = router.io.tx.clone();
        let beat = Beat::router();
        let start = tokio::time::Instant::now();
        let pings = Arc::new(std::sync::Mutex::new(Vec::<u128>::new()));
        let log = Arc::clone(&pings);
        let sink = Box::pin(futures_util::sink::unfold((), move |(), m: Message| {
            let log = Arc::clone(&log);
            async move {
                if matches!(m, Message::Ping(_)) {
                    log.lock().unwrap().push(start.elapsed().as_millis());
                }
                Ok::<(), std::convert::Infallible>(())
            }
        }));
        let (_texts, texts) = mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(write_requests(sink, texts, beat.clone()));
        let routed = tokio::spawn(router.with_beat(beat.clone()).run(own));
        let ticks = tokio::spawn(beat_ticks(
            tx.clone(),
            std::time::Duration::from_millis(ROUTER_BEAT_MS),
        ));
        tokio::time::sleep(std::time::Duration::from_millis(2950)).await;
        assert_eq!(
            *pings.lock().unwrap(),
            vec![500, 1000, 1500, 2000, 2500],
            "pings while the loop turns"
        );
        assert!(beat.fresh());
        tx.send(RouterMsg::Stop).unwrap();
        routed.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
        assert_eq!(
            *pings.lock().unwrap(),
            vec![500, 1000, 1500, 2000, 2500, 3000, 3500],
            "the last stamp at 2950 ms: pings until {} ms after it, none later",
            BEAT_FRESH_MS
        );
        assert!(!beat.fresh());
        ticks.await.unwrap();
        writer.abort();
    }

    #[tokio::test]
    async fn a_scripts_close_with_a_reason_is_a_link_record() {
        // PR #146 review M5: a silent hub's close is told from a crash.
        assert_eq!(
            closed_fields("band", "1001 silent"),
            json!({"instance": "band", "closed": "1001 silent"})
        );
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _own, rx) = offline_router(dir.path());
        router.handle(RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Closed {
                why: "1001 silent".into(),
            },
        });
        let got: Vec<Value> = records(&rx)
            .into_iter()
            .filter(|r| r["ev"] == "link")
            .map(|mut r| {
                r.as_object_mut().unwrap().remove("ts");
                r
            })
            .collect();
        assert_eq!(
            got,
            vec![json!({"ev": "link", "instance": "band", "closed": "1001 silent"})]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_beat_ticks_every_200_ms_until_the_router_is_gone() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ticks = tokio::spawn(beat_ticks(tx, std::time::Duration::from_millis(200)));
        let start = tokio::time::Instant::now();
        let mut at = Vec::new();
        for _ in 0..3 {
            let msg = rx.recv().await.unwrap();
            assert!(matches!(msg, RouterMsg::Beat));
            at.push(start.elapsed().as_millis());
        }
        assert_eq!(at, vec![200, 400, 600]);
        drop(rx);
        tokio::time::timeout(std::time::Duration::from_secs(1), ticks)
            .await
            .expect("the ticks end with the router's channel")
            .unwrap();
    }

    fn router(dir: &std::path::Path) -> Router {
        let (tx, _) = mpsc::unbounded_channel();
        Router::new(
            BTreeMap::new(),
            HubState::default(),
            dir.to_path_buf(),
            RouterIo {
                tx,
                events: EventLog::off(),
            },
        )
    }

    /// A router over `band` on a port nothing listens on (the instance stays
    /// offline: a write is answered "instance offline" at once), its own
    /// messages and the event-log records.
    pub(super) fn offline_router(
        dir: &std::path::Path,
    ) -> (
        Router,
        mpsc::UnboundedReceiver<RouterMsg>,
        std::sync::mpsc::Receiver<Value>,
    ) {
        offline_router_with(dir, HubState::default())
    }

    /// The same router, started from `state` (`hub-state.json` as loaded).
    pub(super) fn offline_router_with(
        dir: &std::path::Path,
        state: HubState,
    ) -> (
        Router,
        mpsc::UnboundedReceiver<RouterMsg>,
        std::sync::mpsc::Receiver<Value>,
    ) {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let (live, _task) = LiveHandle::spawn(
            &crate::config::InstanceCfg {
                name: "band".into(),
                port,
            },
            Arc::new(|_: &str, _: LiveEvent| {}),
        );
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, records) = EventLog::channel(1024);
        let router = Router::new(
            BTreeMap::from([("band".to_string(), live)]),
            state,
            dir.to_path_buf(),
            RouterIo { tx, events },
        );
        (router, rx, records)
    }

    pub(super) fn attach(router: &mut Router, client: ClientId) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
            lan: true,
        });
        outbox
    }

    pub(super) fn set_msg(client: ClientId, instance: &str, seq: u64, value: f64) -> RouterMsg {
        RouterMsg::Set {
            client,
            instance: instance.into(),
            target: "live_set  tracks 0 mixer_device volume".into(),
            prop: "value".into(),
            value: json!(value),
            seq,
            t: 1_000.5,
            is_final: true,
            hub_ms: 1_250.5,
            offset_ms: Some(200.0),
        }
    }

    /// Every record waiting, by `ev`.
    pub(super) fn records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
        rx.try_iter().collect()
    }

    fn evs(records: &[Value]) -> Vec<&str> {
        records.iter().map(|r| r["ev"].as_str().unwrap()).collect()
    }

    const VOLUME: &str = "band|live_set tracks 0 mixer_device volume|value";

    #[test]
    fn a_transport_change_is_recorded_once_with_what_it_was_before() {
        assert_eq!(
            transport_change(None, false),
            Some(json!({"playing": false, "prev": null}))
        );
        assert_eq!(
            transport_change(Some(false), true),
            Some(json!({"playing": true, "prev": false}))
        );
        assert_eq!(
            transport_change(Some(true), false),
            Some(json!({"playing": false, "prev": true}))
        );
        assert_eq!(transport_change(Some(true), true), None);
        assert_eq!(transport_change(Some(false), false), None);
    }

    #[tokio::test]
    async fn the_router_follows_every_instances_transport_from_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        assert_eq!(router.transport.len(), 1);
        assert_eq!(
            router.transport["band"].key,
            "band|live_set|is_playing|false"
        );
        assert_eq!(router.transport_now("band"), None, "nothing heard yet");
        assert_eq!(
            router.subs.subscriptions("band"),
            1,
            "is_playing, the rules' own"
        );
        let now = status(&mut router);
        assert_eq!(now.subscriptions["band"], 0, "the hub's own");
        assert_eq!(now.listeners["band"], 0, "the hub's own");
        assert_eq!(now.playing, BTreeMap::from([("band".to_string(), None)]));
    }

    #[tokio::test]
    async fn a_flush_hands_the_transport_to_the_rules_and_a_pages_values_to_its_outbox() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        let page = attach(&mut router, 1);
        page.take();
        router.handle(RouterMsg::Sub {
            client: 1,
            instance: "band".into(),
            target: "live_set".into(),
            prop: "tempo".into(),
            display: false,
        });
        page.take();
        // The table goes online and Live answers both listeners: the rules'
        // transport and the page's tempo.
        router.subs.connected("band");
        let out = router.subs.drain_outgoing();
        assert!(!out.is_empty());
        for o in out {
            let slots: Vec<Value> = o
                .commands
                .iter()
                .map(|c| match c["args"]["prop"].as_str() {
                    Some("is_playing") => {
                        json!({"ok": true, "data": {"key": "live_1.is_playing", "value": true}})
                    }
                    Some("tempo") => {
                        json!({"ok": true, "data": {"key": "live_1.tempo", "value": 120.0}})
                    }
                    _ => json!({"ok": true, "data": null}),
                })
                .collect();
            router.subs.on_result(&o.instance, &o.uuid, &slots);
        }
        router.flush();
        // The rules heard the transport (recorded); the page gets its tempo
        // and nothing of the rules'.
        let got = records(&rx);
        assert_eq!(evs(&got), ["transport"]);
        assert_eq!(
            (&got[0]["playing"], &got[0]["prev"]),
            (&json!(true), &json!(null))
        );
        assert_eq!(router.transport_now("band"), Some(true));
        let items: Vec<(String, Option<Value>)> = page
            .take()
            .unwrap()
            .into_iter()
            .flat_map(|m| match m {
                ServerMsg::Values { items } => items
                    .into_iter()
                    .map(|i| (i.sub, i.value))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            items,
            vec![("band|live_set|tempo|false".to_string(), Some(json!(120.0)))]
        );
    }

    #[test]
    fn a_rules_delivery_says_its_error_first_then_its_value() {
        assert_eq!(
            transport_heard(Some(json!(true)), None),
            Some(Ok(json!(true)))
        );
        assert_eq!(
            transport_heard(None, Some("not observable".into())),
            Some(Err("not observable".into()))
        );
        assert_eq!(
            transport_heard(Some(json!(false)), Some("gone".into())),
            Some(Err("gone".into()))
        );
        assert_eq!(transport_heard(None, None), None);
    }

    /// The table of `router` online, with Live's answer to each pending
    /// listener: `answer(prop)` the slot for that prop.
    fn answer_listeners(router: &mut Router, answer: impl Fn(&str) -> Value) {
        router.subs.connected("band");
        let out = router.subs.drain_outgoing();
        assert!(!out.is_empty());
        for o in out {
            let slots: Vec<Value> = o
                .commands
                .iter()
                .map(|c| answer(c["args"]["prop"].as_str().unwrap_or("")))
                .collect();
            router.subs.on_result(&o.instance, &o.uuid, &slots);
        }
    }

    /// Every `values` item waiting in an outbox: its key and value.
    fn values_in(outbox: &Outbox) -> Vec<(String, Option<Value>)> {
        outbox
            .take()
            .unwrap()
            .into_iter()
            .flat_map(|m| match m {
                ServerMsg::Values { items } => items
                    .into_iter()
                    .map(|i| (i.sub, i.value))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[tokio::test]
    async fn a_failed_transport_listener_is_logged_and_recorded_and_forgets_the_transport() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        router.transport.get_mut("band").unwrap().playing = Some(true);
        answer_listeners(
            &mut router,
            |_| json!({"ok": false, "error": "not observable", "errorType": "PathError"}),
        );
        router.flush();
        let got = records(&rx);
        assert_eq!(evs(&got), ["transport"]);
        assert_eq!(
            (&got[0]["instance"], &got[0]["error"]),
            (&json!("band"), &json!("not observable"))
        );
        assert!(crate::events::is_warn(&got[0]), "{}", got[0]);
        assert_eq!(router.transport_now("band"), None, "the rules are blind");
        // A failure of another key reaches no rule.
        router.transport_failed("band|live_set|tempo|false", "gone");
        assert!(records(&rx).is_empty());
    }

    #[tokio::test]
    async fn a_page_and_the_rules_share_the_transport_and_the_rules_keep_it_after_the_pages_unsub()
    {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        let page = attach(&mut router, 1);
        page.take();
        router.handle(RouterMsg::Sub {
            client: 1,
            instance: "band".into(),
            target: "live_set".into(),
            prop: "is_playing".into(),
            display: false,
        });
        page.take();
        let key = "band|live_set|is_playing|false";
        assert_eq!(router.transport["band"].key, key, "one entry, both clients");
        answer_listeners(
            &mut router,
            |_| json!({"ok": true, "data": {"key": "live_1.is_playing", "value": false}}),
        );
        router.flush();
        // Both get the value: the page in its outbox, the rules recorded.
        assert_eq!(
            values_in(&page),
            vec![(key.to_string(), Some(json!(false)))]
        );
        assert_eq!(router.transport_now("band"), Some(false));
        assert_eq!(evs(&records(&rx)), ["transport"]);
        // The page lets it go; Live's next value still reaches the rules.
        router.handle(RouterMsg::Unsub {
            client: 1,
            sub: key.into(),
        });
        page.take();
        router.subs.on_values(
            "band",
            &[crate::live::LiveValue {
                key: "live_1.is_playing".into(),
                value: json!(true),
                display: None,
                error: None,
            }],
        );
        router.flush();
        assert_eq!(router.transport_now("band"), Some(true));
        let got = records(&rx);
        assert_eq!(evs(&got), ["transport"]);
        assert_eq!(
            (&got[0]["playing"], &got[0]["prev"]),
            (&json!(true), &json!(false))
        );
        assert!(
            values_in(&page).is_empty(),
            "the page is no subscriber any more"
        );
    }

    /// Waits (bounded) for the next record whose `ev` is `ev`.
    async fn next_record(rx: &std::sync::mpsc::Receiver<Value>, ev: &str) -> Value {
        for _ in 0..300 {
            if let Some(found) = rx.try_iter().find(|r| r["ev"] == ev) {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("no {ev} record within 3 s");
    }

    #[tokio::test]
    async fn a_transport_change_is_recorded_and_drives_stage_aut() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        let key = router.transport["band"].key.clone();
        // Not a bool, or another key: nothing.
        router.on_transport(&key, &json!(1));
        router.on_transport("band|live_set|current_song_time|false", &json!(true));
        assert!(records(&rx).is_empty());
        assert_eq!(router.transport_now("band"), None);
        // The first value, then the same again: one record.
        router.on_transport(&key, &json!(false));
        router.on_transport(&key, &json!(false));
        assert_eq!(router.transport_now("band"), Some(false));
        let got = records(&rx);
        assert_eq!(evs(&got), ["transport"]);
        assert_eq!(
            (&got[0]["instance"], &got[0]["playing"], &got[0]["prev"]),
            (&json!("band"), &json!(false), &json!(null))
        );
        assert_eq!(
            status(&mut router).playing,
            BTreeMap::from([("band".to_string(), Some(false))])
        );
        // STAGE AUT on, then a binding in the band: the rule applies the
        // transport it finds (stopped: live) and records the write.
        let a = attach(&mut router, 1);
        router.handle(RouterMsg::SetHub {
            client: 1,
            key: HUB_STAGE_AUT.into(),
            value: json!(true),
        });
        let flag = records(&rx);
        assert_eq!(evs(&flag), ["stage_aut"]);
        assert_eq!(
            (
                &flag[0]["step"],
                &flag[0]["on"],
                &flag[0]["client"],
                &flag[0]["peer"]
            ),
            (&json!("flag"), &json!(true), &json!(1), &json!("10.0.0.1"))
        );
        a.take();
        let binding = Binding {
            instance: "band".into(),
            anchor: fohmixer_proto::layout::Anchor::Track {
                name: "Mics Stage #".into(),
            },
            path: None,
        };
        router.set_stage_binding(Some(&binding));
        let write = next_record(&rx, "stage_aut").await;
        assert_eq!(
            (&write["step"], &write["instance"], &write["mute"]),
            (&json!("write"), &json!("band"), &json!(false))
        );
        assert_eq!(router.stage.writes(), 1);
        // The instance is offline: the write fails, recorded.
        let failed = next_record(&rx, "stage_aut").await;
        assert_eq!(failed["step"], "failed");
        assert_eq!(failed["target"], "live_set tracks[name=Mics Stage #]");
        assert!(
            failed["problem"].as_str().is_some_and(|p| !p.is_empty()),
            "{failed}"
        );
        // A start of the band's transport: recorded, and the rule mutes.
        router.on_transport(&key, &json!(true));
        let got = records(&rx);
        assert_eq!(evs(&got), ["transport", "stage_aut"]);
        assert_eq!(
            (&got[0]["playing"], &got[0]["prev"]),
            (&json!(true), &json!(false))
        );
        assert_eq!(
            (&got[1]["step"], &got[1]["mute"]),
            (&json!("write"), &json!(true))
        );
        // A disconnect forgets the transport: the next value is a first one.
        router.handle(RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Disconnected,
        });
        assert_eq!(router.transport_now("band"), None);
        router.on_transport(&key, &json!(true));
        let again = next_record(&rx, "transport").await;
        assert_eq!(
            (&again["playing"], &again["prev"]),
            (&json!(true), &json!(null))
        );
    }

    #[test]
    fn a_write_to_an_unknown_instance_is_acked_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (events, rx) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let mut router = Router::new(
            BTreeMap::new(),
            HubState::default(),
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        );
        let outbox = attach(&mut router, 3);
        outbox.take();
        assert!(router.handle(set_msg(3, "drums", 9, 0.5)));
        let key = "drums|live_set tracks 0 mixer_device volume|value";
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(key, 9, "unknown instance \"drums\"")]
            }]
        );
        let written = records(&rx);
        assert_eq!(evs(&written), vec!["set", "ack"]);
        assert_eq!(written[0]["unknown"], json!(true));
        assert_eq!(written[0]["peer"], "10.0.0.3");
        assert_eq!(written[0]["seq"], 9);
        assert_eq!(written[0]["t"], 1_000.5);
        assert_eq!(written[0]["final"], json!(true));
        // Sent at page 1000.5, here at hub 1250.5, the page 200 ms behind:
        // 50 ms on the way.
        assert_eq!(written[0]["hub_ms"], 1_250.5);
        assert_eq!(written[0]["offset_ms"], 200.0);
        assert_eq!(written[0]["delay_ms"], 50.0);
        assert_eq!(written[0]["gap_ms"], Value::Null);
        assert_eq!(written[0]["dropped_old"], json!(false));
        assert_eq!(written[1]["error"], "unknown instance \"drums\"");
        assert_eq!(written[1]["client"], 3);
        assert_eq!(written[1]["batch"], Value::Null, "no batch answered it");
        assert_eq!(writes::unknown_instance("x"), "unknown instance \"x\"");
    }

    #[tokio::test]
    async fn a_write_to_an_offline_instance_goes_round_and_is_acked_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, records_rx) = offline_router(dir.path());
        let outbox = attach(&mut router, 1);
        outbox.take();
        router.handle(set_msg(1, "band", 1, 0.5));
        // Batch 1 is in flight: a newer set waits.
        router.handle(set_msg(1, "band", 2, 0.6));
        router.handle(set_msg(1, "band", 2, 0.7));
        let applied = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the batch's result comes back")
            .unwrap();
        let RouterMsg::Applied {
            ref instance,
            batch,
            ref outcome,
        } = applied
        else {
            panic!("an applied message")
        };
        assert_eq!((instance.as_str(), batch), ("band", 1));
        assert_eq!(outcome, &Err("instance offline".to_string()));
        // The result is taken 30 ms after the write: Live's round trip on
        // the router's own clock.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        router.handle(applied);
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(VOLUME, 1, "instance offline")]
            }]
        );
        // The newer want went out as batch 2.
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(next, RouterMsg::Applied { batch: 2, .. }));
        router.handle(next);
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(VOLUME, 2, "instance offline")]
            }]
        );
        let written = records(&records_rx);
        assert_eq!(
            evs(&written),
            vec![
                "set", "batch", "set", "set", "applied", "ack", "batch", "applied", "ack"
            ]
        );
        assert_eq!(written[0]["key"], VOLUME);
        assert_eq!(written[0]["gap_ms"], Value::Null);
        assert_eq!(written[0]["dropped_old"], json!(false));
        assert_eq!(written[0]["unknown"], json!(false));
        assert_eq!(written[0]["delay_ms"], 50.0);
        assert_eq!(written[1]["batch"], 1);
        assert_eq!(written[1]["n"], 1);
        assert_eq!(
            written[1]["sent"],
            json!([{"key": VOLUME, "client": 1, "seq": 1, "value": 0.5}])
        );
        let gap = written[2]["gap_ms"].as_f64().unwrap();
        assert!((0.0..1_000.0).contains(&gap), "{gap}");
        assert_eq!(written[3]["dropped_old"], json!(true), "seq 2 twice");
        assert_eq!(written[4]["errors"], 1);
        assert_eq!(written[4]["n"], 1);
        assert_eq!(written[4]["batch"], 1);
        assert_eq!(
            written[4]["sent"],
            json!([{"key": VOLUME, "client": 1, "seq": 1}])
        );
        let rtt = written[4]["rtt_ms"].as_f64().unwrap();
        assert!((30.0..5_000.0).contains(&rtt), "{rtt}");
        assert_eq!(written[5]["seq"], 1);
        assert_eq!(written[5]["error"], "instance offline");
        assert_eq!(written[5]["peer"], "10.0.0.1");
        assert_eq!(written[5]["batch"], 1);
        assert_eq!(written[5]["rtt_ms"], rtt);
        assert_eq!(written[6]["sent"][0]["value"], 0.6);
        // A result for a batch no longer in flight changes nothing.
        router.handle(RouterMsg::Applied {
            instance: "band".into(),
            batch: 7,
            outcome: Ok(vec![]),
        });
        assert_eq!(outbox.take().unwrap(), vec![]);
        assert!(records(&records_rx).is_empty());
    }

    #[tokio::test]
    async fn a_disconnect_forgets_the_writes_and_a_detach_the_clients_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, _records) = offline_router(dir.path());
        attach(&mut router, 1);
        router.handle(set_msg(1, "band", 4, 0.5));
        router.handle(set_msg(1, "band", 5, 0.6));
        assert!(router.setters["band"].in_flight().is_some());
        assert_eq!(router.setters["band"].pending().len(), 1);
        router.handle(RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Disconnected,
        });
        assert!(router.setters["band"].in_flight().is_none());
        assert!(router.setters["band"].pending().is_empty());
        // The old batch's answer acks nothing.
        let late = rx.recv().await.unwrap();
        router.handle(late);
        assert_eq!(router.setters["band"].tracked(), 1);
        router.handle(RouterMsg::Detach { client: 1 });
        assert_eq!(router.setters["band"].tracked(), 0);
        assert!(!router.peers.contains_key(&1));
        assert!(!router.lan.contains_key(&1));
    }

    /// A `sets` item (#127): `band`'s `target` muted, sent at page time
    /// 1000.5.
    fn mute_item(target: &str, seq: u64) -> SetItem {
        SetItem {
            instance: "band".into(),
            target: target.into(),
            prop: "mute".into(),
            value: json!(true),
            seq,
            t: 1_000.5,
            is_final: true,
        }
    }

    #[tokio::test]
    async fn a_linked_mutes_sets_go_to_live_in_one_batch_and_each_is_acked() {
        // #127: three `set`s would go as two batches (the first alone, the
        // others after its result); a `sets` takes all three, then writes.
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, records_rx) = offline_router(dir.path());
        let outbox = attach(&mut router, 1);
        outbox.take();
        router.handle(RouterMsg::Sets {
            client: 1,
            sets: vec![
                mute_item("live_set tracks 1", 1),
                mute_item("live_set  tracks 2", 2),
                mute_item("live_set return_tracks 0", 3),
            ],
            hub_ms: 1_250.5,
            offset_ms: Some(200.0),
        });
        let written = records(&records_rx);
        assert_eq!(evs(&written), vec!["set", "set", "set", "batch"]);
        let keys = [
            "band|live_set return_tracks 0|mute",
            "band|live_set tracks 1|mute",
            "band|live_set tracks 2|mute",
        ];
        assert_eq!(written[1]["key"], keys[2]);
        assert_eq!(written[1]["seq"], 2);
        // Each set's own record: the page 200 ms behind, 50 ms on the way.
        assert_eq!(written[2]["delay_ms"], 50.0);
        assert_eq!(written[3]["n"], 3);
        let sent: Vec<(&str, u64)> = written[3]["sent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| (i["key"].as_str().unwrap(), i["seq"].as_u64().unwrap()))
            .collect();
        assert_eq!(sent, [(keys[0], 3), (keys[1], 1), (keys[2], 2)]);
        let applied = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the batch's result comes back")
            .unwrap();
        router.handle(applied);
        let mut acked: Vec<AckItem> = outbox
            .take()
            .unwrap()
            .into_iter()
            .flat_map(|m| match m {
                ServerMsg::Ack { items } => items,
                _ => Vec::new(),
            })
            .collect();
        acked.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(
            acked,
            vec![
                AckItem::failed(keys[0], 3, "instance offline"),
                AckItem::failed(keys[1], 1, "instance offline"),
                AckItem::failed(keys[2], 2, "instance offline"),
            ]
        );
    }

    #[tokio::test]
    async fn a_sets_of_more_than_32_writes_has_its_first_32_written_and_the_rest_acked_failed() {
        // #127 review M3: a `sets` takes at most 32 items (far above any
        // real link); those after them are acked failed at once, with why,
        // and the first 32 are written as one batch.
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, records_rx) = offline_router(dir.path());
        let outbox = attach(&mut router, 1);
        outbox.take();
        let sets: Vec<SetItem> = (0..34u64)
            .map(|i| mute_item(&format!("live_set tracks {i}"), i + 1))
            .collect();
        router.handle(RouterMsg::Sets {
            client: 1,
            sets,
            hub_ms: 1_250.5,
            offset_ms: Some(200.0),
        });
        let written = records(&records_rx);
        let mut want = vec!["ack"; 2];
        want.extend(["set"; 32]);
        want.push("batch");
        assert_eq!(evs(&written), want);
        let why = "more than 32 writes in one sets";
        let refused = [
            AckItem::failed("band|live_set tracks 32|mute", 33, why),
            AckItem::failed("band|live_set tracks 33|mute", 34, why),
        ];
        for (record, ack) in written.iter().zip(&refused) {
            assert_eq!(record["key"], ack.key);
            assert_eq!(record["seq"], ack.seq);
            assert_eq!(record["error"], why);
        }
        assert_eq!(written[2]["key"], "band|live_set tracks 0|mute");
        assert_eq!(written[33]["key"], "band|live_set tracks 31|mute");
        assert_eq!(written[34]["n"], 32);
        let mut sent: Vec<u64> = written[34]["sent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["seq"].as_u64().unwrap())
            .collect();
        sent.sort_unstable();
        assert_eq!(sent, (1..=32).collect::<Vec<u64>>());
        // The page hears the two refusals before any result of the batch.
        let mut acked: Vec<AckItem> = outbox
            .take()
            .unwrap()
            .into_iter()
            .flat_map(|m| match m {
                ServerMsg::Ack { items } => items,
                _ => Vec::new(),
            })
            .collect();
        acked.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(acked, refused);
    }

    #[test]
    fn live_health_goes_to_every_client_and_a_change_to_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let (events, rx) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let mut router = Router::new(
            BTreeMap::new(),
            HubState::default(),
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        );
        router
            .states
            .insert("band".into(), InstanceState::default());
        let a = attach(&mut router, 1);
        let b = attach(&mut router, 2);
        a.take();
        b.take();
        let busy = |changed: bool, tick_age_ms: f64| RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Busy {
                busy: true,
                changed,
                tick_age_ms,
                reason: changed.then(|| "no heartbeat for 301 ms".to_string()),
            },
        };
        router.handle(busy(true, 301.0));
        let link = |tick_age_ms: f64| ServerMsg::Link {
            instance: "band".into(),
            tick_age_ms,
            busy: true,
        };
        let state = ServerMsg::Instance {
            name: "band".into(),
            online: false,
            busy: true,
            set_name: String::new(),
        };
        for outbox in [&a, &b] {
            assert_eq!(outbox.take().unwrap(), vec![state.clone(), link(301.0)]);
        }
        assert!(router.states["band"].busy);
        let written = records(&rx);
        assert_eq!(evs(&written), vec!["link"]);
        assert_eq!(written[0]["busy"], json!(true));
        assert_eq!(written[0]["tick_age_ms"], 301.0);
        assert_eq!(written[0]["reason"], "no heartbeat for 301 ms");
        // While busy: a link only, no state change, no record.
        router.handle(busy(false, 560.0));
        assert_eq!(a.take().unwrap(), vec![link(560.0)]);
        assert!(records(&rx).is_empty());
    }

    pub(super) fn status(router: &mut Router) -> RouterStatus {
        let (reply, mut answer) = oneshot::channel();
        assert!(router.handle(RouterMsg::Status { reply }));
        answer.try_recv().expect("answered at once")
    }

    #[test]
    fn a_client_gets_the_layout_revision_only_once_one_is_served() {
        let dir = tempfile::tempdir().unwrap();
        let mut router = router(dir.path());
        let first = attach(&mut router, 1);
        assert_eq!(
            first.take().unwrap(),
            vec![
                ServerMsg::Reverb(router.reverb.rule.state()),
                ServerMsg::Autotune(router.autotune.state()),
                ServerMsg::Hub {
                    key: HUB_STAGE_AUT.into(),
                    value: json!(false)
                }
            ],
            "no layout yet: no revision (REVERB's state, #130, and AUTOTUNE's, \
             #104, are replies)"
        );
        router.handle(RouterMsg::Layout {
            rev: 1,
            stage: None,
            targets: BTreeMap::new(),
            strips: Vec::new(),
            reverb: Vec::new(),
            fader_leaders: Vec::new(),
            autotune: Vec::new(),
        });
        assert_eq!(first.take().unwrap(), vec![ServerMsg::Layout { rev: 1 }]);
        let second = attach(&mut router, 2);
        assert!(
            second
                .take()
                .unwrap()
                .contains(&ServerMsg::Layout { rev: 1 })
        );
        let now = status(&mut router);
        assert_eq!(now.clients, 2);
        assert!(now.unresolved.is_empty());
        router.handle(RouterMsg::Detach { client: 1 });
        assert_eq!(status(&mut router).clients, 1);
        assert!(!router.handle(RouterMsg::Stop));
    }

    #[test]
    fn an_older_layout_revision_never_replaces_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut router = router(dir.path());
        let first = attach(&mut router, 1);
        first.take();
        let layout = |rev| RouterMsg::Layout {
            rev,
            stage: None,
            targets: BTreeMap::new(),
            strips: Vec::new(),
            reverb: Vec::new(),
            fader_leaders: Vec::new(),
            autotune: Vec::new(),
        };
        router.handle(layout(2));
        router.handle(layout(1));
        router.handle(layout(2));
        assert_eq!(router.layout_rev, 2);
        assert_eq!(first.take().unwrap(), vec![ServerMsg::Layout { rev: 2 }]);
    }

    #[tokio::test]
    async fn the_strips_tracks_are_followed_for_their_groups() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        let layout = |rev, strips: Vec<(String, String)>| RouterMsg::Layout {
            rev,
            stage: None,
            targets: BTreeMap::new(),
            strips,
            reverb: Vec::new(),
            fader_leaders: Vec::new(),
            autotune: Vec::new(),
        };
        router.handle(layout(1, vec![("band".into(), "Keys 1".into())]));
        assert_eq!(
            router.subs.subscriptions("band"),
            3,
            "the keeper's watches of the band's tracks and visible tracks (#58), \
             and the rules' transport (#104)"
        );
        let now = status(&mut router);
        assert_eq!(now.subscriptions["band"], 0, "the hub's own read");
        assert_eq!(now.unfolded, BTreeMap::new(), "offline: no group known");
        // A new layout without it lets it go; an instance the hub does not
        // know is refused (logged) and kept nowhere.
        router.handle(layout(2, vec![("nowhere".into(), "X".into())]));
        assert_eq!(router.subs.subscriptions("band"), 1, "the rules' transport");
        assert_eq!(
            cached_value(&Cached::Value {
                value: json!(1),
                display: None
            }),
            Some(&json!(1))
        );
        assert_eq!(cached_value(&Cached::Error("gone".into())), None);
    }

    #[tokio::test]
    async fn a_failed_read_of_the_strips_groups_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, _records) = offline_router(dir.path());
        router.handle(RouterMsg::Layout {
            rev: 1,
            stage: None,
            targets: BTreeMap::new(),
            strips: vec![("band".into(), "Keys 1".into())],
            reverb: Vec::new(),
            fader_leaders: Vec::new(),
            autotune: Vec::new(),
        });
        // The read goes to an offline instance: its failure comes back…
        let failed = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the read's answer")
            .unwrap();
        let RouterMsg::UnfoldRead {
            ref instance,
            ref outcome,
            ..
        } = failed
        else {
            panic!("the read's answer")
        };
        assert_eq!(
            (instance.as_str(), outcome),
            ("band", &Err("instance offline".to_string()))
        );
        router.handle(failed);
        // …and the read is due again after RETRY_MS.
        let started = std::time::Instant::now();
        let retry = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a retry")
            .unwrap();
        assert!(
            matches!(retry, RouterMsg::UnfoldRetry { ref instance } if instance == "band"),
            "a retry of the band"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(unfold::RETRY_MS - 100));
        // The retry reads again: its answer comes back too.
        router.handle(retry);
        let again = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the second read's answer")
            .unwrap();
        assert!(matches!(again, RouterMsg::UnfoldRead { .. }));
    }

    /// The next message the router's own tasks send that `want` takes.
    pub(super) async fn next_of(
        rx: &mut mpsc::UnboundedReceiver<RouterMsg>,
        what: &str,
        want: impl Fn(&RouterMsg) -> bool,
    ) -> RouterMsg {
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap_or_else(|_| panic!("no {what}"))
                .expect("the channel is open");
            if want(&msg) {
                return msg;
            }
        }
    }

    #[tokio::test]
    async fn a_connect_and_a_failed_rename_read_the_tuners_again() {
        // #68 PR C: the script's object refs start over with each
        // connection; a rename names a Tuner by its ref.
        let dir = tempfile::tempdir().unwrap();
        let (router, mut rx, _records) = offline_router(dir.path());
        let store = Arc::new(crate::layout::store_in(
            dir.path(),
            std::path::Path::new("layout.json"),
            vec!["band".into()],
        ));
        let mut router = router.with_layout(store);
        // The keeper knows the band's tracks (its watch's value; the read
        // that value asks for is left out).
        let tracks = fohmixer_proto::client::hub_key("band", "live_set", markers::TRACKS, false);
        let _ = router
            .markers
            .value(&tracks, Some(&json!([{"$ref": "t0", "name": "Bass #"}])));
        // A connect reads the Tuners again (offline here: the read fails).
        router.handle(RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Connected(crate::live::ConnectInfo {
                instance: "band".into(),
                set_name: "set".into(),
                script_version: "0".into(),
                live_version: "12".into(),
            }),
        });
        let read = next_of(&mut rx, "read", |m| {
            matches!(m, RouterMsg::MarkersRead { .. })
        })
        .await;
        let RouterMsg::MarkersRead { instance, step, .. } = read else {
            unreachable!()
        };
        assert_eq!((instance.as_str(), step), ("band", markers::Step::Devices));
        // A rename that fails (an unknown ref, here an offline instance)
        // asks for a read again.
        router.markers_apply(vec![markers::Action::Rename {
            instance: "band".into(),
            target: json!({"$ref": "gone"}),
            name: r#""Bass" +G:A:1"#.into(),
        }]);
        let retry = next_of(&mut rx, "retry", |m| {
            matches!(m, RouterMsg::MarkersRetry { .. })
        })
        .await;
        let RouterMsg::MarkersRetry { instance } = retry else {
            unreachable!()
        };
        assert_eq!(instance, "band");
    }

    #[tokio::test]
    async fn the_markers_are_followed_and_a_failed_read_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let (router, mut rx, _records) = offline_router(dir.path());
        let store = Arc::new(crate::layout::store_in(
            dir.path(),
            std::path::Path::new("layout.json"),
            vec!["band".into()],
        ));
        let mut router = router.with_layout(store);
        assert_eq!(
            router.subs.subscriptions("band"),
            3,
            "the keeper's watches of the band's tracks and return tracks (#68), \
             and the rules' transport (#104)"
        );
        assert_eq!(
            status(&mut router).subscriptions["band"],
            0,
            "the hub's own"
        );
        // A read (a retry starts one) goes to an offline instance: its
        // failure comes back…
        router.handle(RouterMsg::MarkersRetry {
            instance: "band".into(),
        });
        let failed = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the read's answer")
            .unwrap();
        let RouterMsg::MarkersRead {
            ref instance,
            ref outcome,
            step,
            ..
        } = failed
        else {
            panic!("the read's answer")
        };
        assert_eq!(
            (instance.as_str(), outcome, step),
            (
                "band",
                &Err("instance offline".to_string()),
                markers::Step::Devices
            )
        );
        router.handle(failed);
        // …and the read is due again after RETRY_MS.
        let started = std::time::Instant::now();
        let retry = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a retry")
            .unwrap();
        assert!(
            matches!(retry, RouterMsg::MarkersRetry { ref instance } if instance == "band"),
            "a retry of the band"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(retries::RETRY_MS - 100));
    }

    #[tokio::test(start_paused = true)]
    async fn a_markers_retry_due_while_live_is_busy_waits_and_reads_nothing() {
        // PR #145 review M1: a stalled Live gets no read queued behind the
        // stall; the retry waits another RETRY_MS and reads once Live is
        // ready.
        let dir = tempfile::tempdir().unwrap();
        let (router, mut rx, _records) = offline_router(dir.path());
        let store = Arc::new(crate::layout::store_in(
            dir.path(),
            std::path::Path::new("layout.json"),
            vec!["band".into()],
        ));
        let mut router = router.with_layout(store);
        router.states.get_mut("band").unwrap().busy = true;
        router.handle(RouterMsg::MarkersRetry {
            instance: "band".into(),
        });
        let started = tokio::time::Instant::now();
        let next = next_of(&mut rx, "a message", |_| true).await;
        assert!(
            matches!(next, RouterMsg::MarkersRetry { ref instance } if instance == "band"),
            "waited again, no read"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(retries::RETRY_MS));
        router.states.get_mut("band").unwrap().busy = false;
        router.handle(next);
        let read = next_of(&mut rx, "a message", |_| true).await;
        assert!(matches!(read, RouterMsg::MarkersRead { .. }), "a read");
    }

    #[tokio::test]
    async fn new_markers_are_served_as_a_new_layout_revision_only_when_it_changes() {
        use fohmixer_proto::layout::{Control, Section};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("layout.json"),
            serde_json::to_vec(&json!({
                "schema": 2,
                "default_page": "p",
                "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
                    {"kind": "group", "id": "g", "tags": "A"}]}]}]
            }))
            .unwrap(),
        )
        .unwrap();
        let store = Arc::new(crate::layout::store_in(
            dir.path(),
            std::path::Path::new("layout.json"),
            vec!["band".into()],
        ));
        assert_eq!(store.poll(), Some(1));
        let (router, mut rx, _records) = offline_router(dir.path());
        let mut router = router.with_layout(Arc::clone(&store));
        // The next quiet-time message (the reads' answers go by).
        async fn settled(rx: &mut mpsc::UnboundedReceiver<RouterMsg>) -> RouterMsg {
            loop {
                let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                    .await
                    .expect("a message")
                    .expect("the channel is open");
                if matches!(msg, RouterMsg::MarkersSettle { .. }) {
                    return msg;
                }
            }
        }
        let found = |name: &str| {
            vec![fohmixer_proto::markers::Found {
                instance: "band".into(),
                kind: fohmixer_proto::markers::TrackKind::Track,
                index: 3,
                name: name.into(),
                tuners: 1,
            }]
        };
        router.markers_found(found(r#""Vox" +G:A"#));
        // Nothing before the quiet time.
        assert_eq!(router.layout_rev, 0);
        let started = std::time::Instant::now();
        let settle = settled(&mut rx).await;
        assert!(started.elapsed() >= std::time::Duration::from_millis(markers::SETTLE_MS - 50));
        router.handle(settle);
        assert_eq!(router.layout_rev, 2);
        assert_eq!(store.current().0, 2);
        assert_eq!(store.markers_status().found, 1);
        // The same markers: the same composition, no revision (I10).
        router.markers_found(found(r#""Vox" +G:A"#));
        let settle = settled(&mut rx).await;
        router.handle(settle);
        assert_eq!(router.layout_rev, 2);
        // Two changes in a row: only the latest is served, once.
        router.markers_found(found(r#""One" +G:A"#));
        router.markers_found(found(r#""Two" +G:A"#));
        let first = settled(&mut rx).await;
        let second = settled(&mut rx).await;
        router.handle(first);
        assert_eq!(router.layout_rev, 2, "a newer change came");
        router.handle(second);
        assert_eq!(router.layout_rev, 3);
        let served = store.current().1.unwrap();
        let Section::Group(group) = &served.pages[0].rows[0].sections[0] else {
            panic!("the group")
        };
        let Control::Strip(strip) = &group.controls[0] else {
            panic!("a strip")
        };
        assert_eq!(strip.label.as_deref(), Some("Two"));
        // A router with no store keeps its layout as it is.
        let (mut bare, mut bare_rx, _records) = offline_router(dir.path());
        bare.markers_found(found(r#""Other" +G:A"#));
        bare.handle(RouterMsg::MarkersSettle { generation: 0 });
        assert_eq!(bare.layout_rev, 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), bare_rx.recv())
                .await
                .is_err(),
            "no quiet time without a store"
        );
        assert_eq!(store.current().0, 3);
    }

    #[test]
    fn a_stage_write_failure_says_why() {
        assert_eq!(write_failure(&Ok(vec![json!({"ok": true})])), None);
        assert_eq!(
            write_failure(&Ok(vec![json!({"ok": false})])),
            Some(r#"[{"ok":false}]"#.to_string())
        );
        assert_eq!(write_failure(&Ok(vec![])), Some("[]".to_string()));
        assert_eq!(
            write_failure(&Err(LiveError::Offline)),
            Some("instance offline".to_string())
        );
    }

    #[test]
    fn subbed_carries_the_cached_state() {
        assert_eq!(
            subbed("k", None),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: None,
                display: None,
                error: None
            }
        );
        assert_eq!(
            subbed(
                "k",
                Some(&Cached::Value {
                    value: json!(0.5),
                    display: Some("-6.0 dB".into())
                })
            ),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: Some(json!(0.5)),
                display: Some("-6.0 dB".into()),
                error: None
            }
        );
        assert_eq!(
            subbed("k", Some(&Cached::Error("gone".into()))),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: None,
                display: None,
                error: Some("gone".into())
            }
        );
    }
}
