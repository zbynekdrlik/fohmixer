//! The Claps strip's left column (#220; the owner's approved mockup
//! `docs/mockups/claps-v2.html`; #260's v3 removed the beat dots): the
//! calm state line and TAP. Everything the hub decides (the taps' beats, the
//! bar, the clip, 4/4) comes in its `claps` message
//! (`fohmixer_proto::claps::ClapsState`);
//! here only what the page draws from it between messages: the words and
//! TAP's flash at each clap, timed by Live's playhead moved on at its tempo.

use fohmixer_proto::claps::{BEATS, ClapsPhase, ClapsState};

/// TAP lights as Live claps for this much of a beat.
pub const HIT_BEATS: f64 = 0.24;
/// TAP flashes this long (ms) after the lift of each tap, so the engineer
/// sees every touch land (#262), whatever the hub makes of it.
pub const TAP_FLASH_MS: f64 = 160.0;
/// Live's beat is moved on from a told playhead at most this long (ms):
/// past it the page has heard nothing for too long to guess it.
pub const GUESS_MS: f64 = 30_000.0;

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
/// told more than [`GUESS_MS`] ago.
pub fn position(state: &ClapsState, at: f64, now: f64) -> Option<f64> {
    let (beat, tempo) = (state.beat?, state.tempo?);
    let dt = now - at;
    (tempo.is_finite() && tempo > 0.0 && (0.0..=GUESS_MS).contains(&dt))
        .then(|| beat + dt * tempo / 60_000.0)
}

/// The beat of Live's bar now (0 to 3), while the column says something
/// moves on the beats (tapping, the next bar, clapping, a clip); none else.
pub fn beat_now(state: &ClapsState, at: f64, now: f64) -> Option<usize> {
    let moving = matches!(
        state.state,
        ClapsPhase::Tapping | ClapsPhase::NextBar | ClapsPhase::On | ClapsPhase::Clip
    );
    let pos = position(state, at, now).filter(|_| moving)?;
    Some((pos.floor() as i64).rem_euclid(BEATS as i64) as usize)
}

/// Whether TAP flashes now: Live claps this beat (on, the beat in the
/// pattern) and the beat began less than [`HIT_BEATS`] ago.
pub fn hit(state: &ClapsState, at: f64, now: f64) -> bool {
    if state.state != ClapsPhase::On {
        return false;
    }
    let (Some(pos), Some(beat)) = (position(state, at, now), beat_now(state, at, now)) else {
        return false;
    };
    state.pattern[beat] && pos - pos.floor() < HIT_BEATS
}

/// Whether TAP shows the flash of a tap lifted at `tapped` (page epoch ms)
/// at `now`: from the lift for [`TAP_FLASH_MS`].
pub fn tap_flash(tapped: Option<f64>, now: f64) -> bool {
    tapped.is_some_and(|t| now >= t && now - t < TAP_FLASH_MS)
}

/// What a 1 s hold of TAP may do now: stop, while the page is connected
/// and the hub told this connection its claps (none: refused).
pub fn stop_value(told: bool, connected: bool) -> Option<()> {
    (told && connected).then_some(())
}

#[cfg(test)]
mod tests;
