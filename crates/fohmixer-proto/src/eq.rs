//! The Pro-Q 4 screen's protocol (#71 PR E, spec F28, D17): the pieces of
//! the `eq_*` messages ([`crate::client`]) and the binary frame contract.
//!
//! A page lists a strip's Pro-Q 4 instances (`eq_list` with the strip's
//! binding, answered by `eq_list` with [`EqItem`]s), opens one (`eq_open`
//! with the instance and the device's LOM path; the hub answers `eq` with
//! [`EqState`]: `opening`, then `open` with the picture's session and size,
//! or `closed` with why), touches it (`eq_input`: [`Touch`] at a point in the
//! picture's pixels) and leaves it (`eq_close`). Every client hears which
//! editors are held (`eq_locks`, [`EqLock`]), at once on every change.
//!
//! **Binary frames:** while a client holds an open editor, the hub sends its
//! picture as binary WebSocket messages, the newest frame winning (a slow
//! link gets fewer frames, never old ones): [`FRAME_HEADER`] bytes of the
//! session number (big-endian `u32`, the `session` of the `eq` message that
//! opened it), then one JPEG. A page shows a frame only when its session is
//! the one it opened: a frame of an editor it already left is dropped.

use serde::{Deserialize, Serialize};

/// The plug-in this screen shows: a `PluginDevice` whose
/// `class_display_name` is this (it holds when the device is renamed).
pub const PRODUCT: &str = "Pro-Q 4";
/// Where a Pro-Q sits when it is in the track's own device chain.
pub const ON_TRACK: &str = "na tracku";
/// Between a rack's and its chain's names in a place.
pub const PLACE_SEP: &str = " › ";
/// The bytes before a frame's JPEG: its session number.
pub const FRAME_HEADER: usize = 4;

/// Why an editor did not open or closed (`eq` `reason`).
pub mod reason {
    /// Another client holds it (`since` says from when).
    pub const LOCKED: &str = "locked";
    /// The hub did not list this device (an `eq_list` names what it opens).
    pub const UNKNOWN: &str = "unknown";
    /// The hub's Pro-Q screen is off (`[eq] backend = "off"`).
    pub const OFF: &str = "off";
    /// The page left the screen (`eq_close`).
    pub const EXIT: &str = "exit";
    /// The page's socket closed.
    pub const DETACH: &str = "detach";
    /// The page opened another editor.
    pub const SWITCH: &str = "switch";
    /// The editor's window went away (closed in Live or on the PC).
    pub const GONE: &str = "window closed";
    /// It is still closing: open it again once it closed.
    pub const CLOSING: &str = "closing";
    /// The listed path names another device now (a track or device moved,
    /// another set): list again.
    pub const MOVED: &str = "moved";
    /// Its editor is already open in Live (opened on the PC, a failed
    /// guard, a hub restart): the hub never closes it without its window.
    pub const OPEN_ON_PC: &str = "open on the PC";
}

/// A finger's phase on the picture (`eq_input`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Touch {
    Down,
    Move,
    Up,
    Cancel,
}

impl Touch {
    /// Its name on the wire and in the records.
    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Move => "move",
            Self::Up => "up",
            Self::Cancel => "cancel",
        }
    }
}

/// One Pro-Q 4 of a strip's track (`eq_list`): its LOM path (index form,
/// what `eq_open` names), where it sits ([`ON_TRACK`] or its racks and
/// chains, [`place`]), the device's name, and whether the hub keeps a last
/// picture of it (`GET /api/eq/picture`; none before a first open).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EqItem {
    pub path: String,
    pub place: String,
    pub name: String,
    #[serde(default)]
    pub picture: bool,
}

/// A held editor (`eq_locks`): held by this client (`mine`) or another,
/// since `since` (the hub's UTC ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqLock {
    pub instance: String,
    pub path: String,
    pub mine: bool,
    pub since: f64,
}

/// This client's editor (`eq`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqState {
    /// Taken: the hub opens its window.
    Opening,
    /// Its window is open: frames of `session` follow.
    Open,
    /// Closed, refused or failed (`reason`).
    Closed,
}

/// The place of a Pro-Q inside racks: each rack's and chain's names, from
/// the track down (`Vocal FX › Main`); [`ON_TRACK`] for none.
pub fn place(steps: &[(String, String)]) -> String {
    if steps.is_empty() {
        return ON_TRACK.to_string();
    }
    steps
        .iter()
        .map(|(rack, chain)| format!("{rack}{PLACE_SEP}{chain}"))
        .collect::<Vec<_>>()
        .join(PLACE_SEP)
}

/// A binary frame: `session` (big-endian) and the JPEG.
pub fn frame(session: u32, jpeg: &[u8]) -> Vec<u8> {
    let mut bytes = session.to_be_bytes().to_vec();
    bytes.extend_from_slice(jpeg);
    bytes
}

/// A binary frame's session and JPEG; none for a message shorter than its
/// header.
pub fn frame_parts(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let head: [u8; FRAME_HEADER] = bytes.get(..FRAME_HEADER)?.try_into().ok()?;
    Some((u32::from_be_bytes(head), &bytes[FRAME_HEADER..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_frame_carries_its_session_then_the_jpeg() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let bytes = frame(0x0102_0304, &jpeg);
        assert_eq!(bytes, vec![1, 2, 3, 4, 0xFF, 0xD8, 0xFF, 0xD9]);
        assert_eq!(frame_parts(&bytes), Some((0x0102_0304, &jpeg[..])));
        assert_eq!(FRAME_HEADER, 4);
        // A header alone is an empty picture; less is no frame.
        assert_eq!(frame_parts(&[0, 0, 0, 7]), Some((7, &[][..])));
        assert_eq!(frame_parts(&[0, 0, 7]), None);
        assert_eq!(frame_parts(&[]), None);
    }

    #[test]
    fn a_place_names_the_racks_and_chains_from_the_track_down() {
        assert_eq!(place(&[]), "na tracku");
        assert_eq!(
            place(&[("Vocal FX".into(), "Main".into())]),
            "Vocal FX › Main"
        );
        assert_eq!(
            place(&[
                ("Vocal FX".into(), "Main".into()),
                ("Inner".into(), "Air".into())
            ]),
            "Vocal FX › Main › Inner › Air"
        );
    }

    #[test]
    fn the_pieces_have_their_wire_shapes() {
        assert_eq!(serde_json::to_value(Touch::Move).unwrap(), json!("move"));
        for touch in [Touch::Down, Touch::Move, Touch::Up, Touch::Cancel] {
            assert_eq!(serde_json::to_value(touch).unwrap(), json!(touch.name()));
        }
        assert_eq!(
            serde_json::to_value(EqState::Opening).unwrap(),
            json!("opening")
        );
        assert_eq!(serde_json::to_value(EqState::Open).unwrap(), json!("open"));
        assert_eq!(
            serde_json::to_value(EqState::Closed).unwrap(),
            json!("closed")
        );
        let item = EqItem {
            path: "live_set tracks 1 devices 0".into(),
            place: ON_TRACK.into(),
            name: "Pro-Q 4".into(),
            picture: false,
        };
        let wire = json!({"path": "live_set tracks 1 devices 0", "place": "na tracku",
                          "name": "Pro-Q 4", "picture": false});
        assert_eq!(serde_json::to_value(&item).unwrap(), wire);
        assert_eq!(serde_json::from_value::<EqItem>(wire).unwrap(), item);
        // An older hub's item without `picture`: none kept.
        let bare = json!({"path": "p", "place": "na tracku", "name": "n"});
        assert!(!serde_json::from_value::<EqItem>(bare).unwrap().picture);
        let lock = EqLock {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            mine: true,
            since: 1_790_000_000_000.5,
        };
        let wire = json!({"instance": "band", "path": "live_set tracks 1 devices 0",
                          "mine": true, "since": 1_790_000_000_000.5});
        assert_eq!(serde_json::to_value(&lock).unwrap(), wire);
        assert_eq!(serde_json::from_value::<EqLock>(wire).unwrap(), lock);
    }
}
