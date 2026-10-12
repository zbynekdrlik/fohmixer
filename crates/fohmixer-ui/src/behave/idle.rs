//! The page's idle state (#51; design draft §2, and the owner's decision of
//! 2026-10-12: touches only, no motion sensor, and a hidden page saves at
//! once). Two states: Active, everything as before, and Idle, where the page
//! saves (the hub pauses or paces its meters; the link waits longer,
//! `store/conn.rs` and `behave/link.rs`).
//!
//! - **Hidden** (`document.hidden`: a locked iPad, the app in the
//!   background) makes the page Idle at once with its meters paused
//!   ([`meter_ms`] 0): the `visibilitychange` handler sends it before iOS
//!   freezes the page. A page idle for no touch that is then hidden goes
//!   Idle again, paused.
//! - **Shown again**, it wakes (`visible`) and the countdown starts over.
//! - **Touch-idle:** after the layout's `idle_after_s` ([`idle_after_ms`];
//!   absent or 0: never) with no pointer down while visible, the page goes
//!   Idle with its meters paced (the layout's `idle_meter_ms`, 250 ms when
//!   absent or 0: never paused). Something that holds the page (a finger
//!   down, an open Pro-Q 4 screen, a pending EQ input) keeps it Active, and
//!   each tick it holds starts the countdown over; any pointer down wakes it
//!   (`touch`), and touch-idle switched off wakes it (`config`). A hidden
//!   page stays Idle until it is shown.
//!
//! Pure, tested natively; `store/live/idle.rs` carries it out. Times are the
//! page's clock in ms since the epoch (the clock of `set` and `ping`).

use fohmixer_proto::client::IDLE_METER_MS;
use serde_json::{Value, json};

/// Why the page went idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sleep {
    /// It is hidden: a locked iPad, the app in the background.
    Hidden,
    /// No pointer went down for the layout's `idle_after_s`.
    Touch,
}

impl Sleep {
    /// Its name in the `idle` event.
    pub fn name(self) -> &'static str {
        match self {
            Sleep::Hidden => "hidden",
            Sleep::Touch => "touch",
        }
    }
}

/// Why the page woke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// A pointer went down.
    Touch,
    /// It was shown again.
    Visible,
    /// The layout switched touch-idle off.
    Config,
}

impl Wake {
    /// Its name in the `idle` event.
    pub fn name(self) -> &'static str {
        match self {
            Wake::Touch => "touch",
            Wake::Visible => "visible",
            Wake::Config => "config",
        }
    }
}

/// A change of the page's state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Turn {
    /// The page goes idle (or, idle for no touch, is now hidden too).
    Sleep(Sleep),
    /// It wakes after `idle_ms` idle.
    Wake { why: Wake, idle_ms: f64 },
}

impl Turn {
    /// Its event for the flight recorder at `t` (essential): `on`, why, and
    /// at the wake how long the page was idle (whole ms).
    pub fn event(self, t: f64) -> Value {
        match self {
            Turn::Sleep(why) => json!({"ev": "idle", "t": t, "on": true, "why": why.name()}),
            Turn::Wake { why, idle_ms } => json!({
                "ev": "idle",
                "t": t,
                "on": false,
                "why": why.name(),
                "ms": idle_ms.round(),
            }),
        }
    }
}

/// Touch-idle's countdown from the layout's `idle_after_s`: none (never idle
/// for no touch) when absent or 0.
pub fn idle_after_ms(setting: Option<u32>) -> Option<f64> {
    setting.filter(|&s| s != 0).map(|s| f64::from(s) * 1000.0)
}

/// The meter period an idle page asks the hub for (`idle`'s `meter_ms`): 0,
/// no meters until it wakes, only when hidden; for no touch the layout's
/// `idle_meter_ms`, [`IDLE_METER_MS`] when absent or 0 (the hub clamps the
/// rest).
pub fn meter_ms(why: Sleep, setting: Option<u32>) -> u32 {
    match why {
        Sleep::Hidden => 0,
        Sleep::Touch => setting.filter(|&ms| ms != 0).unwrap_or(IDLE_METER_MS),
    }
}

/// Whether a countdown of `after` ms that started at `last` is over at `now`.
pub fn due(now: f64, last: f64, after: f64) -> bool {
    now - last >= after
}

/// The page's idle state.
#[derive(Debug, Clone, PartialEq)]
pub struct IdleWatch {
    /// Touch-idle's countdown started here: the last pointer down, the last
    /// tick something held the page or touch-idle was off, the page shown.
    last: f64,
    /// While idle: since when, and why (the latest reason).
    idle: Option<(f64, Sleep)>,
}

impl IdleWatch {
    /// An Active page whose countdown starts at `now`.
    pub fn new(now: f64) -> Self {
        Self {
            last: now,
            idle: None,
        }
    }

    /// Whether the page is idle.
    pub fn is_idle(&self) -> bool {
        self.idle.is_some()
    }

    /// Why the page is idle; none while it is Active.
    pub fn sleeping(&self) -> Option<Sleep> {
        self.idle.map(|(_, why)| why)
    }

    /// The page is hidden or not at `now` (its `visibilitychange`, and each
    /// tick in case one was missed): hidden makes it Idle at once (a page
    /// idle for no touch goes Idle again, paused); shown, a hidden page
    /// wakes and the countdown starts over. No change: nothing.
    pub fn visibility(&mut self, now: f64, hidden: bool) -> Option<Turn> {
        match (hidden, self.idle) {
            (true, Some((_, Sleep::Hidden))) | (false, None | Some((_, Sleep::Touch))) => None,
            (true, idle) => {
                let since = idle.map_or(now, |(since, _)| since);
                self.idle = Some((since, Sleep::Hidden));
                Some(Turn::Sleep(Sleep::Hidden))
            }
            (false, Some((_, Sleep::Hidden))) => {
                self.last = now;
                self.wake_from(Sleep::Hidden, now, Wake::Visible)
            }
        }
    }

    /// A pointer went down at `now` (any pointer, anywhere on the page): the
    /// countdown starts over and a page idle for no touch wakes. A hidden
    /// page stays idle until it is shown.
    pub fn touched(&mut self, now: f64) -> Option<Turn> {
        self.last = now;
        self.wake_from(Sleep::Touch, now, Wake::Touch)
    }

    /// The link's tick at `now`: `after` is touch-idle's countdown
    /// ([`idle_after_ms`]; none: off), `held` whether something holds the
    /// page Active (a finger down, an open Pro-Q 4 screen, a pending EQ
    /// input). Active, a tick that holds or with touch-idle off starts the
    /// countdown over, and a due one makes the page Idle; idle for no touch,
    /// touch-idle switched off wakes it.
    pub fn tick(&mut self, now: f64, after: Option<f64>, held: bool) -> Option<Turn> {
        match (self.idle, after) {
            (Some(_), Some(_)) => None,
            (Some(_), None) => self.wake_from(Sleep::Touch, now, Wake::Config),
            (None, Some(after)) if !held => due(now, self.last, after).then(|| {
                self.idle = Some((now, Sleep::Touch));
                Turn::Sleep(Sleep::Touch)
            }),
            (None, _) => {
                self.last = now;
                None
            }
        }
    }

    /// A page idle for `from` wakes at `now` for `why`; any other state
    /// stays.
    fn wake_from(&mut self, from: Sleep, now: f64, why: Wake) -> Option<Turn> {
        let (since, _) = self.idle.filter(|&(_, asleep)| asleep == from)?;
        self.idle = None;
        Some(Turn::Wake {
            why,
            idle_ms: now - since,
        })
    }
}

#[cfg(test)]
#[path = "idle/tests.rs"]
mod tests;
