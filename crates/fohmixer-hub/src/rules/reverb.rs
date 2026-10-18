//! REVERB (#130, spec D18, F30): the reverb faders (the `+REVERB` marker
//! strips, `Layout::reverb_faders`) and the hub's REVERB AUT rule. Pure: the
//! router hands it each layout's faders, Live's values, the transport, the
//! pages' taps and writes, the acks of its last writes and the time, and
//! carries out what it returns ([`Act`]); every step is an event-log record
//! (`reverb`) with one hub log line ([`step_line`]).
//!
//! The rules (design B.3–B.6):
//!
//! - **REVERB AUT:** a stop of the followed Live (the first fader's) that the
//!   rule sees while AUT is on starts a wait of [`STOP_WAIT_MS`] (the last
//!   chord's tail); then every fader glides [`DROP_DB`] down over
//!   [`LOWER_MS`]. A start glides them back in [`RETURN_MS`], each exactly
//!   to the level it had; a start within the wait lowers nothing.
//! - **The tap** (`reverb_low`): down in [`TAP_MS`], back in [`RETURN_MS`];
//!   two taps asking the same thing agree. A tap while AUT is on overrides
//!   the rule until the next stop or start, and AUT stays on.
//! - **The hand wins:** a page's write of a fader while the rule holds it
//!   (lowered, the wait, a glide), or a value of Live's off where the rule
//!   left it once the rule's last write was applied, lets that fader go
//!   (`RUKA`) until the next stop, which takes it again from where it is.
//! - **A first value never lowers:** a hub start, a reconnect or a set load
//!   only returns (Live plays, the faders lowered by the stop, AUT on).
//! - **A return waits for Live's value:** a held fader whose value the hub
//!   has not heard (a restart, a reconnect, an offline instance) is never
//!   written; it stays held until its first value: where the rule left it,
//!   it glides back; elsewhere, the hand wins.
//! - **`low` means a fader is held lowered:** the last one let go ends the
//!   lowering.
//! - **AUT off while low** leaves the faders where they are (a tap returns
//!   them); **AUT on** while Live stands starts the wait, or takes a tap's
//!   lowering over (it returns at the start).
//! - **Kept over a restart:** what the rule holds ([`Saved`], `hub-state.json`)
//!   by identity (the fader's Live set and label, never its track index); a
//!   layout that names a held fader again holds it again.
//! - **A track-list change or an offline instance** stops that instance's
//!   glides at once. Offline, what the rule holds stays; a track-list
//!   change lets go of a fader it stopped mid-glide (its value lies between
//!   the levels for good). Either opens the instance's window: until the
//!   router says its strips are settled again (`tracks_known`: the marker
//!   keeper's read since answered whole, no composition waiting), nothing is
//!   written to that instance's faders and Live's values of them are kept,
//!   not judged: a lowering skips them, a return waits (an index may name
//!   another track).

use std::collections::BTreeSet;

use fohmixer_proto::client::{Low, ReverbFader, ReverbState, ReverbStatus, set_key};
use fohmixer_proto::layout::RuleFader;
use fohmixer_proto::volume::{FLOOR_DB, value2db, volume_of};
use serde::{Deserialize, Serialize};

/// How far REVERB lowers a fader (dB).
pub const DROP_DB: f64 = 10.0;
/// After a stop, how long REVERB AUT waits (the last chord's tail), ms.
pub const STOP_WAIT_MS: f64 = 2000.0;
/// The stop's lowering glide, ms.
pub const LOWER_MS: f64 = 2000.0;
/// A tap's lowering glide, ms.
pub const TAP_MS: f64 = 500.0;
/// The return glide (a start or a tap), ms.
pub const RETURN_MS: f64 = 500.0;
/// A glide's write period, ms (40 a second per fader; a page sends 60).
pub const STEP_MS: f64 = 25.0;
/// Two volumes this close are one level (Live keeps a float32).
pub const SAME_VOLUME: f64 = 1e-6;

/// Whether two volumes are one level.
pub fn same_volume(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME_VOLUME
}

/// The volume [`DROP_DB`] under `v` on the strips' law (−∞ stays).
pub fn lowered(v: f64) -> f64 {
    volume_of(value2db(v) - DROP_DB)
}

/// Whether a time `at` has come at `now`.
pub fn due(at: f64, now: f64) -> bool {
    now >= at
}

/// Whether a volume is −∞ (nothing to lower).
pub fn silent(v: f64) -> bool {
    v <= 0.0
}

/// A glide of one fader, linear in dB (each end at the −70 dB floor at
/// least), its end exactly `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    from_db: f64,
    to_db: f64,
    to: f64,
    t0: f64,
    ms: f64,
}

impl Glide {
    pub fn new(from: f64, to: f64, t0: f64, ms: f64) -> Self {
        Self {
            from_db: value2db(from).max(FLOOR_DB),
            to_db: value2db(to).max(FLOOR_DB),
            to,
            t0,
            ms,
        }
    }

    /// Where it ends (a return's: the song level).
    pub fn to(&self) -> f64 {
        self.to
    }

    /// Its volume at `now` and whether it arrived.
    pub fn at(&self, now: f64) -> (f64, bool) {
        if due(self.t0 + self.ms, now) {
            return (self.to, true);
        }
        let p = ((now - self.t0) / self.ms).max(0.0);
        (
            volume_of(self.from_db + (self.to_db - self.from_db) * p),
            false,
        )
    }
}

/// What `hub-state.json` keeps of REVERB (flattened into `HubState`; the
/// defaults write no key, so a file of STAGE AUT alone stays as it was).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(
        default,
        rename = "reverb_aut",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub aut: bool,
    /// The faders the rule holds lowered, and why.
    #[serde(
        default,
        rename = "reverb_low",
        skip_serializing_if = "Option::is_none"
    )]
    pub low: Option<SavedLow>,
}

/// The lowered state across a hub restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLow {
    pub by: Low,
    pub faders: Vec<SavedFader>,
}

/// A held fader across a restart: who (its set and label), where it
/// returns, where the rule left it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedFader {
    pub instance: String,
    pub label: String,
    pub back: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<f64>,
}

/// A step of REVERB: the `reverb` record's fields (`step` and its own).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Aut {
        on: bool,
    },
    Tap {
        low: bool,
    },
    /// A stop seen: the lowering starts in `ms`.
    Wait {
        ms: f64,
    },
    /// The wait ended with nothing lowered: Live played within it, a tap
    /// took over, AUT went off, or every fader was in the hand.
    Cancelled,
    Lower {
        why: Low,
        faders: Vec<Move>,
    },
    Return {
        faders: Vec<Move>,
    },
    /// A held fader's return waits: for Live's value of it (`value`: none
    /// heard since a restart), or for its instance's window to end (`track
    /// list`: an index may name another track).
    Pending {
        label: String,
        why: String,
    },
    /// The hand took a fader: a page's write (`page`) or Live's own value
    /// (`live`, its `value`).
    Hand {
        label: String,
        by: By,
        value: Option<f64>,
    },
    Arrived {
        label: String,
        value: f64,
    },
    /// A glide stopped: `tracks`, `offline`, `layout` (its track moved),
    /// `hand` (a page during a return).
    Stopped {
        label: String,
        why: String,
    },
    /// Nothing to lower: `silent` (−∞) or `no value`.
    Skipped {
        label: String,
        why: String,
    },
    /// A layout named these faders.
    Faders {
        labels: Vec<String>,
    },
    /// Held faders a layout named again; `waiting` still wait for theirs.
    Restored {
        faders: usize,
        waiting: usize,
    },
    /// AUT off while low: the faders stay.
    Off,
    /// An instance's window (review M3): it opens (`open`) when its track
    /// list changed (`tracks`) or it went offline (`offline`), and nothing is
    /// written to its faders until it ends: its markers settled with no new
    /// layout (`settled`), or a layout of them served (`layout`).
    Window {
        instance: String,
        open: bool,
        why: String,
    },
}

/// One fader's glide in a lowering or a return.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Move {
    pub label: String,
    pub from: f64,
    pub to: f64,
    pub ms: f64,
}

/// Who took a fader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Page,
    Live,
}

/// What the router carries out.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// A glide step to a fader's volume (`last`: its end).
    Write {
        instance: String,
        target: String,
        key: String,
        value: f64,
        last: bool,
    },
    /// A record and its log line.
    Step(Step),
    /// `hub-state.json` gets what the rule holds.
    Save,
    /// Every page gets the new state.
    Tell,
}

/// One reverb fader.
#[derive(Debug, Clone, PartialEq)]
struct Fader {
    spec: RuleFader,
    /// Its volume's write key (a page's `set` of it has this key).
    key: String,
    /// Live's latest value.
    live: Option<f64>,
    /// Where it returns: the rule holds it low.
    back: Option<f64>,
    /// Where the rule left it (its lowering's end).
    left: Option<f64>,
    /// The engineer took it (`RUKA`) while the rule held it.
    hand: bool,
    glide: Option<Glide>,
    /// Live's values are its own: the rule's last write was applied, or
    /// Live reported where the rule left it.
    settled: bool,
    /// Held, its return asked before Live's value of it was heard.
    pending: bool,
    /// Held, Live's value of it heard in its instance's window: judged when
    /// the window ends (`Reverb::resume`).
    unjudged: bool,
}

impl Fader {
    fn of(spec: RuleFader) -> Self {
        let key = set_key(&spec.instance, &spec.target, "value");
        Self {
            spec,
            key,
            live: None,
            back: None,
            left: None,
            hand: false,
            glide: None,
            settled: true,
            pending: false,
            unjudged: false,
        }
    }

    fn held(&self) -> bool {
        self.back.is_some()
    }

    fn same(&self, other: &Fader) -> bool {
        self.spec.instance == other.spec.instance && self.spec.label == other.spec.label
    }

    /// Where it is now: its glide's value, else Live's.
    fn now_at(&self, now: f64) -> Option<f64> {
        self.glide.map(|g| g.at(now).0).or(self.live)
    }

    fn let_go(&mut self) {
        self.back = None;
        self.left = None;
        self.glide = None;
        self.hand = true;
        self.pending = false;
    }

    /// A held fader's return at `now`: a glide from where it is back to its
    /// song level; none while Live's value of it is unknown, or while its
    /// index may name another track (`writable` false): it waits,
    /// `pending`, never written blind.
    fn back_up(&mut self, now: f64, writable: bool) -> Option<Move> {
        let from = self.now_at(now).filter(|_| writable);
        self.pending = from.is_none();
        let (from, back) = (from?, self.back.take()?);
        self.left = None;
        self.settled = false;
        self.glide = Some(Glide::new(from, back, now, RETURN_MS));
        Some(Move {
            label: self.spec.label.clone(),
            from,
            to: back,
            ms: RETURN_MS,
        })
    }

    fn saved(&self) -> Option<SavedFader> {
        self.back.map(|back| SavedFader {
            instance: self.spec.instance.clone(),
            label: self.spec.label.clone(),
            back,
            left: self.left,
        })
    }

    /// Takes over the last layout's fader of its identity: what the rule
    /// holds; Live's value and a glide only when its target is the same.
    fn take_over(&mut self, kept: Fader) -> Option<Act> {
        self.back = kept.back;
        self.left = kept.left;
        self.hand = kept.hand;
        self.settled = kept.settled;
        self.pending = kept.pending;
        if kept.key == self.key {
            self.live = kept.live;
            self.glide = kept.glide;
            self.unjudged = kept.unjudged;
            return None;
        }
        kept.glide.map(|_| {
            Act::Step(Step::Stopped {
                label: self.spec.label.clone(),
                why: "layout".into(),
            })
        })
    }
}

/// Held faders waiting for a layout to name them: what is saved of each,
/// and whether its return waits for Live's value (review N4).
#[derive(Debug, Clone, PartialEq)]
struct Waiting {
    by: Low,
    faders: Vec<(SavedFader, bool)>,
}

/// REVERB's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Reverb {
    aut: bool,
    low: Option<Low>,
    faders: Vec<Fader>,
    /// The followed transport (the first fader's Live).
    playing: Option<bool>,
    /// The stop's lowering starts then (router ms).
    wait: Option<f64>,
    /// Held faders waiting for a layout to name them.
    restore: Option<Waiting>,
    writes: u64,
    hands: u64,
    /// Instances whose strips' places are unknown (their window): the track
    /// list changed, or the instance went offline, and the router has not
    /// said since that its markers are settled (review W1, M1, W2). Nothing
    /// is written to their faders, and Live's values of them are judged only
    /// when the window ends.
    moved: BTreeSet<String>,
}

impl Reverb {
    /// From `hub-state.json`: AUT, and what the rule held (restored once a
    /// layout names its faders).
    pub fn new(saved: &Saved) -> Self {
        Self {
            aut: saved.aut,
            low: None,
            faders: Vec::new(),
            playing: None,
            wait: None,
            restore: saved.low.as_ref().map(|low| Waiting {
                by: low.by,
                faders: low.faders.iter().map(|f| (f.clone(), false)).collect(),
            }),
            writes: 0,
            hands: 0,
            moved: BTreeSet::new(),
        }
    }

    /// The Live whose transport the rule follows.
    pub fn followed(&self) -> Option<&str> {
        self.faders.first().map(|f| f.spec.instance.as_str())
    }

    fn labels(&self) -> Vec<String> {
        self.faders.iter().map(|f| f.spec.label.clone()).collect()
    }

    /// A served layout's faders at `now` (router ms): each takes over what
    /// the rule held of its identity; a held fader the layout no longer
    /// names waits for one that does. A layout ends no window: its
    /// composition may predate an instance's newest read (review M1), so the
    /// router ends each settled one after it (`tracks_known`).
    pub fn set_faders(&mut self, specs: Vec<RuleFader>, now: f64) -> Vec<Act> {
        if self.faders.iter().map(|f| &f.spec).eq(specs.iter()) {
            return Vec::new();
        }
        let before = self.saved();
        let was = self.followed().map(str::to_string);
        let mut old = std::mem::take(&mut self.faders);
        let mut acts = Vec::new();
        for spec in specs {
            let mut fader = Fader::of(spec);
            if let Some(i) = old.iter().position(|o| o.same(&fader)) {
                acts.extend(fader.take_over(old.remove(i)));
            }
            self.faders.push(fader);
        }
        // A fader the layout drops stops its glide (a record each).
        for gone in old.iter().filter(|f| f.glide.is_some()) {
            acts.push(Act::Step(Step::Stopped {
                label: gone.spec.label.clone(),
                why: "layout".into(),
            }));
        }
        // A held fader the layout no longer names waits for one that does
        // (an empty wait is dropped by `restore_held` at once).
        let by = self.low.unwrap_or(Low::Stop);
        self.restore
            .get_or_insert_with(|| Waiting {
                by,
                faders: Vec::new(),
            })
            .faders
            .extend(
                old.iter()
                    .filter_map(|f| f.saved().map(|kept| (kept, f.pending))),
            );
        if self.followed() != was.as_deref() {
            self.playing = None;
            if self.wait.take().is_some() {
                acts.push(Act::Step(Step::Cancelled));
            }
        }
        acts.extend(self.restore_held(now));
        acts.push(Act::Step(Step::Faders {
            labels: self.labels(),
        }));
        // `hub-state.json` only when what the rule holds changed (a file
        // the hub could not read stays until then).
        if self.saved() != before {
            acts.push(Act::Save);
        }
        acts.push(Act::Tell);
        acts
    }

    /// Held faders a layout names again (by set and label) are held again;
    /// a lowering for the stop found while AUT is on and Live plays
    /// returns at once (it would stay low through the song).
    fn restore_held(&mut self, now: f64) -> Vec<Act> {
        let Some(saved) = self.restore.take() else {
            return Vec::new();
        };
        let mut found: Vec<usize> = Vec::new();
        let mut waiting: Vec<(SavedFader, bool)> = Vec::new();
        for (s, pending) in saved.faders {
            let at = self.faders.iter().position(|f| {
                f.spec.instance == s.instance && f.spec.label == s.label && !f.held()
            });
            let Some(i) = at else {
                waiting.push((s, pending));
                continue;
            };
            let fader = &mut self.faders[i];
            fader.back = Some(s.back);
            fader.left = s.left;
            fader.hand = false;
            fader.settled = true;
            fader.pending = pending;
            found.push(i);
        }
        let count = waiting.len();
        if count > 0 {
            self.restore = Some(Waiting {
                by: saved.by,
                faders: waiting,
            });
        }
        if found.is_empty() {
            return Vec::new();
        }
        if self.low.is_none() {
            self.low = Some(saved.by);
        }
        let mut acts = vec![Act::Step(Step::Restored {
            faders: found.len(),
            waiting: count,
        })];
        if saved.by == Low::Stop && self.aut && self.playing == Some(true) {
            acts.extend(self.return_these(&found, now));
        }
        acts
    }

    /// Live's value of a fader's volume (`key`: its write key) at `now`
    /// (router ms).
    pub fn live(&mut self, key: &str, value: f64, now: f64) -> Vec<Act> {
        let Some(fader) = self.faders.iter_mut().find(|f| f.key == key) else {
            return Vec::new();
        };
        fader.live = Some(value);
        if self.moved.contains(&fader.spec.instance) {
            // Its index may name another track until the window ends: the
            // value is judged then (`resume`), never by a stale index (W2).
            fader.unjudged = fader.held();
            return Vec::new();
        }
        fader.unjudged = false;
        let at_left = fader.left.is_some_and(|left| same_volume(left, value));
        if fader.pending {
            // The return waited for this value: where the rule left the
            // fader, it glides back; elsewhere, the hand wins.
            let label = fader.spec.label.clone();
            let step = match at_left.then(|| fader.back_up(now, true)).flatten() {
                Some(back) => Step::Return { faders: vec![back] },
                None => {
                    fader.let_go();
                    self.hands += 1;
                    Step::Hand {
                        label,
                        by: By::Live,
                        value: Some(value),
                    }
                }
            };
            self.drop_low();
            return vec![Act::Step(step), Act::Save, Act::Tell];
        }
        if !fader.settled {
            fader.settled = at_left;
            return Vec::new();
        }
        if !fader.held() || fader.glide.is_some() || at_left {
            return Vec::new();
        }
        fader.let_go();
        let label = fader.spec.label.clone();
        self.hands += 1;
        self.drop_low();
        vec![
            Act::Step(Step::Hand {
                label,
                by: By::Live,
                value: Some(value),
            }),
            Act::Save,
            Act::Tell,
        ]
    }

    /// Live's value of a fader's volume the hub already holds, handed at a
    /// new layout: taken only by a fader with no value yet (one recomposed
    /// onto a key the hub already follows, review N1); a fader that has its
    /// value heard it already.
    pub fn cached(&mut self, key: &str, value: f64, now: f64) -> Vec<Act> {
        if self.faders.iter().any(|f| f.key == key && f.live.is_none()) {
            return self.live(key, value, now);
        }
        Vec::new()
    }

    /// Whether a held fader's return waits for Live's value, its strip shown
    /// or away (N3, R2-1).
    fn waiting_return(&self) -> bool {
        self.faders.iter().any(|f| f.pending)
            || self
                .restore
                .as_ref()
                .is_some_and(|w| w.faders.iter().any(|(_, pending)| *pending))
    }

    /// The window of the instances `ended` closed (review W1, W2): a return
    /// that waited for it, and a held fader whose value came in it, go on as
    /// Live's value would take them.
    fn resume(&mut self, ended: &BTreeSet<String>, now: f64) -> Vec<Act> {
        let heard: Vec<(String, f64)> = self
            .faders
            .iter()
            .filter(|f| (f.pending || f.unjudged) && ended.contains(&f.spec.instance))
            .filter_map(|f| f.live.map(|v| (f.key.clone(), v)))
            .collect();
        heard
            .into_iter()
            .flat_map(|(key, value)| self.live(&key, value, now))
            .collect()
    }

    /// The router found `instance`'s strips settled (the marker keeper's read
    /// answered whole since the window opened, no composition waiting, and a
    /// layout of it served or none needed): the window closes.
    pub fn tracks_known(&mut self, instance: &str, now: f64) -> Vec<Act> {
        self.end_window(instance, "settled", now)
    }

    /// A layout served `instance`'s strips with its markers settled (review
    /// M1): the window closes.
    pub fn tracks_served(&mut self, instance: &str, now: f64) -> Vec<Act> {
        self.end_window(instance, "layout", now)
    }

    /// Opens `instance`'s window (`why`: its cause), recorded once.
    fn open_window(&mut self, instance: &str, why: &str) -> Vec<Act> {
        if !self.moved.insert(instance.to_string()) {
            return Vec::new();
        }
        vec![Act::Step(Step::Window {
            instance: instance.to_string(),
            open: true,
            why: why.to_string(),
        })]
    }

    /// Ends `instance`'s window (`why`: how), recorded, and resumes what
    /// waited for it.
    fn end_window(&mut self, instance: &str, why: &str, now: f64) -> Vec<Act> {
        if !self.moved.remove(instance) {
            return Vec::new();
        }
        let mut acts = vec![Act::Step(Step::Window {
            instance: instance.to_string(),
            open: false,
            why: why.to_string(),
        })];
        acts.extend(self.resume(&BTreeSet::from([instance.to_string()]), now));
        acts
    }

    /// `low` says a fader is held lowered: with none held, it ends.
    fn drop_low(&mut self) {
        if !self.faders.iter().any(Fader::held) {
            self.low = None;
        }
    }

    /// A transport value of `instance` at `now` (router ms).
    pub fn transport(&mut self, instance: &str, playing: bool, now: f64) -> Vec<Act> {
        if self.followed() != Some(instance) {
            return Vec::new();
        }
        let before = self.playing.replace(playing);
        if before == Some(playing) {
            return Vec::new();
        }
        let mut acts = vec![Act::Tell];
        match (before, playing) {
            (None, true) if self.aut && self.low == Some(Low::Stop) => {
                acts.extend(self.return_all(now));
            }
            (None, _) => {}
            (Some(_), false) if self.aut => {
                // The next stop takes a fader the hand had let go again.
                for fader in self.faders.iter_mut().filter(|f| !f.held()) {
                    fader.hand = false;
                }
                self.wait = Some(now + STOP_WAIT_MS);
                acts.push(Act::Step(Step::Wait { ms: STOP_WAIT_MS }));
            }
            (Some(_), false) => {}
            (Some(_), true) => {
                if self.wait.take().is_some() {
                    acts.push(Act::Step(Step::Cancelled));
                }
                if self.aut && self.low.is_some() {
                    acts.extend(self.return_all(now));
                }
            }
        }
        acts
    }

    /// Every fader neither held low nor in the hand is remembered and glides
    /// 10 dB down over `ms`. A fader still returning keeps the song level
    /// its return glides to; a held one whose return waits stays held.
    fn lower(&mut self, why: Low, ms: f64, now: f64) -> Vec<Act> {
        // A held fader whose strip is away is taken back too: it comes back
        // lowered for this reason, its return no longer waiting (R2-1); a
        // change of it is a lowering, recorded and saved (review M2).
        let taken = self.restore.as_mut().is_some_and(|waiting| {
            let changed = waiting.by != why || waiting.faders.iter().any(|(_, p)| *p);
            waiting.by = why;
            for (_, pending) in &mut waiting.faders {
                *pending = false;
            }
            changed
        });
        let mut acts = Vec::new();
        let mut moves = Vec::new();
        for fader in &mut self.faders {
            if fader.held() {
                fader.pending = false;
                continue;
            }
            if fader.hand {
                continue;
            }
            let label = fader.spec.label.clone();
            if self.moved.contains(&fader.spec.instance) {
                // Its index may name another track until the next layout:
                // it stays at its level for this lowering (review W1).
                acts.push(Act::Step(Step::Skipped {
                    label,
                    why: "track list".into(),
                }));
                continue;
            }
            // A fader not held glides only back up: its song level is that
            // glide's end, wherever the glide is now.
            let level = fader.glide.map(|g| g.to()).or(fader.live);
            let Some((from, level)) = fader.now_at(now).zip(level) else {
                acts.push(Act::Step(Step::Skipped {
                    label,
                    why: "no value".into(),
                }));
                continue;
            };
            if silent(level) {
                acts.push(Act::Step(Step::Skipped {
                    label,
                    why: "silent".into(),
                }));
                continue;
            }
            let to = lowered(level);
            fader.back = Some(level);
            fader.left = Some(to);
            fader.hand = false;
            fader.settled = false;
            fader.glide = Some(Glide::new(from, to, now, ms));
            moves.push(Move {
                label,
                from,
                to,
                ms,
            });
        }
        if taken || self.faders.iter().any(Fader::held) {
            self.low = Some(why);
            acts.insert(0, Act::Step(Step::Lower { why, faders: moves }));
            acts.push(Act::Save);
            acts.push(Act::Tell);
        }
        acts
    }

    /// Every held fader glides back to its song level; the rule lets go.
    fn return_all(&mut self, now: f64) -> Vec<Act> {
        let held: Vec<usize> = self
            .faders
            .iter()
            .enumerate()
            .filter(|(_, f)| f.held())
            .map(|(i, _)| i)
            .collect();
        for fader in &mut self.faders {
            fader.hand = false;
        }
        self.wait = None;
        self.return_these(&held, now)
    }

    /// The held faders `which` return: each glides back, or (Live's value
    /// of it unknown) waits for it, recorded once.
    fn return_these(&mut self, which: &[usize], now: f64) -> Vec<Act> {
        let mut moves = Vec::new();
        let mut waits = Vec::new();
        for &i in which {
            let writable = !self.moved.contains(&self.faders[i].spec.instance);
            let fader = &mut self.faders[i];
            let was = fader.pending;
            match fader.back_up(now, writable) {
                Some(back) => moves.push(back),
                None if !was => waits.push(Act::Step(Step::Pending {
                    label: fader.spec.label.clone(),
                    why: pending_why(writable).into(),
                })),
                None => {}
            }
        }
        self.drop_low();
        let mut acts = Vec::new();
        if !moves.is_empty() {
            acts.push(Act::Step(Step::Return { faders: moves }));
        }
        acts.extend(waits);
        acts.push(Act::Save);
        acts.push(Act::Tell);
        acts
    }

    /// A page's tap at `now`: the faders lowered (`low`) or at the song
    /// level, as the button asks (two taps asking the same agree).
    pub fn tap(&mut self, low: bool, now: f64) -> Vec<Act> {
        let mut acts = vec![Act::Step(Step::Tap { low })];
        match (low, self.low.is_some()) {
            (true, false) => {
                if self.wait.take().is_some() {
                    acts.push(Act::Step(Step::Cancelled));
                }
                // A tap asks for every reverb down: the hand's too.
                for fader in &mut self.faders {
                    fader.hand = false;
                }
                let why = if self.aut && self.playing == Some(false) {
                    Low::Stop
                } else {
                    Low::Tap
                };
                acts.extend(self.lower(why, TAP_MS, now));
            }
            (false, true) => acts.extend(self.return_all(now)),
            (false, false) => {
                if self.wait.take().is_some() {
                    acts.push(Act::Step(Step::Cancelled));
                }
            }
            // Low already: one lowering; but a return waiting for Live's
            // value is called off, the fader kept low (review N3).
            (true, true) if self.waiting_return() => {
                let why = if self.aut && self.playing == Some(false) {
                    Low::Stop
                } else {
                    Low::Tap
                };
                acts.extend(self.lower(why, TAP_MS, now));
            }
            (true, true) => {}
        }
        acts.push(Act::Tell);
        acts
    }

    /// A page turned REVERB AUT on or off at `now`.
    pub fn set_aut(&mut self, on: bool, now: f64) -> Vec<Act> {
        if on == self.aut {
            return vec![Act::Tell];
        }
        self.aut = on;
        let mut acts = vec![Act::Step(Step::Aut { on }), Act::Save, Act::Tell];
        if on {
            match (self.playing, self.low) {
                (Some(false), None) => {
                    self.wait = Some(now + STOP_WAIT_MS);
                    acts.push(Act::Step(Step::Wait { ms: STOP_WAIT_MS }));
                }
                (Some(false), Some(Low::Tap)) => self.low = Some(Low::Stop),
                _ => {}
            }
        } else {
            if self.wait.take().is_some() {
                acts.push(Act::Step(Step::Cancelled));
            }
            if self.low == Some(Low::Stop) {
                self.low = Some(Low::Tap);
                acts.push(Act::Step(Step::Off));
            }
        }
        acts
    }

    /// A page wrote a fader's volume (`key`: its write key).
    pub fn hand(&mut self, key: &str) -> Vec<Act> {
        let rule_holds = self.wait.is_some() || self.low.is_some();
        let Some(fader) = self.faders.iter_mut().find(|f| f.key == key) else {
            return Vec::new();
        };
        let label = fader.spec.label.clone();
        if fader.held() || (rule_holds && !fader.hand) {
            fader.let_go();
            self.hands += 1;
            self.drop_low();
            return vec![
                Act::Step(Step::Hand {
                    label,
                    by: By::Page,
                    value: None,
                }),
                Act::Save,
                Act::Tell,
            ];
        }
        if fader.glide.take().is_some() {
            return vec![Act::Step(Step::Stopped {
                label,
                why: "hand".into(),
            })];
        }
        Vec::new()
    }

    /// The ack of the rule's last write of `key` came.
    pub fn applied(&mut self, key: &str) {
        if let Some(fader) = self
            .faders
            .iter_mut()
            .find(|f| f.key == key && f.glide.is_none())
        {
            fader.settled = true;
        }
    }

    /// The time is `now` (router ms): the wait's end, and every glide's step.
    pub fn tick(&mut self, now: f64) -> Vec<Act> {
        let mut acts = Vec::new();
        if self.wait.is_some_and(|at| due(at, now)) {
            self.wait = None;
            let lowered = self.lower(Low::Stop, LOWER_MS, now);
            let none = !lowers(&lowered);
            acts.extend(lowered);
            if none {
                acts.push(Act::Step(Step::Cancelled));
            }
            // The wait is over: every page is told.
            acts.push(Act::Tell);
        }
        for fader in &mut self.faders {
            let Some(glide) = fader.glide else {
                continue;
            };
            let (value, last) = glide.at(now);
            acts.push(Act::Write {
                instance: fader.spec.instance.clone(),
                target: fader.spec.target.clone(),
                key: fader.key.clone(),
                value,
                last,
            });
            self.writes += 1;
            if last {
                fader.glide = None;
                acts.push(Act::Step(Step::Arrived {
                    label: fader.spec.label.clone(),
                    value,
                }));
            }
        }
        acts
    }

    /// When the rule next needs the time: the wait's end, or the next
    /// glide step; none while it rests.
    pub fn next_tick(&self, now: f64) -> Option<f64> {
        let step = self
            .faders
            .iter()
            .any(|f| f.glide.is_some())
            .then_some(now + STEP_MS);
        match (self.wait, step) {
            (Some(wait), Some(step)) => Some(wait.min(step)),
            (wait, step) => wait.or(step),
        }
    }

    /// An instance went offline: its glides stop, its values (and, for the
    /// followed one, the transport and the wait) are forgotten; what the
    /// rule holds stays. Its window opens (review W2): a set loaded or a
    /// track moved meanwhile is known only from the connect's read.
    pub fn offline(&mut self, instance: &str) -> Vec<Act> {
        let mut acts = self.open_window(instance, "offline");
        for fader in self
            .faders
            .iter_mut()
            .filter(|f| f.spec.instance == instance)
        {
            fader.live = None;
            fader.settled = true;
            if fader.glide.take().is_some() {
                acts.push(Act::Step(Step::Stopped {
                    label: fader.spec.label.clone(),
                    why: "offline".into(),
                }));
            }
        }
        if self.followed() == Some(instance) {
            self.playing = None;
            if self.wait.take().is_some() {
                acts.push(Act::Step(Step::Cancelled));
            }
            acts.push(Act::Tell);
        }
        acts
    }

    /// An instance's track list changed: its glides stop at once (an index
    /// names another track until the next layout), and Live's values count
    /// again only once one is where the rule left the fader. A fader stopped
    /// mid-glide is let go: its value lies between the levels for good. Until
    /// the router says its strips are settled (`tracks_known`), nothing is
    /// written to the instance's faders (review W1, M1).
    pub fn tracks_changed(&mut self, instance: &str) -> Vec<Act> {
        let mut acts = self.open_window(instance, "tracks");
        let before = self.saved();
        let mut stopped = Vec::new();
        for fader in self
            .faders
            .iter_mut()
            .filter(|f| f.spec.instance == instance)
        {
            fader.settled = false;
            if fader.glide.is_some() {
                stopped.push(Act::Step(Step::Stopped {
                    label: fader.spec.label.clone(),
                    why: "tracks".into(),
                }));
                fader.let_go();
            }
        }
        if !stopped.is_empty() {
            acts.extend(stopped);
            self.drop_low();
            // A stopped return glide changes nothing the rule holds.
            if self.saved() != before {
                acts.push(Act::Save);
            }
            acts.push(Act::Tell);
        }
        acts
    }

    /// The followed Live's transport listener failed: the transport is
    /// unknown (a later value is a first one) and the stop's wait ends: an
    /// unknown transport never lowers.
    pub fn transport_lost(&mut self, instance: &str) -> Vec<Act> {
        if self.followed() != Some(instance) {
            return Vec::new();
        }
        self.playing = None;
        let mut acts = Vec::new();
        if self.wait.take().is_some() {
            acts.push(Act::Step(Step::Cancelled));
        }
        acts.push(Act::Tell);
        acts
    }

    /// What every page is told.
    pub fn state(&self) -> ReverbState {
        ReverbState {
            aut: self.aut,
            low: self.low,
            playing: self.playing,
            waiting: self.wait.is_some(),
            faders: self
                .faders
                .iter()
                .map(|f| ReverbFader {
                    key: f.key.clone(),
                    label: f.spec.label.clone(),
                    back: f.back,
                    hand: f.hand,
                })
                .collect(),
        }
    }

    /// What `hub-state.json` keeps: AUT, and every held fader (those a
    /// layout names and those still waiting for one).
    pub fn saved(&self) -> Saved {
        let mut faders: Vec<SavedFader> = self.faders.iter().filter_map(Fader::saved).collect();
        if let Some(waiting) = &self.restore {
            faders.extend(waiting.faders.iter().map(|(kept, _)| kept.clone()));
        }
        let by = self.low.or_else(|| self.restore.as_ref().map(|r| r.by));
        Saved {
            aut: self.aut,
            low: by.map(|by| SavedLow { by, faders }),
        }
    }

    /// `/api/status` `reverb`.
    pub fn status(&self) -> ReverbStatus {
        ReverbStatus {
            aut: self.aut,
            low: self.low,
            faders: self.faders.len(),
            writes: self.writes,
            hands: self.hands,
        }
    }
}

/// Why a return waits: its index may name another track (`writable` false),
/// or Live's value of it is unknown.
fn pending_why(writable: bool) -> &'static str {
    if writable { "value" } else { "track list" }
}

/// Whether `acts` hold a lowering (the stop's wait ended with one).
fn lowers(acts: &[Act]) -> bool {
    acts.iter()
        .any(|a| matches!(a, Act::Step(Step::Lower { .. })))
}

/// A step's hub log line.
pub fn step_line(step: &Step) -> String {
    let labels = |moves: &[Move]| {
        moves
            .iter()
            .map(|m| m.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match step {
        Step::Aut { on } => format!("REVERB AUT turned {}", if *on { "on" } else { "off" }),
        Step::Tap { low } => format!("REVERB tapped: {}", if *low { "down" } else { "back" }),
        Step::Wait { ms } => {
            format!("Live stopped: REVERB AUT lowers the reverb faders in {ms:.0} ms")
        }
        Step::Cancelled => "REVERB AUT's wait ended with nothing lowered".to_string(),
        Step::Lower {
            why: Low::Stop,
            faders,
        } if faders.is_empty() => {
            "REVERB AUT takes over the reverb faders already low: they return at the start"
                .to_string()
        }
        Step::Lower {
            why: Low::Tap,
            faders,
        } if faders.is_empty() => {
            "REVERB keeps the reverb faders low (tap): their waiting return is off".to_string()
        }
        Step::Lower { why, faders } => format!(
            "REVERB lowers {} by {DROP_DB:.0} dB ({})",
            labels(faders),
            if *why == Low::Stop { "stop" } else { "tap" }
        ),
        Step::Return { faders } => {
            format!("REVERB returns {} to the song level", labels(faders))
        }
        Step::Pending { label, why } if why == "value" => {
            format!("REVERB returns {label} once Live reports where it is")
        }
        Step::Pending { label, .. } => {
            format!("REVERB returns {label} once its track list settles")
        }
        Step::Hand {
            label,
            by: By::Page,
            ..
        } => format!("REVERB lets go of {label}: moved on a page"),
        Step::Hand {
            label,
            by: By::Live,
            value,
        } => format!(
            "REVERB lets go of {label}: Live has it at {}",
            value.unwrap_or(f64::NAN)
        ),
        Step::Arrived { label, value } => format!("REVERB: {label} arrived at {value}"),
        Step::Stopped { label, why } => format!("REVERB stopped the glide of {label}: {why}"),
        Step::Skipped { label, why } => format!("REVERB skips {label}: {why}"),
        Step::Faders { labels } if labels.is_empty() => {
            "REVERB has no faders (no +REVERB strip)".to_string()
        }
        Step::Faders { labels } => format!("REVERB's faders: {}", labels.join(", ")),
        Step::Restored { faders, waiting } => {
            format!("REVERB holds {faders} restored fader(s) low; {waiting} wait for their strip")
        }
        Step::Off => {
            "REVERB AUT off while the reverb faders are low: they stay, a tap returns them"
                .to_string()
        }
        Step::Window {
            instance,
            open: true,
            why,
        } => format!(
            "REVERB writes nothing to {instance}'s reverb faders until their strips are settled ({why})"
        ),
        Step::Window { instance, why, .. } => format!(
            "REVERB writes {instance}'s reverb faders again: their strips are settled ({why})"
        ),
    }
}

/// Whether a step's line is a warning: a glide stopped, a fader skipped, or
/// held faders whose strip no layout names.
pub fn step_warn(step: &Step) -> bool {
    match step {
        Step::Stopped { .. } | Step::Skipped { .. } => true,
        Step::Restored { waiting, .. } => *waiting > 0,
        _ => false,
    }
}

#[cfg(test)]
#[path = "reverb/tests.rs"]
mod tests;
