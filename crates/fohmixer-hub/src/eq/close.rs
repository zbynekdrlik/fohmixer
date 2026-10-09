//! Which device a close turns off in Live (#71 PR E), pure: never one other
//! than the editor the hub opened. The close turns it off through the
//! `$ref` its open read ([`ref_off`] reads the answer): the same object
//! wherever it moved. A ref the script answers [`STALE_REF`] names a device
//! that was deleted (or whose pointer holds another class now: the device
//! went too), and Live closed its editor with it: nothing of the hub's is
//! open, so Live is left alone ([`Shut::NoneOpen`]). Only when the ref is
//! unknown to the script ([`UNKNOWN_REF`]: another connection's registry)
//! or no ref is held does the path check below decide; a ref turn-off that
//! got no answer or failed otherwise leaves Live alone (the ref may still be
//! good, and the set may still land). The hub holds an editor by its path in
//! Live's index form (`live_set tracks 1 devices 0`); a track inserted,
//! deleted or moved above it, or a device before it, makes that path name
//! another device. So before `is_editor_open = false` the close re-reads, in
//! one batch, the device at the held path: its `class_display_name` and its
//! `is_editor_open`.
//!
//! - **Still a Pro-Q 4 with its editor open:** the held one; it closes there.
//! - **Anything else** (another device, none, a closed editor, a failed
//!   read): the editor moved. The set's tracks, its return tracks and its
//!   master track are walked for their Pro-Q 4s (`eq::walk`, each walk
//!   bounded as a list's), and their `is_editor_open` read in one batch:
//!   - **exactly one open:** the moved editor (a client holds one editor,
//!     and this one's window is the hub's): it closes at its new path;
//!   - **none:** nothing is open to close ([`Check::NoneOpen`]);
//!   - **several,** or a read or walk that failed: Live is left alone (the
//!     window is handed back, the editor may stay open: the router records
//!     a `problem`).
//!
//! What a close left in Live is a [`Shut`]: the editor turned off, nothing
//! of the hub's open there any more, or the editor maybe still open (only
//! then does its holder hear "left open in Live"). The router
//! (`router/eq.rs`) carries the [`Check`]s out: the reads through the
//! script, the walks, then the close or nothing.

use fohmixer_proto::eq::PRODUCT;
use serde_json::Value;

use super::walk::{DISPLAY_NAME, answered, get, items};

/// The Live property that opens and closes a plug-in's editor.
pub const EDITOR_OPEN: &str = "is_editor_open";
/// The set, and its lists of tracks a moved editor is looked for on.
pub const SET: &str = "live_set";
pub const TRACKS: &str = "tracks";
pub const RETURNS: &str = "return_tracks";
/// The master track, walked after the others.
pub const MASTER: &str = "live_set master_track";
/// The most steps a check takes: the held device's read, the tracks'
/// read, the walks, the open states' read.
pub const STEPS: usize = 4;

/// Why a close leaves Live alone: nothing is open to close.
pub const NONE_OPEN: &str = "the editor moved and no open Pro-Q 4 was found: Live is left alone";
/// Why a close leaves Live alone: the editor may still be open.
pub const SEVERAL_OPEN: &str =
    "the editor moved and several open Pro-Q 4s were found: Live is left alone";
pub const TRACKS_UNREAD: &str =
    "the editor moved and the set's tracks could not be read: Live is left alone";
pub const OPEN_UNREAD: &str =
    "the editor moved and an editor's open state could not be read: Live is left alone";
pub const OUT_OF_TURN: &str = "the close check was answered out of turn: Live is left alone";
pub const UNFINISHED: &str = "the close check did not finish: Live is left alone";

/// Why Live is left alone when a read through the script failed.
pub fn unread(why: &str) -> String {
    format!("Live could not be read, it is left alone: {why}")
}

/// Why Live is left alone when a track's walk failed.
pub fn walk_failed(why: &str) -> String {
    format!("the editor moved and a track could not be walked ({why}): Live is left alone")
}

/// Why Live is left alone when the turn-off through the editor's `$ref`
/// failed while the ref may still be good (no answer, or the set failed).
pub fn ref_failed(why: &str) -> String {
    format!("the editor could not be turned off through its ref, Live is left alone: {why}")
}

/// Why Live is left alone when the editor's device was deleted (the
/// script's `StaleRef`): Live closed its editor with it.
pub fn deleted(why: &str) -> String {
    format!("the editor's device was deleted (Live closed its editor), nothing to close: {why}")
}

/// The script's `errorType` of a `$ref` its registry never issued or
/// cleared: each connection starts a new registry.
pub const UNKNOWN_REF: &str = "UnknownRef";
/// The script's `errorType` of a `$ref` whose object was deleted, or whose
/// pointer holds another class now (the object went too).
pub const STALE_REF: &str = "StaleRef";

/// What the turn-off through an editor's `$ref` answered.
#[derive(Debug, Clone, PartialEq)]
pub enum RefOff {
    /// It went through.
    Done,
    /// The script does not know the ref (another connection's registry):
    /// the held path is checked.
    Unknown(String),
    /// The ref's device was deleted, and Live closed its editor with it:
    /// nothing to close, Live is left alone (the held path names another
    /// device now, if any).
    Deleted(String),
    /// No answer (Live offline, a timeout, a refused request) or another
    /// failure: the ref may still be good and the set may still land, so
    /// Live is left alone.
    Failed(String),
}

/// What the turn-off through a `$ref` answered: its slots, or why the call
/// got none.
pub fn ref_off(answer: Result<&[Value], String>) -> RefOff {
    let slots = match answer {
        Ok(slots) => slots,
        Err(why) => return RefOff::Failed(why),
    };
    let slot = slots.first();
    if slot.and_then(|s| s.get("ok")) == Some(&Value::Bool(true)) {
        return RefOff::Done;
    }
    let text = Value::from(slots.to_vec()).to_string();
    match slot
        .and_then(|s| s.get("errorType"))
        .and_then(Value::as_str)
    {
        Some(UNKNOWN_REF) => RefOff::Unknown(text),
        Some(STALE_REF) => RefOff::Deleted(text),
        _ => RefOff::Failed(text),
    }
}

/// What a close left in Live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shut {
    /// `is_editor_open = false` went through.
    Off,
    /// Nothing of the hub's is open in Live any more (its device was
    /// deleted, or no open Pro-Q 4 was found after a move): why.
    NoneOpen(String),
    /// The editor may still be open in Live (the guard could not tap, the
    /// turn-off failed, or the hub could not be sure which device to turn
    /// off): why.
    LeftOpen(String),
}

impl Shut {
    /// Whether the editor may still be open in Live: its holder hears so.
    pub fn left_open(&self) -> bool {
        matches!(self, Self::LeftOpen(_))
    }

    /// The `eq` record of a close that did not turn its editor off, its
    /// `what` and why: `problem` (left open in Live) or `none_open`
    /// (nothing open there any more).
    pub fn note(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Off => None,
            Self::NoneOpen(why) => Some(("none_open", why.as_str())),
            Self::LeftOpen(why) => Some(("problem", why.as_str())),
        }
    }
}

/// What a close does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    /// These commands through the script, their slots to
    /// [`CloseCheck::answer`].
    Read(Vec<Value>),
    /// A walk of each of these tracks (`eq::walk`), the Pro-Q 4s' paths to
    /// [`CloseCheck::walked`].
    Walk(Vec<String>),
    /// `is_editor_open = false` at `path`; `moved`: not the held path.
    Close { path: String, moved: bool },
    /// The editor moved and no open Pro-Q 4 was found: nothing to close.
    NoneOpen,
    /// Live is left alone, the editor maybe open: why.
    Leave(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    Held,
    Tracks,
    Walking,
    Open(Vec<String>),
    Done,
}

/// One close's check of the device it turns off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseCheck {
    held: String,
    stage: Stage,
}

/// Whether the held device's re-read (`class_display_name`, then
/// `is_editor_open`) says it is still the Pro-Q 4 with its editor open.
pub fn still_held(slots: &[Value]) -> bool {
    let product = slots.first().and_then(answered).and_then(Value::as_str);
    let open = slots.get(1).and_then(answered).and_then(Value::as_bool);
    product == Some(PRODUCT) && open == Some(true)
}

/// The one open Pro-Q 4 among `paths` (their `is_editor_open` in `slots`,
/// in order).
fn open_one(paths: &[String], slots: &[Value]) -> Check {
    let mut open = Vec::new();
    for (path, slot) in paths
        .iter()
        .zip(slots.iter().map(Some).chain(std::iter::repeat(None)))
    {
        match slot.and_then(answered).and_then(Value::as_bool) {
            Some(true) => open.push(path),
            Some(false) => {}
            None => return Check::Leave(OPEN_UNREAD.to_string()),
        }
    }
    match open.as_slice() {
        [] => Check::NoneOpen,
        [path] => Check::Close {
            path: (*path).clone(),
            moved: true,
        },
        _ => Check::Leave(SEVERAL_OPEN.to_string()),
    }
}

/// What a check left after its last step: a close or a leave; one still
/// reading or walking leaves Live alone.
pub fn settled(step: Check) -> Check {
    match step {
        Check::Read(_) | Check::Walk(_) => Check::Leave(UNFINISHED.to_string()),
        done => done,
    }
}

impl CloseCheck {
    /// The check of the editor held at `held`, and its first read.
    pub fn start(held: &str) -> (Self, Check) {
        let check = Self {
            held: held.to_string(),
            stage: Stage::Held,
        };
        let first = Check::Read(vec![get(held, DISPLAY_NAME), get(held, EDITOR_OPEN)]);
        (check, first)
    }

    /// The slots of the last read: the next step.
    pub fn answer(&mut self, slots: &[Value]) -> Check {
        match std::mem::replace(&mut self.stage, Stage::Done) {
            Stage::Held if still_held(slots) => Check::Close {
                path: self.held.clone(),
                moved: false,
            },
            Stage::Held => {
                self.stage = Stage::Tracks;
                Check::Read(vec![get(SET, TRACKS), get(SET, RETURNS)])
            }
            Stage::Tracks => self.tracks(slots),
            Stage::Open(paths) => open_one(&paths, slots),
            Stage::Walking | Stage::Done => Check::Leave(OUT_OF_TURN.to_string()),
        }
    }

    /// The set's tracks and return tracks read: every one of them and the
    /// master track to walk.
    fn tracks(&mut self, slots: &[Value]) -> Check {
        let (Some(tracks), Some(returns)) = (
            slots.first().and_then(answered),
            slots.get(1).and_then(answered),
        ) else {
            return Check::Leave(TRACKS_UNREAD.to_string());
        };
        let mut targets: Vec<String> = items(tracks)
            .into_iter()
            .chain(items(returns))
            .map(|(_, path, _)| path.to_string())
            .collect();
        targets.push(MASTER.to_string());
        self.stage = Stage::Walking;
        Check::Walk(targets)
    }

    /// The walks' outcomes, one per track asked (its Pro-Q 4s' paths, or
    /// why it could not be walked): their open states to read.
    pub fn walked(&mut self, outcomes: Vec<Result<Vec<String>, String>>) -> Check {
        if !matches!(
            std::mem::replace(&mut self.stage, Stage::Done),
            Stage::Walking
        ) {
            return Check::Leave(OUT_OF_TURN.to_string());
        }
        let mut paths = Vec::new();
        for outcome in outcomes {
            match outcome {
                Ok(found) => paths.extend(found),
                Err(why) => return Check::Leave(walk_failed(&why)),
            }
        }
        if paths.is_empty() {
            return Check::NoneOpen;
        }
        let commands = paths.iter().map(|path| get(path, EDITOR_OPEN)).collect();
        self.stage = Stage::Open(paths);
        Check::Read(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HELD: &str = "live_set tracks 1 devices 0";
    const MOVED: &str = "live_set tracks 0 devices 0";
    const OTHER: &str = "live_set tracks 0 devices 1 chains 0 devices 0";

    fn ok(data: Value) -> Value {
        json!({"ok": true, "data": data})
    }

    fn failed(why: &str) -> Value {
        json!({"ok": false, "error": why})
    }

    fn track(path: &str) -> Value {
        json!({"$ref": "r", "path": path, "class": "Track", "name": "t"})
    }

    fn read(target: &str, prop: &str) -> Value {
        json!({"target": target, "name": "get_prop", "args": {"prop": prop}})
    }

    /// A check past its held read: walking the set's tracks.
    fn walking() -> CloseCheck {
        let (mut check, _) = CloseCheck::start(HELD);
        let tracks = check.answer(&[ok(json!("EQ Eight")), ok(json!(false))]);
        assert_eq!(
            tracks,
            Check::Read(vec![
                read("live_set", "tracks"),
                read("live_set", "return_tracks")
            ])
        );
        let walk = check.answer(&[
            ok(json!([
                track("live_set tracks 0"),
                track("live_set tracks 1")
            ])),
            ok(json!([track("live_set return_tracks 0")])),
        ]);
        assert_eq!(
            walk,
            Check::Walk(vec![
                "live_set tracks 0".into(),
                "live_set tracks 1".into(),
                "live_set return_tracks 0".into(),
                "live_set master_track".into(),
            ])
        );
        check
    }

    /// A check that walked the set and found Pro-Q 4s at `MOVED` and
    /// `OTHER`: their open states asked.
    fn found_two() -> CloseCheck {
        let mut check = walking();
        let step = check.walked(vec![
            Ok(vec![MOVED.to_string(), OTHER.to_string()]),
            Ok(Vec::new()),
            Ok(Vec::new()),
            Ok(Vec::new()),
        ]);
        assert_eq!(
            step,
            Check::Read(vec![
                read(MOVED, "is_editor_open"),
                read(OTHER, "is_editor_open")
            ])
        );
        check
    }

    #[test]
    fn the_held_device_still_an_open_pro_q_closes_at_its_path() {
        let (mut check, first) = CloseCheck::start(HELD);
        assert_eq!(
            first,
            Check::Read(vec![
                read(HELD, "class_display_name"),
                read(HELD, "is_editor_open")
            ])
        );
        assert_eq!(
            check.answer(&[ok(json!("Pro-Q 4")), ok(json!(true))]),
            Check::Close {
                path: HELD.into(),
                moved: false
            }
        );
        // Answered again: out of turn.
        assert_eq!(
            check.answer(&[ok(json!("Pro-Q 4")), ok(json!(true))]),
            Check::Leave(OUT_OF_TURN.into())
        );
    }

    #[test]
    fn only_an_open_pro_q_is_still_the_held_one() {
        assert!(still_held(&[ok(json!("Pro-Q 4")), ok(json!(true))]));
        // Another product, a closed editor, a failed or a missing read.
        assert!(!still_held(&[ok(json!("Pro-C 2")), ok(json!(true))]));
        assert!(!still_held(&[ok(json!("Pro-Q 4")), ok(json!(false))]));
        assert!(!still_held(&[
            failed("Index out of range"),
            ok(json!(true))
        ]));
        assert!(!still_held(&[ok(json!("Pro-Q 4")), failed("no such prop")]));
        assert!(!still_held(&[ok(json!("Pro-Q 4"))]));
        assert!(!still_held(&[]));
    }

    #[test]
    fn a_moved_editor_found_open_once_closes_at_its_new_path() {
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(true)), ok(json!(false))]),
            Check::Close {
                path: MOVED.into(),
                moved: true
            }
        );
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(false)), ok(json!(true))]),
            Check::Close {
                path: OTHER.into(),
                moved: true
            }
        );
    }

    #[test]
    fn a_moved_editor_found_open_nowhere_has_nothing_to_close() {
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(false)), ok(json!(false))]),
            Check::NoneOpen
        );
        // No Pro-Q 4 in the whole set: nothing to read.
        let mut check = walking();
        assert_eq!(
            check.walked(vec![Ok(Vec::new()), Ok(Vec::new())]),
            Check::NoneOpen
        );
        assert_eq!(settled(Check::NoneOpen), Check::NoneOpen);
    }

    #[test]
    fn a_moved_editor_found_open_several_times_leaves_live_alone() {
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(true)), ok(json!(true))]),
            Check::Leave(SEVERAL_OPEN.into())
        );
    }

    #[test]
    fn a_read_or_a_walk_that_failed_leaves_live_alone() {
        // An open state not read, or missing.
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(true)), failed("gone")]),
            Check::Leave(OPEN_UNREAD.into())
        );
        let mut check = found_two();
        assert_eq!(
            check.answer(&[ok(json!(true))]),
            Check::Leave(OPEN_UNREAD.into())
        );
        // A track not walked, even with an open Pro-Q 4 found elsewhere.
        let mut check = walking();
        assert_eq!(
            check.walked(vec![Ok(vec![MOVED.into()]), Err("instance busy".into())]),
            Check::Leave(walk_failed("instance busy"))
        );
        // The tracks or the return tracks not read.
        for slots in [
            vec![failed("busy"), ok(json!([]))],
            vec![ok(json!([])), failed("busy")],
            vec![ok(json!([]))],
        ] {
            let (mut check, _) = CloseCheck::start(HELD);
            check.answer(&[failed("Index out of range")]);
            assert_eq!(check.answer(&slots), Check::Leave(TRACKS_UNREAD.into()));
        }
    }

    #[test]
    fn a_walk_comes_only_when_asked() {
        // Before the tracks were read, and once more after.
        let (mut check, _) = CloseCheck::start(HELD);
        assert_eq!(
            check.walked(vec![Ok(vec![MOVED.into()])]),
            Check::Leave(OUT_OF_TURN.into())
        );
        let mut check = walking();
        check.walked(vec![Ok(vec![MOVED.into()])]);
        assert_eq!(
            check.walked(vec![Ok(vec![MOVED.into()])]),
            Check::Leave(OUT_OF_TURN.into())
        );
        // A walk is no read's answer.
        let mut check = walking();
        assert_eq!(check.answer(&[]), Check::Leave(OUT_OF_TURN.into()));
    }

    #[test]
    fn a_check_still_reading_after_its_steps_leaves_live_alone() {
        assert_eq!(
            settled(Check::Read(Vec::new())),
            Check::Leave(UNFINISHED.into())
        );
        assert_eq!(
            settled(Check::Walk(Vec::new())),
            Check::Leave(UNFINISHED.into())
        );
        let close = Check::Close {
            path: MOVED.into(),
            moved: true,
        };
        assert_eq!(settled(close.clone()), close);
        assert_eq!(
            settled(Check::Leave("why".into())),
            Check::Leave("why".into())
        );
        assert_eq!(STEPS, 4);
    }

    #[test]
    fn a_ref_turn_off_falls_back_to_the_path_only_when_the_script_does_not_know_the_ref() {
        let answer = |slot: Value| ref_off(Ok(&[slot][..]));
        assert_eq!(answer(json!({"ok": true, "data": null})), RefOff::Done);
        let gone =
            |error: &str, kind: &str| json!({"ok": false, "error": error, "errorType": kind});
        // Unknown: another connection's registry; the held path is checked.
        let unknown = gone("unknown id: live_7", "UnknownRef");
        assert_eq!(
            answer(unknown.clone()),
            RefOff::Unknown(json!([unknown]).to_string())
        );
        // Deleted, or its pointer holds another class now: Live closed the
        // editor with its device.
        for error in [
            "deleted: live_7",
            "class changed: live_7 is Track, not PluginDevice",
        ] {
            let slot = gone(error, "StaleRef");
            let text = json!([slot.clone()]).to_string();
            assert_eq!(answer(slot), RefOff::Deleted(text), "{error}");
        }
        // Any other failure of the set: the ref may still be good.
        let live = gone("live error: busy", "OpError");
        assert_eq!(
            answer(live.clone()),
            RefOff::Failed(json!([live]).to_string())
        );
        let bare = json!({"ok": false});
        assert_eq!(
            answer(bare.clone()),
            RefOff::Failed(json!([bare]).to_string())
        );
        let none: &[Value] = &[];
        assert_eq!(ref_off(Ok(none)), RefOff::Failed("[]".to_string()));
        // No answer at all: Live offline, a timeout, a refused request.
        for why in ["instance offline", "no result within 3 s", "refused: busy"] {
            assert_eq!(
                ref_off(Err(why.to_string())),
                RefOff::Failed(why.to_string())
            );
        }
        assert_eq!((UNKNOWN_REF, STALE_REF), ("UnknownRef", "StaleRef"));
    }

    #[test]
    fn a_close_tells_an_editor_left_open_from_nothing_open() {
        assert!(!Shut::Off.left_open());
        assert!(!Shut::NoneOpen("gone".into()).left_open());
        assert!(Shut::LeftOpen("busy".into()).left_open());
        assert_eq!(Shut::Off.note(), None);
        assert_eq!(
            Shut::NoneOpen("gone".into()).note(),
            Some(("none_open", "gone"))
        );
        assert_eq!(
            Shut::LeftOpen("busy".into()).note(),
            Some(("problem", "busy"))
        );
    }

    #[test]
    fn the_messages_say_why_live_is_left_alone() {
        assert_eq!(
            unread("instance offline"),
            "Live could not be read, it is left alone: instance offline"
        );
        assert_eq!(
            ref_failed("instance offline"),
            "the editor could not be turned off through its ref, Live is left alone: \
             instance offline"
        );
        assert_eq!(
            walk_failed("busy"),
            "the editor moved and a track could not be walked (busy): Live is left alone"
        );
        assert_eq!(
            deleted("deleted: live_7"),
            "the editor's device was deleted (Live closed its editor), nothing to close: \
             deleted: live_7"
        );
    }
}
