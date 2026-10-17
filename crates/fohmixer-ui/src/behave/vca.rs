//! The backing-tracks fader (#58 item 4; the owner's approval, comment
//! 6125310190, of the mockup `docs/mockups/podklady-fader-v1.html`): a
//! `param_fader` whose targets are two or more track volumes. The backing
//! tracks feed the main speakers and the band's in-ear monitors alike, so
//! the fader moves the tracks' own volumes (never a group track's). It
//! shows every track on the strips' dB law and scale, one groove each, its
//! cap at the loudest; a drag moves every track by the same number of dB
//! (a console VCA's, the owner's option A), so a balance the Live set gave
//! them (its prayer mode) stays; while any track is off 0 dB the strip is
//! amber with `≠ 0 dB`, the loudest track's dB large and every track's
//! value under it. A double tap puts every track exactly at 0 dB (the
//! owner's decision of 2026-10-13, comment 6139533903; it no longer keeps
//! the balance): one gesture always brings the backing tracks back to
//! normal, after the prayer mode and after a track was moved on its own
//! ([`Home`]).
//!
//! A volume's dB and a dB's volume are the strips' law, reused, never a copy
//! of the curve (since #130 in `fohmixer_proto::volume`, shared with the
//! hub): [`value2db`] and `audio_for_db_change` from 0 dB (as
//! `scale::tick_pos` places the scale's labels), with −∞ as volume 0 and
//! +6 dB as volume 1.

use fohmixer_proto::layout::{Anchor, ParamTarget};

use super::fader::{UNITY, VolumeLaw, value2db};

// The law's ends and a dB's volume, shared with the hub (#130,
// `fohmixer_proto::volume`).
pub use fohmixer_proto::volume::{FLOOR_DB, TOP_DB, volume_of};
/// A track this far off 0 dB, or further, highlights the fader (the
/// mockup's rule: its readout would not read `0.0`).
pub const MOVED_DB: f64 = 0.05;
/// A level at or below this reads `-∞` (the mockup's: what would read
/// −70.0 is silence).
pub const SILENT_DB: f64 = -69.95;
/// A drag's change under this many dB is the float noise of the finger's
/// summed moves, not a move: the touch's start values go back unchanged, so
/// a drag that comes back to where it started restores Live's values
/// exactly (the dB → volume bisection would not).
pub const SAME_DB: f64 = 1e-6;
/// The readout's values per line (the mockup: three, then two for five
/// tracks).
pub const PER_LINE: usize = 3;
/// A track whose value has not come yet, in the readout.
pub const NO_VALUE: &str = "–";

/// Whether a `param_fader` target is a track's volume: a track (by name, or
/// by index for a marker), its `mixer_device volume`, the `value`.
pub fn is_track_volume(target: &ParamTarget) -> bool {
    let track = matches!(
        target.binding.anchor,
        Anchor::Track { .. } | Anchor::TrackAt { .. }
    );
    let volume = target
        .binding
        .path
        .as_deref()
        .is_some_and(|path| path.split_whitespace().eq(["mixer_device", "volume"]));
    track && volume && target.prop == "value"
}

/// Whether a `param_fader` is the backing-tracks fader: two or more
/// targets, every one a track's volume. Any other (a parameter, a mix of
/// kinds, a single volume) keeps the linear fader of its first target.
pub fn is_vca(targets: &[ParamTarget]) -> bool {
    targets.len() >= 2 && targets.iter().all(is_track_volume)
}

/// Whether a `param_fader` draws as the backing-tracks fader: a linked one
/// (#130, D18: Podklady All, whatever its partners) or two or more track
/// volumes (#58).
pub fn draws_vca(linked: bool, targets: &[ParamTarget]) -> bool {
    linked || is_vca(targets)
}

/// The loudest of the known volumes (the cap's); none before one is known.
pub fn loudest(volumes: &[Option<f64>]) -> Option<f64> {
    volumes.iter().flatten().copied().reduce(f64::max)
}

/// Whether a level is off 0 dB enough to show: the amber highlight.
pub fn is_moved(db: f64) -> bool {
    db.abs() >= MOVED_DB
}

/// Whether a level reads `-∞`.
pub fn is_silent(db: f64) -> bool {
    db <= SILENT_DB
}

/// Whether a drag's change of `delta` dB is no change (float noise).
fn is_same(delta: f64) -> bool {
    delta.abs() < SAME_DB
}

/// One touch's drag: every target's volume at the touch's start (its open
/// write, else Live's), so each move is computed from that start: a track
/// a move pushed below the floor comes back to its own level when the
/// finger does.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    start: Vec<f64>,
    /// The track at the finger: the loudest (#58) or the touched strip's
    /// own (#130).
    anchor: f64,
}

impl Drag {
    /// A drag from these volumes (every target's, in order); none without
    /// a target.
    pub fn new(start: Vec<f64>) -> Option<Self> {
        let anchor = start.iter().copied().reduce(f64::max)?;
        Some(Self::around(start, anchor))
    }

    /// A drag whose `anchor` (a start volume) follows the finger: the fader
    /// link's own track (#130, D18).
    pub fn around(start: Vec<f64>, anchor: f64) -> Self {
        Self { start, anchor }
    }

    /// Every target's volume with the cap at Live volume `finger`: the
    /// anchor at the finger, every other one at its start dB plus the
    /// finger's change from the anchor's start dB (silent at the law's
    /// floor). A change under [`SAME_DB`] gives the start's volumes.
    pub fn volumes(&self, finger: f64) -> Vec<f64> {
        let finger = finger.clamp(0.0, 1.0);
        let delta = value2db(finger) - value2db(self.anchor);
        if is_same(delta) {
            return self.start.clone();
        }
        self.start
            .iter()
            .map(|&volume| {
                if volume == self.anchor {
                    finger
                } else {
                    volume_of(value2db(volume) + delta)
                }
            })
            .collect()
    }
}

/// The double tap's glide (the owner's decision of 2026-10-13, #58 comment
/// 6139533903): every target from its volume at the glide's start to Live's
/// 0 dB volume, whatever balance it had (a prayer mode's, or a track moved
/// on its own). The tracks move on the strips' law's positions, each the
/// same share of its own way at once: so every groove slides as a strip's
/// cap glides, −∞ is simply position 0, the loudest stays the loudest (the
/// cap's), and both ends are exact. The glide lasts as long as a strip's
/// over the furthest track's way ([`Home::span`]), so all arrive together
/// and none moves faster than a strip's glide.
#[derive(Debug, Clone, PartialEq)]
pub struct Home {
    law: VolumeLaw,
    /// Every target's position on the law at the glide's start.
    start: Vec<f64>,
    /// The loudest's start position (the cap's).
    from: f64,
    /// The furthest any track has to go to 0 dB, in positions.
    span: f64,
}

impl Home {
    /// The glide from these volumes (every target's, in order) on `law`;
    /// none without a target or while one has no value.
    pub fn new(law: VolumeLaw, volumes: &[Option<f64>]) -> Option<Self> {
        let start: Vec<f64> = volumes
            .iter()
            .map(|volume| volume.map(|v| law.to_pos(v)))
            .collect::<Option<_>>()?;
        let unity = law.to_pos(UNITY);
        let from = start.iter().copied().reduce(f64::max)?;
        let span = start
            .iter()
            .map(|&pos| (unity - pos).abs())
            .fold(0.0, f64::max);
        Some(Self {
            law,
            start,
            from,
            span,
        })
    }

    /// The cap's start: the loudest track's position.
    pub fn cap_from(&self) -> f64 {
        self.from
    }

    /// The furthest track's way to 0 dB (positions): the glide lasts as
    /// long as a strip's glide over it.
    pub fn span(&self) -> f64 {
        self.span
    }

    /// Every target's volume `p` of the way (0 to 1, clamped): each at
    /// that share of the way from its start position to 0 dB's (what is
    /// left of its way shrinks with `1 − p`), so the start's volumes at 0
    /// and, at 1, exactly the volume a strip's double tap reaches (its
    /// glide's end, `to_pos(UNITY)`, on the same law). A track already at
    /// 0 dB stays exactly there all the way (no float wobble around it).
    pub fn volumes(&self, p: f64) -> Vec<f64> {
        let p = p.clamp(0.0, 1.0);
        let unity = self.law.to_pos(UNITY);
        self.start
            .iter()
            .map(|&pos| self.law.to_live(unity - (unity - pos) * (1.0 - p)))
            .collect()
    }
}

/// The loudest track's readout, large (the mockup's `0.0 dB`, `-14.0 dB`,
/// `+2.0 dB`, `-∞ dB`): one decimal with its sign, `0.0` within the
/// highlight's 0.05 dB of 0 dB.
pub fn loudest_text(db: f64) -> String {
    if is_silent(db) {
        "-∞ dB".to_string()
    } else if is_moved(db) {
        format!("{db:+.1} dB")
    } else {
        "0.0 dB".to_string()
    }
}

/// A track's value under the large readout, in whole dB as the mockup
/// rounds it (half up: `-14`, `0`, `2`, `-∞`); [`NO_VALUE`] before Live's.
pub fn value_text(db: Option<f64>) -> String {
    match db {
        None => NO_VALUE.to_string(),
        Some(db) if is_silent(db) => "-∞".to_string(),
        Some(db) => format!("{}", (db + 0.5).floor()),
    }
}

/// Every track's value in order, [`PER_LINE`] to a line.
pub fn values_text(dbs: &[Option<f64>]) -> String {
    dbs.chunks(PER_LINE)
        .map(|line| {
            line.iter()
                .map(|db| value_text(*db))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One groove of the fader.
#[derive(Debug, Clone, PartialEq)]
pub struct Groove {
    /// Its fill: the track's position on the law (0 before a value).
    pub pos: f64,
    /// Its `data-db`: the track's dB to three decimals, `-inf` for silence,
    /// empty before a value.
    pub db: String,
    /// The track is off 0 dB: its fill is amber.
    pub moved: bool,
}

/// What the fader shows of its targets' volumes (each `None` before
/// Live's value).
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// Any track off 0 dB: the amber strip and its `≠ 0 dB` badge.
    pub moved: bool,
    /// The loudest track's readout (empty before a value).
    pub loudest: String,
    /// Every track's value, [`values_text`].
    pub values: String,
    pub grooves: Vec<Groove>,
}

/// The look of `volumes` on the strips' law `law`.
pub fn look(law: VolumeLaw, volumes: &[Option<f64>]) -> Look {
    let dbs: Vec<Option<f64>> = volumes.iter().map(|v| v.map(value2db)).collect();
    let grooves = volumes
        .iter()
        .zip(&dbs)
        .map(|(volume, db)| Groove {
            pos: volume.map_or(0.0, |v| law.to_pos(v)),
            db: db.map_or_else(String::new, |db| format!("{db:.3}")),
            moved: db.is_some_and(is_moved),
        })
        .collect();
    Look {
        moved: dbs.iter().flatten().any(|db| is_moved(*db)),
        loudest: loudest(volumes).map_or_else(String::new, |v| loudest_text(value2db(v))),
        values: values_text(&dbs),
        grooves,
    }
}

#[cfg(test)]
mod tests;
