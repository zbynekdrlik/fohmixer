//! The system's gestures (#43 PR G, design comment 6011117148): what the
//! browser or the system does with a touch that the controls' own records
//! cannot see. The owner's retest of PR F saw, now and then, WebKit's
//! magnifier (loupe) or the whole control lifted as if dragged; the page's
//! flight recorder had nothing of it. Now:
//!
//! - every control's root prevents its `touchstart` ([`CONTROL_PREVENTS`],
//!   an active listener: only that stops the loupe of a tap followed by a
//!   hold, WebKit bug 231161); Pointer Events stay the input path;
//! - the surface prevents [`SURFACE_PREVENTS`] on every element of it;
//! - the page records each of [`RECORDED`] that reaches the window, after the
//!   surface's listeners ran, as a [`sys`] event: the control's kind and keys
//!   (or the element's kind, [`kind`]), the pointer, and whether something
//!   prevented it; a `lostpointercapture` only while its pointer is still
//!   down ([`records`]: every captured touch ends with one after its lift,
//!   which is no news; which pointers are down is `diag::perf::Perf`'s); and
//!   the visual viewport's scale as a [`zoom`] event when it changes
//!   ([`ZoomWatch`]).
//!
//! Pure, tested natively; `diag.rs` and `dom.rs` hold the listeners and
//! `components` the controls' directive.

use serde_json::{Value, json};

/// What a control's root prevents: the loupe of a tap followed by a hold
/// and the system's long-press gestures start from a touch the page did not
/// take.
pub const CONTROL_PREVENTS: [&str; 1] = ["touchstart"];

/// What the surface prevents on every element of it: no context menu, no
/// selection, no drag.
pub const SURFACE_PREVENTS: [&str; 3] = ["contextmenu", "selectstart", "dragstart"];

/// What the page records when it reaches the window: the surface's
/// prevented three, a pinch's start (WebKit), a cancelled pointer and a
/// capture lost.
pub const RECORDED: [&str; 6] = [
    "contextmenu",
    "selectstart",
    "dragstart",
    "gesturestart",
    "pointercancel",
    "lostpointercapture",
];

/// The attribute a control's root carries its write keys in (a JSON array),
/// so a record names the control.
pub const KEYS_ATTR: &str = "data-keys";

/// A system gesture at page time `t`: `what` (the event's type) on `on`
/// (the control's or element's kind), the control's `keys` (left out when
/// none), the `pointer` of a pointer event, and whether something
/// prevented it.
pub fn sys(
    t: f64,
    what: &str,
    on: &str,
    keys: &[String],
    pointer: Option<i32>,
    prevented: bool,
) -> Value {
    let mut event = json!({"ev": "sys", "t": t, "what": what, "on": on, "prevented": prevented});
    if !keys.is_empty() {
        event["keys"] = json!(keys);
    }
    if let Some(pointer) = pointer {
        event["pointer"] = json!(pointer);
    }
    event
}

/// The visual viewport's scale `scale` at page time `t`.
pub fn zoom(t: f64, scale: f64) -> Value {
    json!({"ev": "zoom", "t": t, "scale": scale})
}

/// What a record calls the element an event came from: the control's kind
/// (the `data-testid` of the control's root it lies in), else the nearest
/// `data-testid`, else its `tag` name in lower case; `none` without an
/// element (an event at the document: a capture lost by a control taken
/// away under the finger).
pub fn kind(control: Option<&str>, nearest: Option<&str>, tag: Option<&str>) -> String {
    match control.or(nearest) {
        Some(testid) => testid.to_string(),
        None => tag.map_or_else(|| "none".to_string(), str::to_ascii_lowercase),
    }
}

/// Whether a gesture `what` is recorded, `held` saying whether its pointer
/// is still down: every one, but a `lostpointercapture` only while its
/// pointer is (a capture lost after the lift ends every captured touch; one
/// lost while the finger is on the glass is the system taking the touch, or
/// the control going away under it).
pub fn records(what: &str, held: bool) -> bool {
    what != "lostpointercapture" || held
}

/// A control's keys as its [`KEYS_ATTR`] holds them.
pub fn keys_text(keys: &[String]) -> String {
    json!(keys).to_string()
}

/// The keys a [`KEYS_ATTR`] holds; none when it holds no list of texts.
pub fn keys_of(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// A scale to two decimals: a zoom record's value, so a pinch does not
/// record every frame's few thousandths.
pub fn rounded(scale: f64) -> f64 {
    (scale * 100.0).round() / 100.0
}

/// The visual viewport's scale last recorded (1 at load).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomWatch {
    scale: f64,
}

impl Default for ZoomWatch {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

impl ZoomWatch {
    /// The visual viewport's scale is `scale`: the scale to record (two
    /// decimals) when that differs from the last recorded.
    pub fn zoom(&mut self, scale: f64) -> Option<f64> {
        let now = rounded(scale);
        if now == self.scale {
            return None;
        }
        self.scale = now;
        Some(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        vec!["band|live_set tracks[name=Vox 1] mixer_device volume|value".to_string()]
    }

    #[test]
    fn a_gesture_names_its_control_its_keys_its_pointer_and_whether_it_was_prevented() {
        assert_eq!(
            sys(12.5, "contextmenu", "fader", &keys(), Some(3), true),
            json!({
                "ev": "sys",
                "t": 12.5,
                "what": "contextmenu",
                "on": "fader",
                "keys": keys(),
                "pointer": 3,
                "prevented": true
            })
        );
    }

    #[test]
    fn a_gesture_off_the_controls_has_no_keys_and_a_plain_event_no_pointer() {
        assert_eq!(
            sys(1.0, "gesturestart", "row", &[], None, false),
            json!({"ev": "sys", "t": 1.0, "what": "gesturestart", "on": "row", "prevented": false})
        );
    }

    #[test]
    fn a_zoom_says_its_scale() {
        assert_eq!(
            zoom(7.0, 1.5),
            json!({"ev": "zoom", "t": 7.0, "scale": 1.5})
        );
    }

    #[test]
    fn the_kind_is_the_controls_else_the_nearest_test_id_else_the_tag_else_none() {
        assert_eq!(
            kind(Some("mute"), Some("strip-label"), Some("SPAN")),
            "mute"
        );
        assert_eq!(kind(None, Some("row"), Some("DIV")), "row");
        assert_eq!(kind(None, None, Some("BODY")), "body");
        assert_eq!(kind(None, None, None), "none", "no element");
        assert_eq!(kind(Some("fader"), None, None), "fader");
    }

    #[test]
    fn a_controls_keys_go_through_its_attribute_and_back() {
        let two = vec![
            keys()[0].clone(),
            "band|live_set tracks[name=Vox 1]|mute".to_string(),
        ];
        assert_eq!(keys_of(&keys_text(&two)), two);
        assert_eq!(keys_text(&[]), "[]");
        assert_eq!(keys_of("[]"), Vec::<String>::new());
        assert_eq!(keys_of("not json"), Vec::<String>::new());
        assert_eq!(
            keys_of(r#"["a", 4]"#),
            Vec::<String>::new(),
            "not a list of texts"
        );
        assert_eq!(keys_text(&two), format!(r#"["{}","{}"]"#, two[0], two[1]));
    }

    #[test]
    fn the_control_and_the_surface_prevent_their_events_and_the_page_records_the_rest() {
        assert_eq!(CONTROL_PREVENTS, ["touchstart"]);
        assert_eq!(
            SURFACE_PREVENTS,
            ["contextmenu", "selectstart", "dragstart"]
        );
        for prevented in SURFACE_PREVENTS {
            assert!(RECORDED.contains(&prevented), "{prevented} is recorded");
        }
        assert_eq!(
            RECORDED[3..],
            ["gesturestart", "pointercancel", "lostpointercapture"]
        );
        assert_eq!(KEYS_ATTR, "data-keys");
    }

    #[test]
    fn a_gesture_and_a_zoom_are_never_dropped_for_the_backlog() {
        assert!(super::super::is_essential("sys"));
        assert!(super::super::is_essential("zoom"));
    }

    #[test]
    fn a_lost_capture_is_recorded_only_while_its_pointer_is_down_every_other_gesture_always() {
        assert!(records("lostpointercapture", true), "still down");
        assert!(
            !records("lostpointercapture", false),
            "the lift's own lost capture"
        );
        for what in [
            "contextmenu",
            "selectstart",
            "dragstart",
            "gesturestart",
            "pointercancel",
        ] {
            assert!(records(what, false), "{what}");
            assert!(records(what, true), "{what} of a pointer down");
        }
    }

    #[test]
    fn a_zoom_is_recorded_when_its_two_decimals_change_away_from_1_and_back() {
        let mut g = ZoomWatch::default();
        assert_eq!(g.zoom(1.0), None, "1 at load");
        assert_eq!(g.zoom(1.004), None, "rounds to 1");
        assert_eq!(g.zoom(1.006), Some(1.01));
        assert_eq!(g.zoom(1.012), None, "still 1.01");
        assert_eq!(g.zoom(2.0), Some(2.0));
        assert_eq!(g.zoom(1.0), Some(1.0), "back to 1");
        assert_eq!(g.zoom(1.0), None);
    }

    #[test]
    fn a_scale_is_rounded_to_two_decimals() {
        assert_eq!(rounded(1.234), 1.23);
        assert_eq!(rounded(1.235_1), 1.24);
        assert_eq!(rounded(0.5), 0.5);
        assert_eq!(rounded(3.0), 3.0);
    }
}
