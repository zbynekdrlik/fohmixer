//! The per-track autotune decision (#140 PR 3b, spec §2.2, §7), pure: what
//! a target's `Latencies` selector should show, and why ([`decide`]), and
//! the guard against another writer on a selector ([`SelectorGuard`]).
//!
//! ```text
//! selected(track) = aut ∧ ¬held ∧ ¬unconfirmed ∧ playing ∧ song ∧ key in force
//!                 ∧ ¬singer_off ∧ ¬problem ∧ ¬another_writer ∧ armed
//!                 ∧ key not bad ∧ (already selected ∨ key confirmed)
//! ```
//!
//! The first condition that fails is the decision's why. A track the rule
//! already selected stays on while its new key's write is on its way
//! (§7: no DRY-and-back flick at a key clip) and goes DRY once that write
//! reads back wrong. It is selected only while the script holds its DRY
//! write whichever list it holds (`armed`: PR #146 review I2, the PR #147
//! ruling: the list it last kept and the newest one sent), so a hub that
//! goes away always leaves it DRY.
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
    /// Its rack is not confirmed by a walk since a hold or a track-list
    /// change, its set not held (PR #147 re-review R6: its own reason).
    pub unconfirmed: bool,
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
    /// The script holds this selector's DRY write whichever list it holds
    /// (the one it last kept and the newest one sent both have it).
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
    if input.unconfirmed {
        return off(SwitchWhy::Unconfirmed);
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
        SwitchWhy::Unconfirmed => "unconfirmed",
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
/// judged against where Live will be once the hub's writes on their way
/// land (`writing`: the last one; PR #147 review I1: a stop right after a
/// switch-on writes DRY at once), else against what Live shows (`shows`:
/// near its on value, its off value, or neither, `rules::autotune::shows`);
/// never once the hub stopped writing it (`stopped`: the guard, or Live
/// refused that write for good).
pub fn needs_write(
    decision: bool,
    shows: Option<bool>,
    writing: Option<bool>,
    stopped: bool,
) -> bool {
    !stopped && writing.map_or(shows != Some(decision), |last| last != decision)
}

/// Whether a selector's move from `before` to `now` (each where it shows:
/// near its on value, its off value, or neither) is another writer's (spec
/// §7): it moved, it is not at the decision, and it is not the echo of a
/// hub write still on its way (`flying`: their values; PR #147 review I1:
/// two may be). `Some(true)`: off DRY while the hub wants DRY;
/// `Some(false)`: off the chain while the hub wants the chain.
pub fn another_move(
    before: Option<bool>,
    now: Option<bool>,
    decision: bool,
    flying: &[bool],
) -> Option<bool> {
    let echo = now.is_some_and(|now| flying.contains(&now));
    (before != now && now != Some(decision) && !echo).then_some(!decision)
}

/// One selector write of the hub's on its way (PR #147 review I1): its
/// number, its value (`on`: the autotune chain), and whether its answer
/// came (Live has taken it then).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flying {
    pub write: u64,
    pub on: bool,
    pub answered: bool,
}

/// The writes still on their way once Live shows `now` (PR #147 review I1):
/// an answered one is taken, since a value the hub hears after a write's
/// answer is Live after it (the script sends a tick's answers before its
/// values, and every earlier value reached the hub before that answer); of
/// the others, a value at one of them lands the first one at it and every
/// one before it (Live takes them in order; PR #147 re-review R5: the
/// writes after it stay, so each later echo is still the hub's own), a
/// value at none of them lands none.
pub fn still_flying(flying: &[Flying], now: Option<bool>) -> Vec<Flying> {
    let open: Vec<Flying> = flying.iter().filter(|f| !f.answered).copied().collect();
    match now.and_then(|now| open.iter().position(|f| f.on == now)) {
        Some(at) => open[at + 1..].to_vec(),
        None => open,
    }
}

/// The writes on their way once write `write`'s answer came (`ok`: it
/// went): a failed one never lands; one that went is answered (its echo,
/// if any, is the next value).
pub fn answered_flying(flying: &[Flying], write: u64, ok: bool) -> Vec<Flying> {
    flying
        .iter()
        .filter(|f| ok || f.write != write)
        .map(|f| Flying {
            write: f.write,
            on: f.on,
            answered: f.answered || f.write == write,
        })
        .collect()
}

/// How many of one selector's writes Live may refuse in a row before the
/// hub stops writing it (PR #147 review I5, ruled: one retry).
pub const SELECTOR_RETRIES: u32 = 1;

/// A selector write Live refused (not a ref gone, not a call Live did not
/// answer: a held parameter, a refused batch): its `$ref`, the side
/// written (`on`: the autotune chain), how often in a row, and Live's
/// words. Like 1.8.0's `found_write`, the hub never writes the same value
/// to the same `$ref` again after its one retry, until a change: a new
/// `$ref`, a new decision that is no `mismatch`, `held` or `unconfirmed`
/// (PR #147 re-review R7: a key change's or a hold's brief DRY is none),
/// or the next song start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub id: String,
    pub on: bool,
    pub tries: u32,
    pub why: String,
}

impl Refusal {
    /// Another refusal of writing `id` to `on` after `prev`: the count goes
    /// on only for the same `$ref` and side.
    pub fn after(prev: Option<&Refusal>, id: &str, on: bool, why: &str) -> Self {
        let tries = prev
            .filter(|r| r.id == id && r.on == on)
            .map_or(1, |r| r.tries + 1);
        Self {
            id: id.to_string(),
            on,
            tries,
            why: why.to_string(),
        }
    }

    /// Whether it still holds for `id` and the rule's decision `on` (a new
    /// `$ref` or a new decision ends it).
    pub fn holds(&self, id: &str, on: bool) -> bool {
        self.id == id && self.on == on
    }

    /// Whether the hub stops writing `id` to `on`: the retry was refused
    /// too.
    pub fn stops(&self, id: &str, on: bool) -> bool {
        self.holds(id, on) && self.tries > SELECTOR_RETRIES
    }

    /// Whether the rule's decision `rule` for the selector `id` ends it: a
    /// new `$ref`, or another side for a reason that is no key being
    /// written (`mismatch`), no hold (`held`) and no walk awaited
    /// (`unconfirmed`).
    pub fn ends(&self, id: &str, rule: Decision) -> bool {
        let passing = matches!(
            rule.why,
            SwitchWhy::Mismatch | SwitchWhy::Held | SwitchWhy::Unconfirmed
        );
        self.id != id || (self.on != rule.on && !passing)
    }
}

/// The decision once a refusal is judged: the rule's own (`rule`), or, when
/// Live refused its chain for good, DRY with a problem (spec §7: a problem
/// is DRY; the pages show Live's words).
pub fn refused_decision(rule: Decision, refusal: Option<&Refusal>, id: &str) -> Decision {
    if rule.on && refusal.is_some_and(|r| r.stops(id, true)) {
        return Decision {
            on: false,
            why: SwitchWhy::Problem,
        };
    }
    rule
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
