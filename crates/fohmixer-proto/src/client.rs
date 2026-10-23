//! The client protocol, hub ⇄ UI (S3 design note §3): JSON text frames on
//! `/ws?token=<jwt>&proto=2`, plus the JSON bodies of the hub's API.
//!
//! The hub never reinterprets Live's values: a `cmd` is the FohMixer script's
//! own envelope (S2 design note §3.10) with the instance added, and values
//! and display strings are Live's, as the script reported them.
//!
//! Protocol 2 (#43, `docs/superpowers/specs/2026-10-03-robust-link-design.md`
//! §2): a control's write is a `set` with the page's sequence number and
//! clock, answered by a coalesced `ack`; `ping` / `pong` carry the times of
//! the exchange; `link` reports Live's main-thread health. `cmd` stays for
//! reads and multi-command batches.
//!
//! The Pro-Q 4 screen (#71 PR E, F28): `eq_list`, `eq_open`, `eq_area`
//! (PR G), `eq_input`, `eq_close`, `eq_frame_ack` (PR I), `eq_undo` (PR
//! P2a) and `eq`, `eq_list`, `eq_locks`, `eq_undo` (their pieces and the
//! binary frames the hub sends an open editor's holder: [`crate::eq`]).

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::eq::{Area, EqItem, EqLock, EqState, Part, Touch};
use crate::layout::{Binding, LinkId};
use crate::path::canonical_target;
use crate::songmap::{Counts, ProblemKind, SectionsState, Span};

/// The client protocol this build speaks.
pub const UI_PROTO: u32 = 2;
/// The oldest client protocol this build serves (a page of protocol 1
/// reloads through the handshake).
pub const MIN_CLIENT_PROTO: u32 = 2;
/// WebSocket close code telling a client with another protocol to reload.
pub const CLOSE_RELOAD: u16 = 4001;
/// The `name` of the `<meta>` in which the hub's `index.html` carries the
/// revision of the page policy it was served with (#90): the page compares
/// it with the hello's `policy`.
pub const POLICY_META: &str = "fohmixer-policy";
/// The hub value of the STAGE AUT flag (spec §2.4, F15).
pub const HUB_STAGE_AUT: &str = "stage_aut";
/// The hub value of REVERB AUT (#130, D18, F30; the hub's rule, kept in
/// `hub-state.json`): a bool.
pub const HUB_REVERB_AUT: &str = "reverb_aut";
/// The hub value a REVERB tap asks for (#130, D18): the
/// reverb faders lowered (`true`) or at the song level, as the button
/// shows `!low`, so two pages' simultaneous taps agree.
pub const HUB_REVERB_LOW: &str = "reverb_low";
/// The hub value of AUTOTUNE AUT (#104, D19; the hub's rule, kept in
/// `hub-state.json`): a bool.
pub const HUB_AUTOTUNE_AUT: &str = "autotune_aut";
/// The hub value an AUTOTUNE tap asks for (#104): every target on (`true`)
/// or off, as the button shows it, so two pages' taps agree.
pub const HUB_AUTOTUNE: &str = "autotune";
/// The hub value of a fader link's switch (#130, D18): this prefix, the
/// leader's Live set, `|`, its id (`fader_link|band|REV`; a bool).
pub const FADER_LINK_PREFIX: &str = "fader_link|";

/// The hub key of `link`'s switch.
pub fn fader_link_key(link: &LinkId) -> String {
    format!("{FADER_LINK_PREFIX}{}|{}", link.instance, link.id)
}

/// The link a hub key names; none for any other key.
pub fn fader_link_of(key: &str) -> Option<LinkId> {
    let (instance, id) = key.strip_prefix(FADER_LINK_PREFIX)?.rsplit_once('|')?;
    (!instance.is_empty() && !id.is_empty()).then(|| LinkId {
        instance: instance.to_string(),
        id: id.to_string(),
    })
}

/// Whether a client speaking `proto` is served.
pub fn proto_ok(proto: u32) -> bool {
    (MIN_CLIENT_PROTO..=UI_PROTO).contains(&proto)
}

/// A field that is present, even as `null` (`Some(Value::Null)`); a missing
/// field is `None` through `#[serde(default)]`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// One command of the script's envelope: a target, an operation name and
/// its arguments, forwarded unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveCommand {
    /// A LOM path (string) or `{"$ref": id}`.
    pub target: Value,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// Client → hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// A command batch for one instance; answered by `result` with the same id.
    Cmd {
        id: String,
        instance: String,
        commands: Vec<LiveCommand>,
    },
    /// Subscribe to a property; answered by `subbed`. Deduplicated in the hub.
    Sub {
        instance: String,
        target: String,
        prop: String,
        #[serde(default)]
        display: bool,
    },
    /// End a subscription (the `sub` key from `subbed`).
    Unsub { sub: String },
    /// Set a hub value: `stage_aut`; since #130 a fader link's switch
    /// (`fader_link|<instance>|<id>`, [`fader_link_key`]) and REVERB's
    /// `reverb_aut` and `reverb_low`; since #104 AUTOTUNE's `autotune_aut`
    /// and `autotune`.
    SetHub { key: String, value: Value },
    /// A control's write (#43): the latest value the engineer set on one
    /// property, answered by an `ack` item of its key. `seq` is the page's,
    /// strictly increasing over all its sets; `t` is the page's clock
    /// (`performance.timeOrigin + performance.now()`, ms); `final` marks a
    /// release, a toggle or a tap.
    Set {
        instance: String,
        target: String,
        prop: String,
        value: Value,
        seq: u64,
        t: f64,
        #[serde(rename = "final")]
        is_final: bool,
    },
    /// Writes Live applies together (#127, a linked mute's tracks): each
    /// item a `set` of its own key, `seq` and `ack`. The hub takes them all
    /// before it writes a batch, so one instance's go to Live in one batch;
    /// at most [`MAX_SETS`] of them (the items after those are acked
    /// failed).
    Sets { sets: Vec<SetItem> },
    /// A liveness probe, answered by `pong` (the client's watchdog: a
    /// socket that stays silent is half-open and is replaced): its number,
    /// the page's clock, and the round trip (ms) of the latest pong on this
    /// socket with the number of the ping it measured (`rtt_n`), so the hub
    /// pairs it with that ping's own time and arrival; none before the
    /// socket's first pong.
    Ping {
        n: u32,
        t: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_n: Option<u32>,
    },
    /// The page's own events for the hub's event log (#43), as it recorded
    /// them (PR A: each finished dropout, `{"ev": "dropout", "t", "ms",
    /// "socket_lost", "rtts"}`).
    Trace { events: Vec<Value> },
    /// The page opened (`on`) or closed the Stream Deck tab (#52): only a
    /// client viewing it gets the keys' images (`deck_keys`).
    DeckView { on: bool },
    /// The page went idle (`on`) or woke (#51): while idle the hub sends
    /// this client's meter values at most once per `meter_ms`, the louder
    /// value of each meter kept; `meter_ms` 0 pauses them until the page
    /// wakes (a hidden page: a locked iPad). Absent: the default period.
    /// [`meter_pace`] reads it. Every other message goes at once.
    Idle {
        on: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meter_ms: Option<u32>,
    },
    /// A Stream Deck key's press (#52): `down` at the touch, up at the
    /// release. `seq` is the page's own counter of presses (not `set`'s);
    /// `t` the page's clock as for `set`; an up carries the hold the page
    /// measured and why it came (`up`, `cancel`, `lost`, `hidden`, `tab`).
    /// Answered by `deck_ack`; never queued, never resent.
    DeckPress {
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
    /// The Pro-Q 4 instances of a strip's track (#71 PR E): `binding` is the
    /// strip's (its track by name or by index); answered by `eq_list`. Read
    /// when asked (a detail opening), never followed.
    EqList { binding: Binding },
    /// Opens the editor of the Pro-Q 4 at `path` (an `eq_list` item's) on
    /// `instance` for this client, closing the one it holds first; answered
    /// by `eq` (`opening`, then `open` or `closed` with why: another client
    /// holds it, `locked`). `area` (PR G): the page's picture area, whose
    /// aspect the hub gives the editor before it answers `open`; none keeps
    /// the editor's own size.
    EqOpen {
        instance: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<Area>,
    },
    /// The page's picture area changed while it holds an open editor (PR G:
    /// a phone turned, a window resized; the page waits for it to settle):
    /// the hub gives the editor that aspect. Its frames then carry the new
    /// picture.
    EqArea { w: f64, h: f64 },
    /// A finger on this client's open editor: `x`, `y` in the picture's
    /// pixels (the hub clamps them to the picture). `n` (PR J): the page's
    /// number of this input in its open session, from 1, every phase; the
    /// frames say the last one they show. `seen`, on a down: the `shown` of
    /// the picture the finger landed on (the hub drops a down whose picture
    /// did not show the page's earlier inputs).
    EqInput {
        touch: Touch,
        x: f64,
        y: f64,
        n: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seen: Option<u32>,
    },
    /// The part of the picture the page shows at 2× and more, with a margin
    /// (PR L): the hub sends only that part of its frames of that size.
    /// None: the whole picture.
    EqView {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<Part>,
    },
    /// This client leaves its editor: the hub closes it (the close guard
    /// first) and releases the lock.
    EqClose,
    /// The page got the binary frame `seq` of its open editor's `session`
    /// (PR I): it acks every frame of that session up to `seq`. The hub
    /// sends that page's next frame only after this ack, or once its wait
    /// for it ran out (one frame in flight per page, two after a lost ack,
    /// so its pongs, acks and values never wait behind a backlog of
    /// pictures).
    EqFrameAck { session: u32, seq: u32 },
    /// Pro-Q 4's own undo of this client's open editor (PR P2a): the hub
    /// reads the ↶ in a fresh picture and taps it when it reads lit (one
    /// change undone); answered by `eq_undo`, to this client only. `n`: the
    /// page's next input number of its open session (`eq_input`'s
    /// sequence), so a picture's `shown` covers the undo and a down on a
    /// picture from before it is refused as stale.
    EqUndo { n: u32 },
}

/// One write's outcome (#43, an `ack` item): Live ran it (`value`: what
/// Live reported, when the result has one), Live refused it (`error`), or
/// another client's newer set replaced it before it was written
/// (`superseded`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckItem {
    /// The write's key ([`set_key`]).
    pub key: String,
    /// The page's sequence number of the `set` it answers.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub superseded: bool,
}

impl AckItem {
    /// Live ran set `seq` of `key`; `value` is what its result reported.
    pub fn applied(key: &str, seq: u64, value: Option<Value>) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value,
            error: None,
            superseded: false,
        }
    }

    /// Set `seq` of `key` failed: Live refused it, it timed out, or the
    /// instance is offline.
    pub fn failed(key: &str, seq: u64, error: &str) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value: None,
            error: Some(error.to_string()),
            superseded: false,
        }
    }

    /// Set `seq` of `key` was replaced by another client's newer set before
    /// it was written.
    pub fn superseded(key: &str, seq: u64) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value: None,
            error: None,
            superseded: true,
        }
    }
}

/// The state of one subscription: Live's value (and display string), or why
/// it has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueItem {
    pub sub: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ValueItem {
    /// Live's value (and display string) for `sub`.
    pub fn value(sub: &str, value: Value, display: Option<String>) -> Self {
        Self {
            sub: sub.to_string(),
            value: Some(value),
            display,
            error: None,
        }
    }

    /// `sub` has no value: its binding does not resolve, or the object went.
    pub fn error(sub: &str, error: &str) -> Self {
        Self {
            sub: sub.to_string(),
            value: None,
            display: None,
            error: Some(error.to_string()),
        }
    }
}

/// One Stream Deck key as Companion drew it (#52): its image (a `data:`
/// URL), its colour (`#rrggbb`) and whether Companion shows it pressed. A
/// key Companion has not drawn yet has neither image nor colour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckKey {
    pub key: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub pressed: bool,
}

/// Hub → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// The first message of every connection.
    Hello {
        proto: u32,
        build: String,
        min_client_proto: u32,
        /// The revision of the page policy (Content-Security-Policy) the hub
        /// sends with its pages now (#90): a page that loaded with another
        /// reloads. None from a hub before it; a page before it ignores it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        policy: Option<String>,
    },
    /// The script's result slots for a `cmd`, in command order.
    Result { id: String, data: Vec<Value> },
    /// A subscription's hub key, with its value when the hub has one.
    Subbed {
        sub: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The latest value of each changed subscription (coalesced per client).
    Values { items: Vec<ValueItem> },
    /// An instance's connection state, sent on change.
    Instance {
        name: String,
        online: bool,
        busy: bool,
        set_name: String,
    },
    /// A hub value (STAGE AUT).
    Hub { key: String, value: Value },
    /// The layout changed: GET `/api/layout` for revision `rev`.
    Layout { rev: u64 },
    /// The outcome of this client's writes (#43), coalesced per client: per
    /// key the item of the highest `seq`.
    Ack { items: Vec<AckItem> },
    /// The answer to `ping`: its number and page time echoed, and the hub's
    /// clock when it answered (`h`, UTC ms).
    Pong { n: u32, t: f64, h: f64 },
    /// Live's main-thread health on an instance (#43): on every busy change
    /// and at most 4 times a second while busy. `tick_age_ms` is the age of
    /// Live's last main-thread tick as the hub knows it.
    Link {
        instance: String,
        tick_age_ms: f64,
        busy: bool,
    },
    /// A request the hub could not serve (`id` of the `cmd`, if any).
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// The Stream Deck (#52): sent on attach and whenever Companion's link
    /// goes up or down, only by a hub with a `[companion]` table (a page
    /// without it shows no tab).
    Deck {
        online: bool,
        columns: u32,
        rows: u32,
        title: String,
    },
    /// Keys' new states (#52), coalesced per key, only to a client viewing
    /// the tab.
    DeckKeys { items: Vec<DeckKey> },
    /// The answer to a `deck_press` (#52): Companion's OK (`rtt_ms`: its
    /// round trip from the hub) or why it was not done (`offline`, Companion's
    /// own error); a press the hub took without forwarding it (the key already
    /// held, or an up from a page that does not hold it) is `ok` at once.
    DeckAck {
        seq: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<f64>,
    },
    /// The answer to `eq_list` (#71 PR E): the strip's binding as asked, the
    /// Pro-Q 4 instances found on its track (on it and in its racks' chains),
    /// or why none could be read (`error`: the instance offline, the track
    /// not found, the screen off).
    EqList {
        binding: Binding,
        items: Vec<EqItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Every held editor, to every client, at once on each change (a held
    /// one shows locked on every other device).
    EqLocks { items: Vec<EqLock> },
    /// This client's editor: `opening`; `open` with the picture's `session`
    /// (its binary frames carry it) and size; `closed` with `reason` (and,
    /// when another client holds it, `since`: the hub's UTC ms).
    Eq {
        instance: String,
        path: String,
        state: EqState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<f64>,
    },
    /// The answer to this client's `eq_undo` (PR P2a): its editor's
    /// `session` (0: it holds none), the undo's input number `n`, and
    /// `result`: `done` (Pro-Q's ↶ read lit and was tapped), `nothing` (it
    /// read grey: nothing to undo, no tap; [`crate::eq::UNDO_NOTHING`]), or
    /// why not (no tap; a [`crate::eq::reason`] such as `UNDO_BUSY`,
    /// `UNDO_SOON`, `UNDO_MISSING`).
    EqUndo {
        session: u32,
        n: u32,
        result: String,
    },
    /// AbleSet's own page behind the hub (#90 B3): sent on attach and on
    /// every change of the hub's link to AbleSet, only by a hub with
    /// `[ableset]` (a page without it shows no AbleSet tab). `name` is
    /// AbleSet's name, `parent` the `[tls]` name (the page frames AbleSet
    /// only when it was opened by that name), `online` the link (`null`:
    /// not known yet, before the hub's first read decides; #90 B3 review
    /// 4), `lan_port` AbleSet's own port on the PC (the emergency link).
    /// `lan` is this page's own: its socket came over the LAN (the Access
    /// check's class, as its socket lines name it), so a failed probe is
    /// no sign-in there (#90 PR C review M2); an older hub names none,
    /// which reads as the tunnel's, as before.
    Ableset {
        title: String,
        name: String,
        parent: String,
        online: Option<bool>,
        lan_port: u16,
        #[serde(default)]
        lan: bool,
    },
    /// REVERB (#130, D18): REVERB AUT, whether the reverb faders are
    /// lowered and why, the followed transport, the stop's wait, and each
    /// fader's return level and hand; at attach and on every change.
    Reverb(ReverbState),
    /// AUTOTUNE (#104, D19): AUTOTUNE AUT, the first target's Live set's
    /// transport and each target's state; at attach and on every change.
    Autotune(AutotuneState),
    /// A message of a newer hub that this page does not know (#90 B3): any
    /// other `type` reads as this, and the page ignores it, so a new
    /// message never breaks an older page.
    #[serde(other)]
    Unknown,
}

/// The most writes one [`ClientMsg::Sets`] carries (#127 review M3): far
/// above any real link; the hub acks the items after them failed.
pub const MAX_SETS: usize = 32;

/// One write of a [`ClientMsg::Sets`] (#127): a `set`'s fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetItem {
    pub instance: String,
    pub target: String,
    pub prop: String,
    pub value: Value,
    pub seq: u64,
    pub t: f64,
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// Why the reverb faders are lowered (#130, D18): a tap of REVERB, or
/// REVERB AUT at a stop of Live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Low {
    Tap,
    Stop,
}

/// One reverb fader in [`ReverbState`] (#130, D18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReverbFader {
    /// Its volume's write key ([`set_key`]): a page's `set` of it is the
    /// engineer's hand.
    pub key: String,
    /// Its strip's label (the rule knows a fader by it across layouts).
    pub label: String,
    /// The level it returns to, while the rule holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<f64>,
    /// The engineer moved it while the rule held it: let go (`RUKA`) until
    /// the next stop.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hand: bool,
}

/// REVERB's state, told to every page (#130, D18, [`ServerMsg::Reverb`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReverbState {
    /// REVERB AUT is on.
    pub aut: bool,
    /// The reverb faders are lowered, and why; none at the song level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<Low>,
    /// The followed transport (the first reverb fader's Live); none before
    /// it is heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing: Option<bool>,
    /// A stop's 2 s wait runs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting: bool,
    /// The reverb faders, in Live's order.
    pub faders: Vec<ReverbFader>,
}

/// REVERB in `GET /api/status` (#130, D18): REVERB AUT, why the faders
/// are lowered (none at the song level), how many there are, the rule's
/// writes and the faders the hand took.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverbStatus {
    pub aut: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<Low>,
    pub faders: usize,
    pub writes: u64,
    pub hands: u64,
}

/// Where one AUTOTUNE target stands (#104): its rack's first macro at its
/// on value (`on`), at any other (`off`), not found yet or no value heard
/// yet (`waiting`), or no rack to switch (`missing`: none, two, or one
/// without a first macro).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    On,
    Off,
    Waiting,
    Missing,
}

/// One AUTOTUNE target in [`AutotuneState`] (#104): its label and state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutotuneTargetState {
    pub label: String,
    pub state: TargetState,
}

/// AUTOTUNE's state, told to every page (#104, D19,
/// [`ServerMsg::Autotune`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutotuneState {
    /// AUTOTUNE AUT is on.
    pub aut: bool,
    /// The first target's Live set's transport; none before it is heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing: Option<bool>,
    /// The targets, in Live's order.
    pub targets: Vec<AutotuneTargetState>,
}

/// One AUTOTUNE target in `GET /api/status` (#104): its Live set, track
/// (LOM target) and label, its on value, its state, the path of the macro
/// the hub found, or why it found none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutotuneTargetStatus {
    pub instance: String,
    pub track: String,
    pub label: String,
    pub on: u8,
    pub state: TargetState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// AUTOTUNE in `GET /api/status` (#104, D19): AUTOTUNE AUT, the rule's
/// writes, the taps, and each target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutotuneStatus {
    pub aut: bool,
    pub writes: u64,
    pub taps: u64,
    pub targets: Vec<AutotuneTargetStatus>,
}

/// One Live set's song map in `GET /api/status` (#140): its counts, which
/// Sections track it read, when it was read (hub UTC ms) and whether it is
/// stale (the set went offline since: the next read replaces it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongMapStatus {
    pub instance: String,
    pub songs: usize,
    pub keyed: usize,
    pub changes: usize,
    pub problems: usize,
    pub sections: SectionsState,
    pub at_ms: u64,
    pub stale: bool,
}

/// One problem of a song map in `GET /api/songmap` (#140): what, which
/// song, where (beats), and in words (English).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemReport {
    pub kind: ProblemKind,
    pub song: Option<u16>,
    pub time: Option<f64>,
    pub text: String,
}

/// `GET /api/songmap?instance=<name>` (#140): a Live set's newest song map,
/// its read's number and time (hub UTC ms), whether it is stale, which
/// Sections track it read, its counts, every span and every problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SongMapReport {
    pub instance: String,
    pub read_no: u64,
    pub at_ms: u64,
    pub stale: bool,
    pub sections: SectionsState,
    pub counts: Counts,
    pub spans: Vec<Span>,
    pub problems: Vec<ProblemReport>,
}

/// A fader link's switch in `GET /api/status` (#130).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaderLinkStatus {
    pub instance: String,
    pub id: String,
    pub on: bool,
}

impl SetItem {
    /// The write a `set` message carries; none for any other message.
    pub fn of(msg: ClientMsg) -> Option<Self> {
        match msg {
            ClientMsg::Set {
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
            } => Some(Self {
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
            }),
            _ => None,
        }
    }
}

/// The hub key of a subscription: `instance|target|prop|display`, with the
/// target in canonical form (whitespace between steps collapsed; names are
/// kept as they are).
pub fn hub_key(instance: &str, target: &str, prop: &str, display: bool) -> String {
    format!("{}|{display}", set_key(instance, target, prop))
}

/// The key of a write (#43, `set` / `ack`): `instance|target|prop`, the hub
/// key without `display`, the target in canonical form.
pub fn set_key(instance: &str, target: &str, prop: &str) -> String {
    format!("{instance}|{}|{prop}", canonical_target(target))
}

/// The write key ([`set_key`]) of a hub key ([`hub_key`]): the hub key
/// without its `display` field (#43: a page closes a write by its
/// subscription's value).
pub fn write_key_of(hub_key: &str) -> &str {
    hub_key
        .rsplit_once('|')
        .map_or(hub_key, |(key, _display)| key)
}

/// The Live properties of a track's meters (spec X2): the one list the page
/// subscribes from and the hub paces (#51).
pub const METER_PROPS: [&str; 3] = [
    "output_meter_level",
    "output_meter_left",
    "output_meter_right",
];

/// Whether a hub key ([`hub_key`]) is a meter's (#51): its property, the
/// second field from the right (`instance|target|prop|display`), is one of
/// [`METER_PROPS`], so a `|` inside a track name in the target cannot fool
/// it.
pub fn is_meter_key(hub_key: &str) -> bool {
    hub_key
        .rsplit('|')
        .nth(1)
        .is_some_and(|prop| METER_PROPS.contains(&prop))
}

/// An idle page's meter period when its `idle` names none (#51), ms.
pub const IDLE_METER_MS: u32 = 250;
/// The shortest meter period an idle page gets, ms.
pub const IDLE_METER_MIN_MS: u32 = 100;
/// The longest meter period an idle page gets, ms.
pub const IDLE_METER_MAX_MS: u32 = 2000;

/// How the hub sends one client's meter values (#51).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pace {
    /// As Live flushes them: an active page.
    #[default]
    Full,
    /// At most once per period, the louder value of each meter kept: an
    /// idle page.
    Every(Duration),
    /// None until the page wakes: a hidden page (a locked iPad).
    Paused,
}

/// The meter pace of an `idle` message (#51): awake (`on` false) is full
/// rate; `meter_ms` 0 pauses; any other period is clamped to
/// [`IDLE_METER_MIN_MS`]..=[`IDLE_METER_MAX_MS`], [`IDLE_METER_MS`] when
/// absent.
pub fn meter_pace(on: bool, meter_ms: Option<u32>) -> Pace {
    match (on, meter_ms) {
        (false, _) => Pace::Full,
        (true, Some(0)) => Pace::Paused,
        (true, ms) => Pace::Every(Duration::from_millis(u64::from(
            ms.unwrap_or(IDLE_METER_MS)
                .clamp(IDLE_METER_MIN_MS, IDLE_METER_MAX_MS),
        ))),
    }
}

/// `POST /api/auth` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    pub pin: String,
}

/// `POST /api/auth` answer: the engineer's token and its lifetime (s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub expires_in: u64,
}

/// The body of an API error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

/// One Live instance in `GET /api/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceStatus {
    pub name: String,
    pub port: u16,
    pub online: bool,
    pub busy: bool,
    pub set_name: String,
    pub live_version: String,
    pub script_version: String,
    /// The last heartbeat's age of Live's main-thread tick.
    pub main_tick_age_ms: Option<f64>,
    /// Client subscriptions (hub keys) on this instance: the pages' (the
    /// hub's own keepers' and rules' left out).
    pub subscriptions: usize,
    /// Live listeners the hub holds on this instance for its clients
    /// (subscriptions and the name guards of name bindings, deduplicated;
    /// the hub's own keepers' left out, #58 and #68).
    pub listeners: usize,
    /// Failed connection attempts since the last connection (0 while
    /// connected).
    pub connect_failures: u64,
    /// Why the last attempt failed (nothing listens, a timeout, the port
    /// answers as another instance), until a connection succeeds.
    pub last_error: Option<String>,
    /// The groups the hub keeps unfolded (#58): those a strip's track sits
    /// in, up the chain, as Live last said; Live meters no track inside a
    /// folded group.
    #[serde(default)]
    pub unfolded: Vec<String>,
    /// The instance's transport (#130, D18), as the hub last heard
    /// `is_playing`; none before it heard one (and in an older hub's
    /// answer).
    #[serde(default)]
    pub playing: Option<bool>,
}

/// A layout binding that does not resolve on its instance (spec §2.5 D4,
/// I5): its LOM target and the script's answer (a missing or an ambiguous
/// name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub instance: String,
    pub target: String,
    pub error: String,
}

/// The layout in `GET /api/status`: the served revision (0: none), why the
/// file on disk is not served, if it is not, and the bindings that did not
/// resolve at the last check of each connected instance (a check runs when
/// a layout is accepted and when an instance connects).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutStatus {
    pub rev: u64,
    pub error: Option<String>,
    pub unresolved: Vec<Unresolved>,
    /// The Tuner markers (#68); absent in an older hub's answer.
    #[serde(default)]
    pub markers: MarkersStatus,
}

/// The Tuner markers in `GET /api/status` (#68, I9): how many the hub
/// found, and every marker problem (also a marker with no group, which
/// shows on no page).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkersStatus {
    pub found: usize,
    pub problems: Vec<crate::markers::MarkerReport>,
    /// Every fader link the composition made, with its partner count
    /// (#130); absent in an older hub's answer.
    #[serde(default)]
    pub links: Vec<crate::markers::LinkReport>,
}

/// STAGE AUT in `GET /api/status`: the flag and the mute writes it made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAutStatus {
    pub on: bool,
    pub writes: u64,
}

/// The HTTPS listener of the public name in `GET /api/status` (#17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpsStatus {
    /// The bound port (the configured one while it is not bound).
    pub port: u16,
    /// Whether the port is bound; when not, why (it is tried again).
    #[serde(default)]
    pub bound: bool,
    #[serde(default)]
    pub bind_error: Option<String>,
    /// Whether it serves (it has a certificate).
    pub serving: bool,
    /// The served certificate's names and its end (Unix seconds).
    pub cert_names: Vec<String>,
    pub not_after: Option<i64>,
    /// Whole days the certificate has left.
    pub days_left: Option<i64>,
    /// Why the stored certificate is not served, when it is not.
    pub cert_error: Option<String>,
    /// Whether the ACME client keeps the certificate (`[acme]`).
    pub acme: bool,
    /// The last ACME attempt's error, until one succeeds, and how many
    /// attempts in a row failed.
    pub acme_error: Option<String>,
    pub acme_failures: u32,
    /// When the ACME client last got a certificate (Unix seconds).
    pub last_issued: Option<i64>,
}

/// One name's certificate in `GET /api/status` (#90): the per-certificate
/// fields of [`HttpsStatus`], with the same meaning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertStatus {
    /// Whether the HTTPS listener serves it.
    pub serving: bool,
    pub cert_names: Vec<String>,
    pub not_after: Option<i64>,
    pub days_left: Option<i64>,
    pub cert_error: Option<String>,
    pub acme_error: Option<String>,
    pub acme_failures: u32,
    pub last_issued: Option<i64>,
}

/// AbleSet's own page behind the hub (#90) in `GET /api/status`: its name
/// (`[ableset]`), the hub's link to AbleSet's web app, that name's own
/// certificate and its taken WebSocket slots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbleSetStatus {
    pub name: String,
    /// AbleSet's web app on this PC (`[ableset] upstream`).
    #[serde(default)]
    pub upstream: String,
    /// The link (B3, the hub reads AbleSet's index every 5 s): up (down
    /// only after two failed reads in a row; none: not known yet), why the
    /// last read failed (none after a good one), and when it was last read
    /// (Unix seconds). The link's fields are absent in an older hub's
    /// answer.
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub checked: Option<i64>,
    pub cert: CertStatus,
    /// The taken WebSocket slots of AbleSet's tunnels (B2): the open
    /// tunnels, the upgrades still waiting for AbleSet's 101 or for their
    /// two ends, and an upgrade's non-101 answer still streaming; absent in
    /// an older hub's answer.
    #[serde(default)]
    pub sockets: usize,
}

/// cloudflared's readiness in `GET /api/status` (#17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    /// Ready connections to Cloudflare's edge (0: the tunnel is down).
    pub ready_connections: u32,
    /// Why the last check failed, when it did.
    pub error: Option<String>,
    /// When it was last checked (Unix seconds; none yet: `None`).
    pub checked: Option<i64>,
}

/// Remote access in `GET /api/status` (#17): the public name, its HTTPS
/// listener, whether internet requests are let in (with an Access JWT) and
/// the tunnel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteStatus {
    /// The `[tls]` name.
    pub name: Option<String>,
    pub https: Option<HttpsStatus>,
    /// `[access]` is configured: internet requests with a valid Access JWT
    /// are served; without it every internet request is refused.
    pub access: bool,
    pub tunnel: Option<TunnelStatus>,
}

/// The longest `error` a page's report carries, in characters (#43): the
/// error's own text (the page cuts it to 300 characters) and the first
/// lines of the thrown value's stack (4096 bytes at most). The page cuts the
/// whole `error` to it and the hub keeps that much; a report stays well
/// under the route's 10 KiB body limit.
pub const REPORT_ERROR_MAX_CHARS: usize = 4500;

/// What a page reports about itself (#26, `POST /api/client-report`): the
/// event and the page's state, every field optional text. The hub keeps
/// these fields only (serde drops any other), each cut short and stripped of
/// control characters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportFields {
    /// What happened: `load`, `connected`, `disconnected`, `reconnect`,
    /// `visibility`, `sw`, `wake-lock`, `error` or `perf`.
    pub kind: Option<String>,
    /// `standalone` (a Home Screen app) or `browser` (a browser tab).
    pub display: Option<String>,
    /// The browser's user agent.
    pub ua: Option<String>,
    /// The page's build (its bundle's `VERSION`).
    pub build: Option<String>,
    /// The host the page was loaded from (`location.host`).
    pub host: Option<String>,
    /// The screen: `<width>x<height>@<device pixel ratio>`.
    pub screen: Option<String>,
    /// The service worker (`data-sw`); none off the https origin.
    pub sw: Option<String>,
    /// The Screen Wake Lock (`data-wake-lock`); none off the https origin.
    pub wake_lock: Option<String>,
    /// `visible` or `hidden`.
    pub visibility: Option<String>,
    /// The hub socket's reconnects since the page loaded.
    pub reconnects: Option<String>,
    /// The error's text (an `error` report).
    pub error: Option<String>,
    /// A `perf` report (#5, K4): frames per second over the page's last
    /// 10 s of frames, one decimal (`59.9`); none before the first window.
    pub fps: Option<String>,
    /// A `perf` report: the longest gap between two frames in that window,
    /// whole ms.
    pub long_frame_ms: Option<String>,
    /// A `perf` report: the most pointers down at once since the page's
    /// previous `perf` report.
    pub touches_max: Option<String>,
    /// A `perf` report: the latest pointer's type (`touch`, `mouse`,
    /// `pen`).
    pub pointer: Option<String>,
}

/// A page's report as the hub keeps it (`GET /api/status`
/// `client_reports`, #26).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientReport {
    /// When the hub got it (Unix seconds).
    pub at: u64,
    /// The address it came from (through the tunnel: cloudflared's).
    pub peer: String,
    /// Through the tunnel: the client's address as Cloudflare names it
    /// (`cf-connecting-ip`); none on the LAN (the peer is the client).
    #[serde(default)]
    pub client: Option<String>,
    /// `lan` or `internet`, as the Access check classifies the request.
    pub source: String,
    #[serde(flatten)]
    pub fields: ReportFields,
}

/// The Stream Deck's Companion link in `GET /api/status` (#52), from the
/// Companion task's snapshot: online, why the last attempt failed (until a
/// session starts), the failed attempts since the last session, Companion's
/// and its Satellite API's versions, and how many keys Companion drew this
/// session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionStatus {
    pub online: bool,
    pub last_error: Option<String>,
    pub connect_failures: u64,
    pub companion_version: Option<String>,
    pub api_version: Option<String>,
    pub keys: u32,
}

/// `GET /api/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubStatus {
    pub instances: Vec<InstanceStatus>,
    pub layout: LayoutStatus,
    pub stage_aut: StageAutStatus,
    /// Connected client WebSockets.
    pub clients: usize,
    /// Remote access (#17); absent in an older hub's answer.
    #[serde(default)]
    pub remote: RemoteStatus,
    /// The pages' latest reports, oldest first (#26); absent in an older
    /// hub's answer.
    #[serde(default)]
    pub client_reports: Vec<ClientReport>,
    /// The Stream Deck's Companion link (#52); none without `[companion]`
    /// (and in an older hub's answer).
    #[serde(default)]
    pub companion: Option<CompanionStatus>,
    /// AbleSet's page (#90); none without `[ableset]` (and in an older
    /// hub's answer).
    #[serde(default)]
    pub ableset: Option<AbleSetStatus>,
    /// REVERB (#130, D18); the defaults in an older hub's answer.
    #[serde(default)]
    pub reverb: ReverbStatus,
    /// The strip leaders' fader link switches (#130); none in an older
    /// hub's answer.
    #[serde(default)]
    pub fader_links: Vec<FaderLinkStatus>,
    /// AUTOTUNE (#104, D19); the defaults in an older hub's answer.
    #[serde(default)]
    pub autotune: AutotuneStatus,
    /// Each Live set's song map, once read (#140); none in an older hub's
    /// answer.
    #[serde(default)]
    pub songmap: Vec<SongMapStatus>,
}

#[cfg(test)]
mod tests;
