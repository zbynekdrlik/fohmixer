//! Listen (#210 phase 2; the owner's approved mockup
//! `docs/mockups/listen-v1.html`): a track's sound, before its fader, in the
//! browser. The channel detail's 🎧 key starts and stops it, a slim volume
//! beside the key while it listens, another channel picked in the detail is
//! followed, and the column foot says `listening <name>`. The hub's side is
//! `/ws/audio` (`fohmixer_proto::listen`, `.claude/rules/listen.md`): one
//! track at a time on a hub, the newest start wins (`taken`).
//!
//! Pure: what a tap, a channel change, the socket and the hub's statuses do
//! ([`Listen`]), the name the hub needs ([`track_name`], [`name_of`]), the
//! foot's text ([`foot_text`]) and the volume's law ([`gain_db`]).
//! `crate::listen` carries it out (the socket, the player).

use fohmixer_proto::layout::{Anchor, Binding};
use fohmixer_proto::listen::{AudioClientMsg, AudioState};
use serde_json::Value;

/// The wait before a lost audio socket is opened again, ms.
pub const RECONNECT_MS: u64 = 1000;
/// Socket opens in a row that close before they open: then the hub's
/// Listen is taken as not there (a hub with `[listen] port = 0` answers
/// the handshake 503) and the page stops trying.
pub const MAX_FAILED_OPENS: u32 = 3;
/// The volume's place at a page's start (the mockup's 70 of 100).
pub const VOLUME_DEFAULT: f64 = 0.7;
/// The volume's dB at the slider's left end (above it; the end is silence).
pub const MIN_DB: f64 = -40.0;
/// The volume's dB at the slider's right end: a quiet track can be raised;
/// the player's -1 dBFS limiter keeps it from clipping.
pub const MAX_DB: f64 = 12.0;

/// A track to listen to: its Live instance, its track's name in Live (the
/// hub picks the device on it by that name) and the label the page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub instance: String,
    pub track: String,
    pub label: String,
}

impl Target {
    /// Whether the hub's status names this track.
    fn named(&self, instance: Option<&str>, track: Option<&str>) -> bool {
        instance == Some(self.instance.as_str()) && track == Some(self.track.as_str())
    }

    /// The start message of this track.
    pub fn start(&self) -> AudioClientMsg {
        AudioClientMsg::ListenStart {
            instance: self.instance.clone(),
            track: self.track.clone(),
        }
    }
}

/// Where the hub's track name comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackName {
    /// The strip binds its track by that name.
    Known(String),
    /// The strip binds its track by its place (a Tuner marker strip, #68,
    /// the master): Live's `name` of this object is read first.
    Read(String),
    /// Nothing to listen to (the song, a control waiting for its Tuner).
    None,
}

/// The track name a strip's binding gives the hub, or where to read it.
pub fn track_name(binding: &Binding) -> TrackName {
    match &binding.anchor {
        Anchor::Track { name } | Anchor::Return { name } => TrackName::Known(name.clone()),
        Anchor::TrackAt { index } => TrackName::Read(format!("live_set tracks {index}")),
        Anchor::ReturnAt { index } => TrackName::Read(format!("live_set return_tracks {index}")),
        Anchor::Master => TrackName::Read("live_set master_track".to_string()),
        Anchor::Song | Anchor::Ctrl { .. } => TrackName::None,
    }
}

/// The name a `get_prop name` read answered: its first slot's text, not
/// empty; none for a refusal, no answer or anything else.
pub fn name_of(outcome: &Result<Vec<Value>, String>) -> Option<String> {
    let slot = outcome.as_ref().ok()?.first()?;
    if slot.get("ok") != Some(&Value::Bool(true)) {
        return None;
    }
    let name = slot.get("data")?.as_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Where the page's listening stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Not listening.
    Off,
    /// Asked: the socket opens or the hub has not answered yet.
    Opening,
    /// The hub sends the track's packets.
    Listening,
    /// The hub hears nothing of the track (no device, two devices, the
    /// instance offline, or silence).
    NoSource,
    /// The audio socket closed while listening: it opens again.
    Lost,
    /// Another page started another track (the newest start wins): stopped.
    Taken,
    /// The hub refused the start (a track this login does not reach).
    Refused,
    /// This browser has no Opus decoder (WebCodecs, a secure page).
    NoDecoder,
    /// The audio socket never opened [`MAX_FAILED_OPENS`] times in a row:
    /// stopped.
    Unavailable,
}

impl Phase {
    /// The key's `data-state`.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Off => "off",
            Phase::Opening => "opening",
            Phase::Listening => "listening",
            Phase::NoSource => "no_source",
            Phase::Lost => "lost",
            Phase::Taken => "taken",
            Phase::Refused => "refused",
            Phase::NoDecoder => "no_decoder",
            Phase::Unavailable => "unavailable",
        }
    }
}

/// What the glue does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Start the player: inside the tap itself, since a browser lets a
    /// page's audio start only in a user's gesture (a track's name to read
    /// comes later).
    Play,
    /// Stop the player.
    Silence,
    /// Open the audio socket.
    Open,
    /// Open it again after [`RECONNECT_MS`], if [`Listen::reopen_due`]
    /// still holds for this turn then.
    Reopen(u64),
    /// Close the audio socket (the hub's lease goes with it).
    Close,
    /// Send this on the open socket.
    Send(AudioClientMsg),
}

/// What an ask is for: a tap's start, or following another channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Start,
    Follow,
}

/// The page's listening: the track wanted, the phase, whether the audio
/// socket is open, the last track's label (the foot names a track taken or
/// refused), the ask waiting for its track's name, the socket's turn (a
/// reopen of an older turn does nothing) and the opens in a row that never
/// opened.
#[derive(Debug, Clone, PartialEq)]
pub struct Listen {
    want: Option<Target>,
    phase: Phase,
    open: bool,
    last: Option<String>,
    /// The newest ask; an answer to an older one is stale.
    asked: u64,
    /// What the newest ask is for, with the label it shows meanwhile.
    pending: Option<(Purpose, String)>,
    turn: u64,
    fails: u32,
    /// A socket opened since the tap: later losses are an outage (a hub
    /// restarting, a Wi-Fi gap), never a hub without Listen.
    opened_once: bool,
}

impl Default for Listen {
    fn default() -> Self {
        Self::new()
    }
}

impl Listen {
    pub fn new() -> Self {
        Self {
            want: None,
            phase: Phase::Off,
            open: false,
            last: None,
            asked: 0,
            pending: None,
            turn: 0,
            fails: 0,
            opened_once: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The track wanted.
    pub fn want(&self) -> Option<&Target> {
        self.want.as_ref()
    }

    /// Whether the key is lit: a track is wanted, or a tap's start waits
    /// for its track's name.
    pub fn lit(&self) -> bool {
        self.want.is_some() || matches!(self.pending, Some((Purpose::Start, _)))
    }

    /// The label the foot names: the track asked for, else the wanted
    /// one's, else the last one's.
    pub fn label(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(|(_, label)| label.as_str())
            .or(self.want.as_ref().map(|t| t.label.as_str()))
            .or(self.last.as_deref())
    }

    /// A new ask: every older answer is stale from now on.
    fn ask(&mut self, purpose: Purpose, label: String) -> u64 {
        self.asked += 1;
        self.pending = Some((purpose, label));
        self.asked
    }

    /// The 🎧 key's tap on the detail of the channel shown as `label`;
    /// `decoder`: the browser decodes Opus. Lit (or a start still waiting
    /// for its name), it stops; else it starts the player and asks for the
    /// channel's track (its number: answer it with [`Listen::answer`]).
    pub fn tap(&mut self, label: String, decoder: bool) -> (Vec<Act>, Option<u64>) {
        if self.lit() {
            let was = self.want.take();
            // The ask waiting goes: its late answer finds nothing pending.
            let pending = self.pending.take().map(|(_, label)| label);
            self.last = was.as_ref().map(|t| t.label.clone()).or(pending);
            self.phase = Phase::Off;
            self.open = false;
            let acts = if was.is_some() {
                vec![Act::Close, Act::Silence]
            } else {
                vec![Act::Silence]
            };
            return (acts, None);
        }
        self.last = Some(label.clone());
        if !decoder {
            self.phase = Phase::NoDecoder;
            return (Vec::new(), None);
        }
        self.phase = Phase::Opening;
        self.fails = 0;
        self.opened_once = false;
        let n = self.ask(Purpose::Start, label);
        (vec![Act::Play], Some(n))
    }

    /// The detail shows the channel labelled `label` now: while listening
    /// (or a start waits) its track is asked for, to be followed (the
    /// mockup: picking another channel keeps listening); none: nothing to
    /// follow.
    pub fn follow(&mut self, label: String) -> Option<u64> {
        if !self.lit() {
            return None;
        }
        let purpose = match self.pending {
            Some((Purpose::Start, _)) => Purpose::Start,
            _ => Purpose::Follow,
        };
        Some(self.ask(purpose, label))
    }

    /// The answer to ask `n`: the channel's track (none: its name is not
    /// known, nothing to listen to there). A stale answer does nothing.
    pub fn answer(&mut self, n: u64, target: Option<Target>) -> Vec<Act> {
        if n != self.asked {
            return Vec::new();
        }
        let Some((purpose, label)) = self.pending.take() else {
            return Vec::new();
        };
        match (purpose, target) {
            (Purpose::Start, None) => {
                self.last = Some(label);
                self.phase = Phase::Off;
                vec![Act::Silence]
            }
            (Purpose::Start, Some(target)) => {
                self.phase = Phase::Opening;
                let act = self.start_act(&target);
                self.want = Some(target);
                vec![act]
            }
            (Purpose::Follow, None) => Vec::new(),
            (Purpose::Follow, Some(target)) => self.follow_track(target),
        }
    }

    /// The start of `target`: sent when the socket is open, else the socket
    /// opens (and asks once open).
    fn start_act(&self, target: &Target) -> Act {
        if self.open {
            Act::Send(target.start())
        } else {
            Act::Open
        }
    }

    /// Another channel's track while listening: the same track keeps
    /// listening with its new label, another is asked for.
    fn follow_track(&mut self, target: Target) -> Vec<Act> {
        let Some(want) = self.want.as_mut() else {
            return Vec::new();
        };
        if want.instance == target.instance && want.track == target.track {
            want.label = target.label;
            return Vec::new();
        }
        self.phase = Phase::Opening;
        let acts = if self.open {
            vec![Act::Send(target.start())]
        } else {
            Vec::new()
        };
        self.want = Some(target);
        acts
    }

    /// The audio socket opened: the wanted track is asked for; with none
    /// wanted any more it closes again.
    pub fn opened(&mut self) -> Vec<Act> {
        self.opened_once = true;
        self.open = true;
        match &self.want {
            Some(target) => vec![Act::Send(target.start())],
            None => {
                self.open = false;
                vec![Act::Close]
            }
        }
    }

    /// The audio socket closed: while a track is wanted it opens again,
    /// unless the first [`MAX_FAILED_OPENS`] opens since the tap all closed
    /// before they opened (a hub with Listen off answers the handshake 503;
    /// once a socket opened, a loss is an outage and it keeps trying).
    pub fn closed(&mut self) -> Vec<Act> {
        let was_open = std::mem::take(&mut self.open);
        if !was_open && !self.opened_once {
            self.fails += 1;
        }
        if self.want.is_none() {
            return Vec::new();
        }
        if self.fails >= MAX_FAILED_OPENS {
            return self.end(Phase::Unavailable);
        }
        self.phase = Phase::Lost;
        self.turn += 1;
        vec![Act::Reopen(self.turn)]
    }

    /// Whether a reopen of `turn` is still due: a track is wanted, no
    /// socket opened since, and no newer turn.
    pub fn reopen_due(&self, turn: u64) -> bool {
        self.want.is_some() && !self.open && turn == self.turn
    }

    /// The hub's `audio_status`. A status of another track (one asked
    /// before a follow) changes nothing.
    pub fn status(
        &mut self,
        state: AudioState,
        instance: Option<&str>,
        track: Option<&str>,
    ) -> Vec<Act> {
        let Some(want) = self.want.as_ref() else {
            return Vec::new();
        };
        let ours = want.named(instance, track);
        match state {
            AudioState::Listening if ours => self.phase = Phase::Listening,
            AudioState::NoSource if ours => self.phase = Phase::NoSource,
            AudioState::Taken if ours => return self.end(Phase::Taken),
            // A refusal of the wanted track, or of a message the hub could
            // not read (none named); never a late one of a track asked
            // before a follow.
            AudioState::Refused if ours || instance.is_none() => {
                return self.end(Phase::Refused);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Listening ends by the hub's word (or the hub's Listen is not there):
    /// an ask still waiting is void too.
    fn end(&mut self, phase: Phase) -> Vec<Act> {
        self.last = self.want.take().map(|t| t.label);
        self.pending = None;
        self.phase = phase;
        self.open = false;
        vec![Act::Close, Act::Silence]
    }
}

/// The column foot's line: what the page listens to, or why it stopped;
/// none while off.
pub fn foot_text(phase: Phase, label: Option<&str>) -> Option<String> {
    let label = label.unwrap_or("");
    match phase {
        Phase::Off => None,
        Phase::Opening | Phase::Listening => Some(format!("listening {label}")),
        Phase::NoSource => Some(format!("{label}: no source")),
        Phase::Lost => Some(format!("{label}: reconnecting")),
        Phase::Taken => Some(format!("{label}: taken by another page")),
        Phase::Refused => Some(format!("{label}: refused")),
        Phase::NoDecoder => Some("no audio decoder in this browser".to_string()),
        Phase::Unavailable => Some(format!("{label}: the hub's Listen does not answer")),
    }
}

/// The volume's dB at its place `pos` (0 to 1): silence (none) at 0, else
/// [`MIN_DB`] to [`MAX_DB`] in a straight line.
pub fn gain_db(pos: f64) -> Option<f64> {
    let pos = pos.clamp(0.0, 1.0);
    (pos > 0.0).then_some(MIN_DB + pos * (MAX_DB - MIN_DB))
}

/// The player's linear gain at `pos`.
pub fn gain(pos: f64) -> f64 {
    gain_db(pos).map_or(0.0, |db| 10f64.powf(db / 20.0))
}

/// Whether the key's lift is a tap: the lift of the finger that went down
/// on it (a finger slid onto the key from elsewhere is none), `inside` the
/// key (a touch stays captured by its down's target, so a finger that slid
/// off and lifted elsewhere still lifts on the key: no tap).
pub fn lift_taps(down: Option<i32>, up: i32, inside: bool) -> bool {
    inside && down == Some(up)
}

/// Whether the point (`x`, `y`) lies in the box from (`left`, `top`) to
/// (`right`, `bottom`), its edges included.
pub fn inside_box(x: f64, y: f64, (left, top, right, bottom): (f64, f64, f64, f64)) -> bool {
    (left..=right).contains(&x) && (top..=bottom).contains(&y)
}

/// The volume's place under a finger at `x` on a slider from `left`,
/// `width` wide.
pub fn volume_at(x: f64, left: f64, width: f64) -> f64 {
    if width <= 0.0 {
        return 0.0;
    }
    ((x - left) / width).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests;
