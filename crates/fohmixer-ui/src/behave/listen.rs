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
        }
    }
}

/// What the glue does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Stop the player.
    Silence,
    /// Open the audio socket.
    Open,
    /// Open it again after [`RECONNECT_MS`].
    Reopen,
    /// Close the audio socket (the hub's lease goes with it).
    Close,
    /// Send this on the open socket.
    Send(AudioClientMsg),
}

/// The page's listening: the track wanted, the phase, whether the audio
/// socket is open, and the last track's label (the foot names a track
/// taken or refused).
#[derive(Debug, Clone, PartialEq)]
pub struct Listen {
    want: Option<Target>,
    phase: Phase,
    open: bool,
    last: Option<String>,
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
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The track wanted.
    pub fn want(&self) -> Option<&Target> {
        self.want.as_ref()
    }

    /// Whether the key is lit: a track is wanted.
    pub fn lit(&self) -> bool {
        self.want.is_some()
    }

    /// The label the foot names: the wanted track's, else the last one's.
    pub fn label(&self) -> Option<&str> {
        self.want
            .as_ref()
            .map(|t| t.label.as_str())
            .or(self.last.as_deref())
    }

    /// The 🎧 key's tap on the detail of `target` (none: its name is not
    /// known, nothing to listen to); `decoder`: the browser decodes Opus.
    /// Lit, it stops; else it starts that track. (The glue starts the
    /// player inside the tap itself: a browser lets a page's audio start
    /// only in a user's gesture, and a name to read comes later.)
    pub fn tap(&mut self, target: Option<Target>, decoder: bool) -> Vec<Act> {
        if let Some(old) = self.want.take() {
            self.last = Some(old.label);
            self.phase = Phase::Off;
            self.open = false;
            return vec![Act::Close, Act::Silence];
        }
        let Some(target) = target else {
            return Vec::new();
        };
        self.last = Some(target.label.clone());
        if !decoder {
            self.phase = Phase::NoDecoder;
            return Vec::new();
        }
        self.phase = Phase::Opening;
        let send = if self.open {
            Act::Send(target.start())
        } else {
            Act::Open
        };
        self.want = Some(target);
        vec![send]
    }

    /// The detail shows `target` now: while listening, another track is
    /// followed (the mockup: picking another channel keeps listening).
    pub fn follow(&mut self, target: Target) -> Vec<Act> {
        let same = self
            .want
            .as_ref()
            .is_none_or(|t| t.instance == target.instance && t.track == target.track);
        if same {
            // Not listening, or the same track: a new label only.
            if let Some(want) = self.want.as_mut() {
                want.label = target.label;
            }
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
        self.open = true;
        match &self.want {
            Some(target) => vec![Act::Send(target.start())],
            None => {
                self.open = false;
                vec![Act::Close]
            }
        }
    }

    /// The audio socket closed: while a track is wanted it opens again.
    pub fn closed(&mut self) -> Vec<Act> {
        self.open = false;
        if self.want.is_none() {
            return Vec::new();
        }
        self.phase = Phase::Lost;
        vec![Act::Reopen]
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
            AudioState::Refused => return self.end(Phase::Refused),
            _ => {}
        }
        Vec::new()
    }

    /// Listening ends by the hub's word.
    fn end(&mut self, phase: Phase) -> Vec<Act> {
        self.last = self.want.take().map(|t| t.label);
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
