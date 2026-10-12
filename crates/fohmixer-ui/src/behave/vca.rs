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
//! value under it.
//!
//! A volume's dB and a dB's volume are the strips' law, reused, never a copy
//! of the curve: [`value2db`] and [`audio_for_db_change`] from 0 dB (as
//! `scale::tick_pos` places the scale's labels), with −∞ as volume 0 and
//! +6 dB as volume 1.

use fohmixer_proto::layout::{Anchor, ParamTarget};

use super::fader::{UNITY, VolumeLaw, audio_for_db_change, value2db};

/// The law's top: Live's volume 1.0 reads +6 dB.
pub const TOP_DB: f64 = 6.0;
/// The law's floor: at or below it [`value2db`] reads −∞ (volume 0).
pub const FLOOR_DB: f64 = -70.0;
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

/// The loudest of the known volumes (the cap's); none before one is known.
pub fn loudest(volumes: &[Option<f64>]) -> Option<f64> {
    volumes.iter().flatten().copied().reduce(f64::max)
}

/// The Live volume of `db` on the strips' law: 1 at +6 dB and above, 0 at
/// the floor and below (−∞), else the volume `db` away from 0 dB.
pub fn volume_of(db: f64) -> f64 {
    if db >= TOP_DB {
        1.0
    } else if db > FLOOR_DB {
        audio_for_db_change(UNITY, db)
    } else {
        0.0
    }
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
    loudest: f64,
}

impl Drag {
    /// A drag from these volumes (every target's, in order); none without
    /// a target.
    pub fn new(start: Vec<f64>) -> Option<Self> {
        let loudest = start.iter().copied().reduce(f64::max)?;
        Some(Self { start, loudest })
    }

    /// Every target's volume with the cap at Live volume `finger`: the
    /// loudest at the finger, every other one at its start dB plus the
    /// finger's change from the loudest's start dB (silent at the law's
    /// floor). A change under [`SAME_DB`] gives the start's volumes.
    pub fn volumes(&self, finger: f64) -> Vec<f64> {
        let finger = finger.clamp(0.0, 1.0);
        let delta = value2db(finger) - value2db(self.loudest);
        if is_same(delta) {
            return self.start.clone();
        }
        self.start
            .iter()
            .map(|&volume| {
                if volume == self.loudest {
                    finger
                } else {
                    volume_of(value2db(volume) + delta)
                }
            })
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
