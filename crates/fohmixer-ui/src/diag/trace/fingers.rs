//! The Pro-Q 4 screen's fingers in the flight recorder (#71 PR I). The
//! owner's first iPad session (2026-10-11) left 36 contacts at the PC and
//! nothing of where each finger landed against the band points: the hub
//! records a contact's down and up, never its points, and the page recorded
//! nothing of the EQ screen. Now every finger on the screen's picture area
//! is recorded, essential like a channel detail's steps:
//!
//! - its down, up or cancel ([`eq_touch`]): the pointer, the picture point
//!   under it (the editor's pixels, through the page's view) and the view's
//!   zoom, whether the finger reached the editor or not (a pinch's fingers
//!   too, and the whole picture's taps, which zoom);
//! - its moves, summarized like a fader's `mv` ([`EqFingers::moved`]): how
//!   many, the first and the last picture point and the time between them,
//!   one `eq_mv` per [`RUN_MS`] of moving and one at its end, so a drag's
//!   path can be read back without a record per move.
//!
//! Points are rounded to 0.1 px, the zoom to 0.01. Pure, tested natively;
//! `components/eq.rs` records them through `diag::record`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::moves::round1;

/// A finger's moves are summarized at least this often while it moves (ms
/// of page time from the first move of a summary).
pub const RUN_MS: f64 = 250.0;

/// `x` to 2 decimals (a zoom).
pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Whether moves from `first` to `t` (page ms) span a whole [`RUN_MS`].
fn spans(first: f64, t: f64) -> bool {
    t - first >= RUN_MS
}

/// A finger's `what` (`down`, `up`, `cancel`) at page time `t`: its
/// pointer, its picture point `at` and the view's `zoom`.
pub fn eq_touch(t: f64, what: &str, pointer: i32, at: (f64, f64), zoom: f64) -> Value {
    json!({
        "ev": "eq_touch",
        "t": t,
        "what": what,
        "p": pointer,
        "x": round1(at.0),
        "y": round1(at.1),
        "z": round2(zoom),
    })
}

/// A finger's moves since its last summary: how many, the first and the
/// last (page ms, picture point).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    n: u32,
    first: (f64, (f64, f64)),
    last: (f64, (f64, f64)),
}

impl Run {
    /// A run of one move at `t` to `at`.
    fn start(t: f64, at: (f64, f64)) -> Self {
        Self {
            n: 1,
            first: (t, at),
            last: (t, at),
        }
    }

    /// The `eq_mv` summary of `pointer`'s run: at its first move's time,
    /// `n` moves from `a` to `b` in `ms`.
    fn summary(&self, pointer: i32) -> Value {
        let (t0, a) = self.first;
        let (t1, b) = self.last;
        json!({
            "ev": "eq_mv",
            "t": t0,
            "p": pointer,
            "n": self.n,
            "a": [round1(a.0), round1(a.1)],
            "b": [round1(b.0), round1(b.1)],
            "ms": round1(t1 - t0),
        })
    }
}

/// A finger down on the screen: its last picture point and its moves not
/// summarized yet.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Down {
    last: (f64, f64),
    run: Option<Run>,
}

/// The fingers down on the EQ screen, as the recorder keeps them.
#[derive(Debug, Default)]
pub struct EqFingers {
    down: BTreeMap<i32, Down>,
}

impl EqFingers {
    /// `pointer` went down at picture point `at` with the view at `zoom`:
    /// its event.
    pub fn down(&mut self, t: f64, pointer: i32, at: (f64, f64), zoom: f64) -> Value {
        self.down.insert(
            pointer,
            Down {
                last: at,
                run: None,
            },
        );
        eq_touch(t, "down", pointer, at, zoom)
    }

    /// `pointer` moved to picture point `at` at `t`: the summary of its
    /// moves so far once they span [`RUN_MS`] (this move starts the next);
    /// none for a pointer not down.
    pub fn moved(&mut self, t: f64, pointer: i32, at: (f64, f64)) -> Option<Value> {
        let down = self.down.get_mut(&pointer)?;
        down.last = at;
        let Some(run) = down.run.as_mut() else {
            down.run = Some(Run::start(t, at));
            return None;
        };
        if spans(run.first.0, t) {
            let summary = run.summary(pointer);
            *run = Run::start(t, at);
            return Some(summary);
        }
        run.n += 1;
        run.last = (t, at);
        None
    }

    /// `pointer`'s touch ended (`what`: `up`, `cancel`) at picture point
    /// `at` (none: where it last was, a cancel's), the view at `zoom`: the
    /// summary of its moves not summarized yet, then its event. Nothing for
    /// a pointer not down (the capture a finger loses after its lift).
    pub fn end(
        &mut self,
        t: f64,
        what: &str,
        pointer: i32,
        at: Option<(f64, f64)>,
        zoom: f64,
    ) -> Vec<Value> {
        let Some(down) = self.down.remove(&pointer) else {
            return Vec::new();
        };
        let mut events: Vec<Value> = down
            .run
            .map(|run| run.summary(pointer))
            .into_iter()
            .collect();
        events.push(eq_touch(t, what, pointer, at.unwrap_or(down.last), zoom));
        events
    }

    /// The screen goes from under the fingers (it closes, the page hides):
    /// each finger's moves, then its `cancel` at its last point.
    pub fn leave(&mut self, t: f64, zoom: f64) -> Vec<Value> {
        let down = std::mem::take(&mut self.down);
        down.into_iter()
            .flat_map(|(pointer, held)| {
                let summary = held.run.map(|run| run.summary(pointer));
                summary
                    .into_iter()
                    .chain([eq_touch(t, "cancel", pointer, held.last, zoom)])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingers_touch_is_its_pointer_picture_point_and_zoom() {
        assert_eq!(
            eq_touch(1_000.5, "down", 7, (431.04, 320.96), 2.996),
            json!({"ev": "eq_touch", "t": 1_000.5, "what": "down", "p": 7,
                   "x": 431.0, "y": 321.0, "z": 3.0})
        );
        assert_eq!(round2(2.494), 2.49);
        assert_eq!(round2(1.0), 1.0);
        assert_eq!(RUN_MS, 250.0);
        assert!(!spans(100.0, 350.0_f64.next_down()));
        assert!(spans(100.0, 350.0));
    }

    #[test]
    fn a_finger_down_moving_and_lifted_is_its_down_its_moves_and_its_up() {
        let mut fingers = EqFingers::default();
        assert_eq!(
            fingers.down(1_000.0, 3, (10.0, 20.0), 3.0),
            eq_touch(1_000.0, "down", 3, (10.0, 20.0), 3.0)
        );
        assert_eq!(fingers.moved(1_010.0, 3, (11.0, 20.0)), None);
        assert_eq!(fingers.moved(1_020.0, 3, (12.0, 21.0)), None);
        assert_eq!(fingers.moved(1_030.0, 3, (14.0, 22.0)), None);
        // Another pointer's move is nothing of it.
        assert_eq!(fingers.moved(1_031.0, 4, (90.0, 90.0)), None);
        assert_eq!(
            fingers.end(1_040.0, "up", 3, Some((14.0, 22.0)), 3.0),
            vec![
                json!({"ev": "eq_mv", "t": 1_010.0, "p": 3, "n": 3,
                       "a": [11.0, 20.0], "b": [14.0, 22.0], "ms": 20.0}),
                eq_touch(1_040.0, "up", 3, (14.0, 22.0), 3.0),
            ]
        );
        // The capture it loses after its lift ends nothing.
        assert_eq!(fingers.end(1_041.0, "cancel", 3, None, 3.0), vec![]);
        // A tap: no moves, its down and its up.
        fingers.down(2_000.0, 5, (1.0, 2.0), 1.0);
        assert_eq!(
            fingers.end(2_030.0, "up", 5, Some((1.0, 3.0)), 1.0),
            vec![eq_touch(2_030.0, "up", 5, (1.0, 3.0), 1.0)]
        );
    }

    #[test]
    fn a_long_drag_is_summarized_every_quarter_second() {
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 1, (0.0, 0.0), 2.5);
        assert_eq!(fingers.moved(100.0, 1, (1.0, 0.0)), None);
        assert_eq!(fingers.moved(200.0, 1, (2.0, 0.0)), None);
        assert_eq!(fingers.moved(349.9, 1, (3.0, 0.0)), None);
        // A whole quarter second after the run's first move: the run so far,
        // and this move starts the next.
        assert_eq!(
            fingers.moved(350.0, 1, (4.0, 0.0)),
            Some(json!({"ev": "eq_mv", "t": 100.0, "p": 1, "n": 3,
                        "a": [1.0, 0.0], "b": [3.0, 0.0], "ms": 249.9}))
        );
        assert_eq!(fingers.moved(360.0, 1, (5.0, 0.0)), None);
        assert_eq!(
            fingers.end(400.0, "cancel", 1, None, 2.5),
            vec![
                json!({"ev": "eq_mv", "t": 350.0, "p": 1, "n": 2,
                       "a": [4.0, 0.0], "b": [5.0, 0.0], "ms": 10.0}),
                eq_touch(400.0, "cancel", 1, (5.0, 0.0), 2.5),
            ]
        );
    }

    #[test]
    fn a_screen_that_goes_cancels_every_finger_at_its_last_point() {
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 2, (5.0, 5.0), 1.0);
        fingers.down(10.0, 1, (50.0, 50.0), 1.0);
        fingers.moved(20.0, 1, (52.0, 51.0));
        assert_eq!(
            fingers.leave(30.0, 1.5),
            vec![
                json!({"ev": "eq_mv", "t": 20.0, "p": 1, "n": 1,
                       "a": [52.0, 51.0], "b": [52.0, 51.0], "ms": 0.0}),
                eq_touch(30.0, "cancel", 1, (52.0, 51.0), 1.5),
                eq_touch(30.0, "cancel", 2, (5.0, 5.0), 1.5),
            ]
        );
        assert_eq!(fingers.leave(40.0, 1.0), vec![], "none left");
        assert_eq!(fingers.moved(50.0, 1, (1.0, 1.0)), None);
    }
}
