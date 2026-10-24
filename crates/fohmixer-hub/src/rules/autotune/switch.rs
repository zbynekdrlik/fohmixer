//! The per-track autotune decision (#140 PR 3b, spec §2.2, §7), pure: what
//! a target's `Latencies` selector should show, and why ([`decide`]), and
//! the guard against another writer on a selector ([`SelectorGuard`]).
//!
//! ```text
//! selected(track) = aut ∧ ¬held ∧ playing ∧ song ∧ key in force
//!                 ∧ ¬singer_off ∧ ¬problem ∧ ¬another_writer ∧ armed
//!                 ∧ key not bad ∧ (already selected ∨ key confirmed)
//! ```
//!
//! The first condition that fails is the decision's why. A track the rule
//! already selected stays on while its new key's write is on its way
//! (§7: no DRY-and-back flick at a key clip) and goes DRY once that write
//! reads back wrong. It is selected only while its set's registration of
//! the DRY writes is kept by the script (`armed`: PR #146 review I2), so a
//! hub that goes away always leaves it DRY.
//!
//! The router's glue is `router/autotune/switch.rs`.

use fohmixer_proto::songmap::Key;

use super::keys::{InForce, KeyState};

pub use fohmixer_proto::client::{Decision, SwitchWhy};

/// A second move of a selector by another writer within this many ms of
/// the first makes the hub stop writing it (spec §7).
pub const SELECTOR_GUARD_MS: f64 = 10_000.0;

/// Where a target's key stands against the key in force (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCheck {
    /// The key in force was written there and read back.
    Confirmed,
    /// A write on its way, none yet, another key wanted, or no key device
    /// followed now.
    Pending,
    /// The key in force was written there and read back wrong, refused, or
    /// changed by another writer since.
    Bad,
}

/// A target's key check (PR #144 review M2: `state` judges `want`, the key
/// last written, so it counts only when `want` is the key in force):
/// `device` false when no key device is followed now.
pub fn key_check(
    device: bool,
    want: Option<Key>,
    state: KeyState,
    in_force: Option<Key>,
) -> KeyCheck {
    if !device || in_force.is_none() || want != in_force {
        return KeyCheck::Pending;
    }
    match state {
        KeyState::Confirmed => KeyCheck::Confirmed,
        KeyState::Mismatch | KeyState::Refused | KeyState::AnotherWriter => KeyCheck::Bad,
        KeyState::NoKey | KeyState::Writing => KeyCheck::Pending,
    }
}

/// Everything one track's decision reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackInput {
    /// AUTOTUNE AUT.
    pub aut: bool,
    /// Its set's writes are held while its markers settle (§7).
    pub held: bool,
    /// Its set's transport (none: not heard).
    pub playing: Option<bool>,
    /// The song and the key in force at its set's playhead.
    pub in_force: InForce,
    /// Its singer is off by hand.
    pub singer_off: bool,
    /// A target problem: its key device missing, its Tuner naming two
    /// singers, its walks given up.
    pub problem: bool,
    /// Another writer moved its selector and the guard stopped the hub
    /// (or it was left DRY), until the next song start.
    pub another_writer: bool,
    /// Its set's DRY writes are kept by the script with this selector.
    pub armed: bool,
    /// Its key against the key in force.
    pub key: KeyCheck,
    /// The rule selected it already (its last decision on).
    pub selected: bool,
}

/// The decision: the first condition that fails, else on (`play`).
pub fn decide(input: &TrackInput) -> Decision {
    let off = |why| Decision { on: false, why };
    if !input.aut {
        return off(SwitchWhy::Aut);
    }
    if input.held {
        return off(SwitchWhy::Held);
    }
    if input.playing != Some(true) {
        return off(SwitchWhy::Stop);
    }
    let (song, key) = input.in_force;
    if song.is_none() {
        return off(SwitchWhy::Span);
    }
    if key.is_none() {
        return off(SwitchWhy::NoKey);
    }
    if input.singer_off {
        return off(SwitchWhy::SingerOff);
    }
    if input.problem {
        return off(SwitchWhy::Problem);
    }
    if input.another_writer {
        return off(SwitchWhy::AnotherWriter);
    }
    if !input.armed {
        return off(SwitchWhy::Unarmed);
    }
    match (input.key, input.selected) {
        (KeyCheck::Bad, _) | (KeyCheck::Pending, false) => off(SwitchWhy::Mismatch),
        (KeyCheck::Confirmed, _) | (KeyCheck::Pending, true) => Decision {
            on: true,
            why: SwitchWhy::Play,
        },
    }
}

/// A why's word in a log line (`/api/status` and the records name it so).
pub fn why_word(why: SwitchWhy) -> &'static str {
    match why {
        SwitchWhy::Aut => "aut",
        SwitchWhy::Held => "held",
        SwitchWhy::Stop => "stop",
        SwitchWhy::Span => "span",
        SwitchWhy::NoKey => "no_key",
        SwitchWhy::SingerOff => "singer_off",
        SwitchWhy::Problem => "problem",
        SwitchWhy::AnotherWriter => "another_writer",
        SwitchWhy::Unarmed => "unarmed",
        SwitchWhy::Mismatch => "mismatch",
        SwitchWhy::Play => "play",
    }
}

/// Whether a selector is written for `decision` (one write per change):
/// Live shows otherwise (`on_now`), no write of it is on its way
/// (`writing`), and the guard has not stopped the hub.
pub fn needs_write(decision: bool, on_now: bool, writing: Option<bool>, given_up: bool) -> bool {
    !given_up && decision != on_now && writing != Some(decision)
}

/// Whether a selector's move from `before` to `now` (each: it shows the
/// autotune chain) is another writer's (spec §7): it moved, away from the
/// decision, and it is not the echo of the hub's own write on its way
/// (`writing`). `Some(now)`: where it went.
pub fn another_move(
    before: bool,
    now: bool,
    decision: bool,
    writing: Option<bool>,
) -> Option<bool> {
    (before != now && now != decision && writing != Some(now)).then_some(now)
}

/// What the guard does with another writer's move to the autotune chain
/// while the hub wants DRY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAct {
    /// DRY again, once.
    WriteDryOnce,
    /// A second move within [`SELECTOR_GUARD_MS`]: the hub stops writing
    /// and shows the track's problem until the next song start.
    GiveUp,
}

/// One selector's guard against another writer (spec §7). DRY always
/// wins: a move to the autotune chain while the hub wants DRY is written
/// DRY again once, a second one within [`SELECTOR_GUARD_MS`] stops the
/// hub; a move to DRY while the hub wants the chain is left as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SelectorGuard {
    /// When the move written DRY again was heard (ms).
    first: Option<f64>,
    given_up: bool,
    left_dry: bool,
}

impl SelectorGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Another writer moved the selector to the autotune chain at `now_ms`
    /// while the hub wants DRY.
    pub fn moved(&mut self, now_ms: f64) -> GuardAct {
        let again = self.given_up
            || self
                .first
                .is_some_and(|first| now_ms - first <= SELECTOR_GUARD_MS);
        if again {
            self.given_up = true;
            return GuardAct::GiveUp;
        }
        self.first = Some(now_ms);
        GuardAct::WriteDryOnce
    }

    /// Another writer moved it to DRY while the hub wants the chain: left
    /// so (DRY is safe; the hub never fights to turn autotune on).
    pub fn left(&mut self) {
        self.left_dry = true;
    }

    /// The hub stopped writing it.
    pub fn given_up(&self) -> bool {
        self.given_up
    }

    /// Another writer holds it (given up, or left DRY): the decision's
    /// `another_writer`.
    pub fn another(&self) -> bool {
        self.given_up || self.left_dry
    }

    /// The next song start: the hub decides it afresh.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
