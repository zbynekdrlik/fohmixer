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
//!   too, and the whole picture's taps, which zoom); `lost` (#124) for a
//!   finger whose end the browser never delivered, ended by the next
//!   primary down of its type or by a hidden page;
//! - its moves, summarized like a fader's `mv` ([`EqFingers::moved`]): how
//!   many, the first and the last picture point and the time between them,
//!   one `eq_mv` per [`RUN_MS`] of moving and one at its end, so a drag's
//!   path can be read back without a record per move.
//!
//! Points are rounded to 0.1 px, the zoom to 0.01. Pure, tested natively;
//! `components/eq.rs` records them through `diag::record`.
//!
//! The bar's ↶ VRÁTIŤ (#71 PR P2b) is recorded too: each press the hub
//! answered, as [`eq_undo`] (its input number, the answer and the ms from
//! the press to it), kept by [`EqUndos`] until the answer comes.

use std::collections::{BTreeMap, VecDeque};

use fohmixer_proto::eq::Part;
use serde_json::{Value, json};

use super::moves::round1;
use crate::behave::Down;
use crate::diag::perf::Pointer;

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

/// A finger's `what` (`down`, `up`, `cancel`, `lost`) at page time `t`: its
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

/// The part of the picture the page asked the hub for at `t` (#71 PR L):
/// its crop and the picture's size it is of, none for the whole picture,
/// in the shape of the hub's `view` record. Essential: the timeline tells
/// whole frames from parts.
pub fn eq_view(t: f64, part: Option<Part>) -> Value {
    json!({"ev": "eq_view", "t": t, "part": part})
}

/// The bar's ↶ VRÁTIŤ pressed at page time `t` (#71 PR P2b): the undo's
/// input number `n`, the hub's answer (`result`: `done`, `nothing` or why
/// not) and the ms from the press to the answer. Essential: the timeline
/// shows it beside the hub's `undo` record.
pub fn eq_undo(t: f64, n: u32, result: &str, ms: f64) -> Value {
    json!({"ev": "eq_undo", "t": t, "n": n, "result": result, "ms": round1(ms)})
}

/// The presses kept waiting for their answer (the oldest goes first): the
/// control takes one each 700 ms and the hub answers each at once, so a
/// few at most.
pub const UNDOS_KEPT: usize = 8;

/// The bar's undo presses waiting for the hub's answer (#71 PR P2b): each
/// one's input number and its page time.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EqUndos {
    waiting: VecDeque<(u32, f64)>,
}

impl EqUndos {
    /// The undo numbered `n` went at page time `t`.
    pub fn pressed(&mut self, n: u32, t: f64) {
        if self.waiting.len() >= UNDOS_KEPT {
            self.waiting.pop_front();
        }
        self.waiting.push_back((n, t));
    }

    /// The hub answered the undo numbered `n` with `result` at page time
    /// `t`: its [`eq_undo`] event, at its press's time; none for one this
    /// page does not wait for (answered already, or long gone).
    pub fn answered(&mut self, n: u32, result: &str, t: f64) -> Option<Value> {
        let at = self.waiting.iter().position(|(m, _)| *m == n)?;
        let (_, pressed) = self.waiting.remove(at)?;
        Some(eq_undo(pressed, n, result, t - pressed))
    }
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

/// A finger down on the screen: its type (#124: a primary down of that
/// type ends it, [`EqFingers::missed`]), its last picture point and its
/// moves not summarized yet.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    kind: Option<Pointer>,
    last: (f64, f64),
    run: Option<Run>,
}

/// The fingers down on the EQ screen, as the recorder keeps them.
#[derive(Debug, Default)]
pub struct EqFingers {
    down: BTreeMap<i32, Held>,
}

impl EqFingers {
    /// `pointer` (of type `kind`) went down at picture point `at` with the
    /// view at `zoom`: its event.
    pub fn down(
        &mut self,
        t: f64,
        pointer: i32,
        kind: Option<Pointer>,
        at: (f64, f64),
        zoom: f64,
    ) -> Value {
        self.down.insert(
            pointer,
            Held {
                kind,
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

    /// `pointer`'s touch ended (`what`: `up`, `cancel`, `lost`) at picture
    /// point `at` (none: where it last was, a cancel's), the view at `zoom`: the
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

    /// The fingers whose end never came (#124) that `by`, a pointer's down
    /// at `t`, ends: when it is primary, every finger of its type this
    /// recorder keeps (`behave::Down::ends`, the screen's fingers' own
    /// rule), a third finger the screen's fingers never took and one whose
    /// contact a dropout cut included: each one's moves, then `lost` at its
    /// last point. A second finger, or a pointer of another type, ends none.
    pub fn missed(&mut self, t: f64, by: Down, zoom: f64) -> Vec<Value> {
        let lost: Vec<i32> = self
            .down
            .iter()
            .filter(|(_, held)| by.ends(held.kind))
            .map(|(pointer, _)| *pointer)
            .collect();
        lost.into_iter()
            .flat_map(|pointer| self.end(t, "lost", pointer, None, zoom))
            .collect()
    }

    /// `pointer`'s contact with the editor ended at `t` while the finger
    /// stays down (a dropout under it, #71 PR J): its moves, then its
    /// `cancel` at its last point. The finger stays kept: its own end, a
    /// primary down of its type or the screen's going still gives it its
    /// row; none for a pointer not down.
    pub fn cut(&mut self, t: f64, pointer: i32, zoom: f64) -> Vec<Value> {
        let Some(held) = self.down.get_mut(&pointer) else {
            return Vec::new();
        };
        let mut events: Vec<Value> = held
            .run
            .take()
            .map(|run| run.summary(pointer))
            .into_iter()
            .collect();
        events.push(eq_touch(t, "cancel", pointer, held.last, zoom));
        events
    }

    /// The screen goes from under the fingers (it closes: `what` is
    /// `cancel`; the page hides: `lost`, #124, a hidden Home Screen app
    /// keeps no touch going): each finger's moves, then its `what` at its
    /// last point.
    pub fn leave(&mut self, t: f64, what: &str, zoom: f64) -> Vec<Value> {
        let down = std::mem::take(&mut self.down);
        down.into_iter()
            .flat_map(|(pointer, held)| {
                let summary = held.run.map(|run| run.summary(pointer));
                summary
                    .into_iter()
                    .chain([eq_touch(t, what, pointer, held.last, zoom)])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finger's type.
    const TOUCH: Option<Pointer> = Some(Pointer::Touch);

    #[test]
    fn a_view_is_its_part_or_the_whole_picture() {
        let part = Part {
            crop: fohmixer_proto::eq::Crop {
                x: 96,
                y: 48,
                w: 320,
                h: 160,
            },
            picture: (1349, 809),
        };
        assert_eq!(
            eq_view(5.0, Some(part)),
            json!({
                "ev": "eq_view",
                "t": 5.0,
                "part": {"crop": {"x": 96, "y": 48, "w": 320, "h": 160}, "picture": [1349, 809]}
            })
        );
        assert_eq!(
            eq_view(6.0, None),
            json!({"ev": "eq_view", "t": 6.0, "part": null})
        );
    }

    #[test]
    fn an_undo_is_recorded_at_its_answer_with_its_press_time_and_its_wait() {
        // #71 PR P2b.
        let mut undos = EqUndos::default();
        assert_eq!(undos.answered(4, "done", 10.0), None, "nothing pressed");
        undos.pressed(4, 1_000.0);
        undos.pressed(5, 1_800.0);
        // Answered out of order, each finds its own press.
        assert_eq!(
            undos.answered(5, "nothing", 1_850.04),
            Some(
                json!({"ev": "eq_undo", "t": 1_800.0, "n": 5, "result": "nothing",
                        "ms": 50.0})
            )
        );
        assert_eq!(
            undos.answered(4, "too soon after the last undo", 1_900.0),
            Some(eq_undo(1_000.0, 4, "too soon after the last undo", 900.0))
        );
        // Each answer once.
        assert_eq!(undos.answered(4, "done", 2_000.0), None);
        assert_eq!(
            eq_undo(7.5, 2, "done", 61.04),
            json!({"ev": "eq_undo", "t": 7.5, "n": 2, "result": "done", "ms": 61.0})
        );
    }

    #[test]
    fn at_most_the_last_8_presses_wait_for_their_answer() {
        assert_eq!(UNDOS_KEPT, 8);
        let mut undos = EqUndos::default();
        for n in 1..=9 {
            undos.pressed(n, f64::from(n) * 1_000.0);
        }
        assert_eq!(undos.answered(1, "done", 20_000.0), None, "the oldest went");
        assert_eq!(
            undos.answered(2, "done", 20_000.0),
            Some(eq_undo(2_000.0, 2, "done", 18_000.0))
        );
        assert_eq!(
            undos.answered(9, "nothing", 20_000.0),
            Some(eq_undo(9_000.0, 9, "nothing", 11_000.0))
        );
    }

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
            fingers.down(1_000.0, 3, TOUCH, (10.0, 20.0), 3.0),
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
        assert_eq!(
            fingers.end(1_041.0, "cancel", 3, None, 3.0),
            Vec::<Value>::new()
        );
        // A tap: no moves, its down and its up.
        fingers.down(2_000.0, 5, TOUCH, (1.0, 2.0), 1.0);
        assert_eq!(
            fingers.end(2_030.0, "up", 5, Some((1.0, 3.0)), 1.0),
            vec![eq_touch(2_030.0, "up", 5, (1.0, 3.0), 1.0)]
        );
    }

    #[test]
    fn a_long_drag_is_summarized_every_quarter_second() {
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 1, TOUCH, (0.0, 0.0), 2.5);
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
        fingers.down(0.0, 2, TOUCH, (5.0, 5.0), 1.0);
        fingers.down(10.0, 1, TOUCH, (50.0, 50.0), 1.0);
        fingers.moved(20.0, 1, (52.0, 51.0));
        assert_eq!(
            fingers.leave(30.0, "cancel", 1.5),
            vec![
                json!({"ev": "eq_mv", "t": 20.0, "p": 1, "n": 1,
                       "a": [52.0, 51.0], "b": [52.0, 51.0], "ms": 0.0}),
                eq_touch(30.0, "cancel", 1, (52.0, 51.0), 1.5),
                eq_touch(30.0, "cancel", 2, (5.0, 5.0), 1.5),
            ]
        );
        assert_eq!(
            fingers.leave(40.0, "cancel", 1.0),
            Vec::<Value>::new(),
            "none left"
        );
        assert_eq!(fingers.moved(50.0, 1, (1.0, 1.0)), None);
    }

    #[test]
    fn a_finger_whose_end_never_came_is_lost_at_its_last_point() {
        // #124: the next primary down of its type ends it (`end` with
        // `lost`), or a hidden page ends every finger (`leave` with
        // `lost`): its moves first, then `lost` where it last was.
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 3, TOUCH, (5.0, 5.0), 3.0);
        fingers.moved(10.0, 3, (6.0, 5.0));
        assert_eq!(
            fingers.end(900.0, "lost", 3, None, 3.0),
            vec![
                json!({"ev": "eq_mv", "t": 10.0, "p": 3, "n": 1,
                       "a": [6.0, 5.0], "b": [6.0, 5.0], "ms": 0.0}),
                json!({"ev": "eq_touch", "t": 900.0, "what": "lost", "p": 3,
                       "x": 6.0, "y": 5.0, "z": 3.0}),
            ]
        );
        assert_eq!(
            fingers.end(950.0, "up", 3, Some((6.0, 5.0)), 3.0),
            Vec::<Value>::new(),
            "its own end, late, is nothing"
        );
        fingers.down(1_000.0, 4, TOUCH, (7.0, 8.0), 2.5);
        assert_eq!(
            fingers.leave(1_100.0, "lost", 2.5),
            vec![eq_touch(1_100.0, "lost", 4, (7.0, 8.0), 2.5)]
        );
    }

    #[test]
    fn a_primary_down_ends_every_finger_of_its_type_the_recorder_keeps() {
        // #124: the recorder keeps every finger on the area (a third one
        // too, which the screen's fingers never take); a primary down ends
        // each one of its type as `lost`, its moves first, by the fingers'
        // own rule (`behave::Down::ends`). A second finger or another type
        // ends nothing.
        let mouse = Some(Pointer::Mouse);
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 1, TOUCH, (1.0, 1.0), 3.0);
        fingers.down(10.0, 2, TOUCH, (2.0, 2.0), 3.0);
        fingers.down(20.0, 3, TOUCH, (3.0, 3.0), 3.0);
        fingers.down(30.0, 4, mouse, (4.0, 4.0), 3.0);
        fingers.moved(40.0, 3, (3.5, 3.0));
        assert_eq!(
            fingers.missed(50.0, Down::touch(9, false), 3.0),
            Vec::<Value>::new(),
            "a second finger"
        );
        let pen = Down {
            id: 8,
            kind: Some(Pointer::Pen),
            primary: true,
        };
        assert_eq!(fingers.missed(50.0, pen, 3.0), Vec::<Value>::new(), "a pen");
        assert_eq!(
            fingers.missed(60.0, Down::touch(9, true), 2.5),
            vec![
                eq_touch(60.0, "lost", 1, (1.0, 1.0), 2.5),
                eq_touch(60.0, "lost", 2, (2.0, 2.0), 2.5),
                json!({"ev": "eq_mv", "t": 40.0, "p": 3, "n": 1,
                       "a": [3.5, 3.0], "b": [3.5, 3.0], "ms": 0.0}),
                eq_touch(60.0, "lost", 3, (3.5, 3.0), 2.5),
            ]
        );
        // Each once; the mouse stays until a primary down of its own kind.
        assert_eq!(
            fingers.missed(70.0, Down::touch(10, true), 2.5),
            Vec::<Value>::new()
        );
        let click = Down {
            id: 1,
            kind: mouse,
            primary: true,
        };
        assert_eq!(
            fingers.missed(80.0, click, 2.5),
            vec![eq_touch(80.0, "lost", 4, (4.0, 4.0), 2.5)]
        );
        assert_eq!(
            fingers.leave(90.0, "cancel", 1.0),
            Vec::<Value>::new(),
            "none left"
        );
    }

    #[test]
    fn a_finger_whose_contact_a_dropout_cut_stays_until_its_own_end() {
        // #71 PR J: a dropout under a finger on the editor cancels its
        // contact (`cut`), but the finger stays down: its own end, or the
        // next primary down of its type (#124), still gets its row.
        let mut fingers = EqFingers::default();
        fingers.down(0.0, 5, TOUCH, (1.0, 1.0), 3.0);
        fingers.moved(10.0, 5, (2.0, 1.0));
        assert_eq!(
            fingers.cut(20.0, 5, 3.0),
            vec![
                json!({"ev": "eq_mv", "t": 10.0, "p": 5, "n": 1,
                       "a": [2.0, 1.0], "b": [2.0, 1.0], "ms": 0.0}),
                eq_touch(20.0, "cancel", 5, (2.0, 1.0), 3.0),
            ]
        );
        assert_eq!(fingers.moved(30.0, 5, (3.0, 1.0)), None);
        assert_eq!(
            fingers.end(40.0, "up", 5, Some((3.0, 1.0)), 3.0),
            vec![
                json!({"ev": "eq_mv", "t": 30.0, "p": 5, "n": 1,
                       "a": [3.0, 1.0], "b": [3.0, 1.0], "ms": 0.0}),
                eq_touch(40.0, "up", 5, (3.0, 1.0), 3.0),
            ],
            "its moves after the cut and its own lift"
        );
        // Cut, then lost at the next primary down of its type.
        fingers.down(50.0, 6, TOUCH, (4.0, 4.0), 3.0);
        assert_eq!(
            fingers.cut(60.0, 6, 3.0),
            vec![eq_touch(60.0, "cancel", 6, (4.0, 4.0), 3.0)]
        );
        assert_eq!(
            fingers.missed(70.0, Down::touch(7, true), 3.0),
            vec![eq_touch(70.0, "lost", 6, (4.0, 4.0), 3.0)]
        );
        assert_eq!(
            fingers.cut(80.0, 6, 3.0),
            Vec::<Value>::new(),
            "a pointer not down"
        );
    }
}
