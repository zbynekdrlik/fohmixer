//! The Claps strip's left column (#220; the owner's approved mockup
//! `docs/mockups/claps-v2.html`): the calm state line, the four beat dots
//! and TAP. Everything the hub decides (the taps' beats, the bar, the clip,
//! 4/4) comes in its `claps` message (`fohmixer_proto::claps::ClapsState`);
//! here only what the page draws from it between messages: the words, the
//! "now" ring moving with Live's playhead at its tempo, the clap's flash,
//! and what a dot's tap asks.

use fohmixer_proto::claps::{BEATS, ClapsPhase, ClapsState};

/// A dot lights as Live claps for this much of a beat.
pub const HIT_BEATS: f64 = 0.24;
/// The ring is drawn from a told playhead at most this long (ms): past it
/// the page has heard nothing for too long to guess Live's beat.
pub const RING_MS: f64 = 30_000.0;

/// The state line's words: nothing while nothing claps.
pub fn state_text(phase: ClapsPhase) -> &'static str {
    match phase {
        ClapsPhase::Off => "",
        ClapsPhase::Tapping => "TAPPING",
        ClapsPhase::NextBar => "NEXT BAR",
        ClapsPhase::On => "CLAPS",
        ClapsPhase::Clip => "CLIP",
        ClapsPhase::Waits => "WAITS",
        ClapsPhase::Signature => "4/4 only",
    }
}

/// Whether the line is lit (still calm text, a step brighter): only while
/// the engineer taps, the pattern waits for its bar, or Live claps it.
pub fn state_lit(phase: ClapsPhase) -> bool {
    matches!(
        phase,
        ClapsPhase::Tapping | ClapsPhase::NextBar | ClapsPhase::On
    )
}

/// The phase's name for `data-state`.
pub fn phase_name(phase: ClapsPhase) -> &'static str {
    match phase {
        ClapsPhase::Off => "off",
        ClapsPhase::Tapping => "tapping",
        ClapsPhase::NextBar => "next_bar",
        ClapsPhase::On => "on",
        ClapsPhase::Clip => "clip",
        ClapsPhase::Waits => "waits",
        ClapsPhase::Signature => "signature",
    }
}

/// Live's playhead now (beats), moved on from the one told at page time
/// `at` (ms) at its tempo; none without one, without a tempo, or from one
/// told more than [`RING_MS`] ago.
pub fn position(state: &ClapsState, at: f64, now: f64) -> Option<f64> {
    let (beat, tempo) = (state.beat?, state.tempo?);
    let dt = now - at;
    (tempo.is_finite() && tempo > 0.0 && (0.0..=RING_MS).contains(&dt))
        .then(|| beat + dt * tempo / 60_000.0)
}

/// The beat of the bar the ring shows (0 to 3), while the column says
/// something moves on the beats (tapping, the next bar, clapping, a clip);
/// none else.
pub fn ring(state: &ClapsState, at: f64, now: f64) -> Option<usize> {
    let moving = matches!(
        state.state,
        ClapsPhase::Tapping | ClapsPhase::NextBar | ClapsPhase::On | ClapsPhase::Clip
    );
    let pos = position(state, at, now).filter(|_| moving)?;
    Some((pos.floor() as i64).rem_euclid(BEATS as i64) as usize)
}

/// Whether the ring's dot flashes now: Live claps that beat (on, the beat
/// in the pattern) and the beat began less than [`HIT_BEATS`] ago.
pub fn hit(state: &ClapsState, at: f64, now: f64) -> bool {
    if state.state != ClapsPhase::On {
        return false;
    }
    let (Some(pos), Some(beat)) = (position(state, at, now), ring(state, at, now)) else {
        return false;
    };
    state.pattern[beat] && pos - pos.floor() < HIT_BEATS
}

/// What a tap of dot `beat` asks: the beat switched the other way.
pub fn dot_tap(pattern: &[bool; BEATS], beat: usize) -> bool {
    !pattern.get(beat).copied().unwrap_or(false)
}

/// What a 1 s hold of TAP may do now: stop, while the page is connected
/// and the hub told this connection its claps (none: refused).
pub fn stop_value(told: bool, connected: bool) -> Option<()> {
    (told && connected).then_some(())
}

#[cfg(test)]
mod tests;
