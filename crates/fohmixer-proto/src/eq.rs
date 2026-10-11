//! The Pro-Q 4 screen's protocol (#71 PR E, spec F28, D17): the pieces of
//! the `eq_*` messages ([`crate::client`]) and the binary frame contract.
//!
//! A page lists a strip's Pro-Q 4 instances (`eq_list` with the strip's
//! binding, answered by `eq_list` with [`EqItem`]s), opens one (`eq_open`
//! with the instance and the device's LOM path; the hub answers `eq` with
//! [`EqState`]: `opening`, then `open` with the picture's session and size,
//! or `closed` with why), touches it (`eq_input`: [`Touch`] at a point in the
//! picture's pixels) and leaves it (`eq_close`). Every client hears which
//! editors are held (`eq_locks`, [`EqLock`]), at once on every change. One
//! editor is on the PC's screen at a time: while a client holds one,
//! another client's open is refused (`locked` for that one, `in use` for any
//! other).
//!
//! **The device's shape (PR G):** `eq_open` carries the page's picture
//! area ([`Area`], CSS px), and the page sends `eq_area` when it changes
//! while it holds an open editor (a phone turned, a window resized): the hub
//! gives Live's editor window that aspect (Pro-Q 4 lays itself out for it)
//! and puts the editor's own size back before its close. The frames carry
//! the new picture; the page sizes the picture from them.
//!
//! **Binary frames:** while a client holds an open editor, the hub sends its
//! picture as binary WebSocket messages, the newest frame winning (a slow
//! link gets fewer frames, never old ones): a [`FrameHead`] of
//! [`FRAME_HEADER`] bytes (the session number, the `session` of the `eq`
//! message that opened it, the frame's number in that session, from 1,
//! `shown` (PR J), and the part's corner `x`, `y` and the whole picture's
//! `width`, `height` (PR L); each a big-endian `u32`, 28 bytes), then one
//! JPEG. A page shows a
//! frame only when its session is the one it opened: a frame of an editor
//! it already left is dropped.
//!
//! **The part the page shows (PR L):** at 2× and more the page tells the
//! hub the part of the picture it shows, with a margin, and the picture's
//! size it drew ([`Part`], `eq_view`), and the hub sends only that part of
//! a picture of that size: a frame's header names its corner in the
//! picture and the whole picture's size, and the page draws it there
//! (outside it, the page keeps what it drew; out of sight). Under 2×, with
//! no part, or for a picture of another size, frames carry the whole
//! picture (corner 0, 0).
//!
//! **Which input a picture shows (PR J):** a page numbers its inputs of an
//! open editor (`eq_input`'s `n`, from 1, every phase), and each frame says
//! the last of them the hub handled at least [`PAINT_MS`] before it grabbed
//! the picture (`shown`: Pro-Q had that long to paint it). One number
//! answers two questions on one clock, the page's: how long a finger's move
//! took to reach the iPad's screen (from the input's send to the draw of the
//! first picture that shows it), and whether a picture is old (it does not
//! show every input the page sent, [`caught_up`]). A finger landing on an
//! old picture never acts: the page refuses it, and the hub drops a down
//! whose `seen` (the `shown` of the picture it landed on) is not caught up
//! with the page's earlier inputs.
//!
//! **One frame in flight (PR I):** the page acks each frame of its open
//! editor as it arrives (`eq_frame_ack` with the session and the frame's
//! number: it acks every frame up to it), and the hub sends that page's
//! next frame only after the ack, or, with one frame in flight, once its
//! wait for the ack ran out (a lost ack never stalls the picture; never a
//! third frame in flight). On a slow link the frames then never pile up in
//! the socket's send buffer, and a pong, an ack or a value waits behind one
//! frame at most.
//!
//! **Pro-Q's own undo (PR P2a):** the page asks the hub to undo the last
//! change of its open editor (`eq_undo`, numbered with its inputs: a
//! picture's `shown` covers it, and a finger on a picture from before it
//! is refused). The hub reads Pro-Q 4's ↶ in a fresh picture and taps it
//! only when it reads lit (something to undo), never twice within
//! [`UNDO_APART_MS`]; it answers the page alone (`eq_undo`):
//! [`UNDO_DONE`], [`UNDO_NOTHING`] (it read grey) or why not (a
//! [`reason`]).

use serde::{Deserialize, Serialize};

/// The plug-in this screen shows: a `PluginDevice` whose
/// `class_display_name` is this (it holds when the device is renamed).
pub const PRODUCT: &str = "Pro-Q 4";
/// Where a Pro-Q sits when it is in the track's own device chain.
pub const ON_TRACK: &str = "na tracku";
/// Between a rack's and its chain's names in a place.
pub const PLACE_SEP: &str = " › ";
/// The bytes before a frame's JPEG: its [`FrameHead`].
pub const FRAME_HEADER: usize = 28;

/// A part of the picture (PR L): its corner and size, in the editor's
/// picture pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The part of the picture a page asks for (PR L): `crop` of a picture of
/// `picture` px, the size the page drew. The hub cuts only a grab of that
/// size (the PR L re-review): a picture of another size goes whole until
/// the page's part for it lands, and one back at that size is cut again,
/// so the page and the hub never disagree about which part goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub crop: Crop,
    pub picture: (u32, u32),
}

/// How long Pro-Q gets to paint an input before a picture counts as
/// showing it (ms, PR J): a picture grabbed less than this after the hub
/// handled an input may not show it yet. Measured in Carla's bridge (#71 PR
/// M/N, 0.1.0-dev.58: 40 injected moves, the first grab that showed each
/// started after p50 17.6 ms, p95 19.0, max 21.3, a grab taking 16.5): the
/// p95 rounded up to 5 ms.
pub const PAINT_MS: f64 = 20.0;

/// The least time (ms, PR P2a) from one undo tap the hub makes on an
/// editor to its next: two taps closer than Windows' double-click time (500
/// ms by default) would be a double click on Pro-Q's ↶, and the page keeps
/// its control disabled that long. The hub refuses a sooner undo
/// ([`reason::UNDO_SOON`]).
pub const UNDO_APART_MS: f64 = 700.0;
/// An undo's result (`eq_undo`, PR P2a): Pro-Q's ↶ read lit and was tapped.
pub const UNDO_DONE: &str = "done";
/// An undo's result: Pro-Q's ↶ read grey, nothing to undo (no tap).
pub const UNDO_NOTHING: &str = "nothing";

/// A binary frame's header (PR I): the session it shows, its number in
/// that session (from 1), which the page's ack names, the last of the
/// page's inputs it shows (PR J, 0: none yet), and (PR L) where its JPEG
/// lies in the picture (`x`, `y`) and the whole picture's size (`width`,
/// `height`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHead {
    pub session: u32,
    pub seq: u32,
    pub shown: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Whether a picture that shows the page's input `shown` shows every input
/// up to `sent` (PR J): one that does not is old, and a finger on it never
/// acts.
pub fn caught_up(shown: u32, sent: u32) -> bool {
    shown >= sent
}

/// Why an editor did not open or closed (`eq` `reason`).
pub mod reason {
    /// Another client holds it (`since` says from when).
    pub const LOCKED: &str = "locked";
    /// Another client holds another editor (`since` says from when): one
    /// Pro-Q 4 on the PC's screen at a time (one screen, one cursor; two
    /// editors on top would overlap).
    pub const IN_USE: &str = "in use";
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
    /// The device's state could not be read before the open: try again.
    pub const UNREAD: &str = "unread";
    /// Live is not connected to the hub (its Live client's own words).
    pub const OFFLINE: &str = "instance offline";
    /// Live gave no answer in time (its thread busy): try again.
    pub const NO_ANSWER: &str = "Live did not answer";
    /// The hub knows no Live instance of that name.
    pub const UNKNOWN_INSTANCE: &str = "unknown instance";
    /// Live opened no editor window within the hub's wait.
    pub const NO_WINDOW: &str = "no window";
    /// Several new editor windows came at once (an open on the PC at the
    /// same moment): try again.
    pub const SEVERAL: &str = "several windows";
    /// The new window has no Pro-Q 4 picture (its `FF_UIWindow` child): it
    /// is not a Pro-Q 4 editor.
    pub const NO_PICTURE: &str = "the window has no Pro-Q picture (FF_UIWindow)";
    /// The taken window never came on top of the others within the hub's
    /// wait (its z-order change is posted to Live's busy thread): try again.
    pub const NOT_ON_TOP: &str = "the window did not come on top";
    /// The hub's window worker stopped: the hub is stopping.
    pub const STOPPED: &str = "the window worker stopped";
    /// A strip's devices nest deeper than the hub's walk reads (racks in
    /// racks): no list.
    pub const TOO_DEEP: &str = "the devices nest too deep";
    /// The close left the editor open in Live: its guard could not tap (a
    /// value field may be open), or the hub could not be sure which device
    /// to turn off. Close it on the PC.
    pub const LEFT_OPEN: &str = "left open in Live";
    /// Windows' press-and-hold could not be turned off for the editor's
    /// window (#71 PR O): a still finger could become a right click (Pro-Q's
    /// menu, with Delete), so the hub does not open it.
    pub const HOLD_ON: &str = "press-and-hold could not be turned off";
    /// The hub has no editor of the page's (an undo, PR P2a, of an editor
    /// it does not hold, or one whose window just went).
    pub const NO_EDITOR: &str = "no such editor";
    /// An undo refused (PR P2a): a finger is on the picture, or the editor
    /// is opening, resizing or closing. Try again once it rests.
    pub const UNDO_BUSY: &str = "the editor is busy";
    /// An undo refused (PR P2a): sooner than [`super::UNDO_APART_MS`] after
    /// the hub's last undo tap on that editor (two taps would be a double
    /// click).
    pub const UNDO_SOON: &str = "too soon after the last undo";
    /// An undo refused (PR P2a): Pro-Q 4's ↶ is not where it was measured
    /// (a size the table does not know, a Pro-Q update): nothing tapped.
    pub const UNDO_MISSING: &str = "Pro-Q's undo is not where it was measured";
}

/// The page's picture area (`eq_open`'s `area`; PR G): the size, in CSS
/// px, of the part of the page's screen that shows the editor's picture.
/// The hub gives the editor its aspect; an open without one keeps the
/// editor's own size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub w: f64,
    pub h: f64,
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

/// A binary frame: its header (big-endian) and the JPEG.
pub fn frame(head: FrameHead, jpeg: &[u8]) -> Vec<u8> {
    [
        &head.session.to_be_bytes()[..],
        &head.seq.to_be_bytes()[..],
        &head.shown.to_be_bytes()[..],
        &head.x.to_be_bytes()[..],
        &head.y.to_be_bytes()[..],
        &head.width.to_be_bytes()[..],
        &head.height.to_be_bytes()[..],
        jpeg,
    ]
    .concat()
}

/// A binary frame's header and JPEG; none for a message shorter than its
/// header.
pub fn frame_parts(bytes: &[u8]) -> Option<(FrameHead, &[u8])> {
    let (head, jpeg) = bytes.split_at_checked(FRAME_HEADER)?;
    let ([session, seq, shown, x, y, width, height], _) = head.as_chunks::<4>() else {
        return None;
    };
    let head = FrameHead {
        session: u32::from_be_bytes(*session),
        seq: u32::from_be_bytes(*seq),
        shown: u32::from_be_bytes(*shown),
        x: u32::from_be_bytes(*x),
        y: u32::from_be_bytes(*y),
        width: u32::from_be_bytes(*width),
        height: u32::from_be_bytes(*height),
    };
    Some((head, jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_frame_carries_its_head_then_the_jpeg() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let head = FrameHead {
            session: 0x0102_0304,
            seq: 0x0506_0708,
            shown: 0x090A_0B0C,
            x: 0x0D0E_0F10,
            y: 0x1112_1314,
            width: 0x1516_1718,
            height: 0x191A_1B1C,
        };
        let bytes = frame(head, &jpeg);
        let mut expected: Vec<u8> = (1..=28).collect();
        expected.extend_from_slice(&jpeg);
        assert_eq!(bytes, expected);
        assert_eq!(frame_parts(&bytes), Some((head, &jpeg[..])));
        assert_eq!(FRAME_HEADER, 28);
        // A header alone is an empty picture; less is no frame.
        assert_eq!(frame_parts(&bytes[..28]), Some((head, &[][..])));
        assert_eq!(frame_parts(&bytes[..27]), None);
        assert_eq!(frame_parts(&bytes[..12]), None);
        assert_eq!(frame_parts(&[]), None);
    }

    #[test]
    fn a_picture_is_caught_up_when_it_shows_every_input_sent() {
        assert_eq!(PAINT_MS, 20.0);
        assert!(caught_up(0, 0), "no input yet");
        assert!(caught_up(5, 5));
        assert!(caught_up(6, 5));
        assert!(!caught_up(4, 5));
        assert!(!caught_up(0, 1));
    }

    #[test]
    fn the_reasons_have_their_wire_words() {
        use super::reason::*;
        assert_eq!(
            [LOCKED, UNKNOWN, OFF, EXIT, DETACH, SWITCH, GONE, CLOSING],
            [
                "locked",
                "unknown",
                "off",
                "exit",
                "detach",
                "switch",
                "window closed",
                "closing"
            ]
        );
        assert_eq!(
            [
                MOVED,
                OPEN_ON_PC,
                UNREAD,
                OFFLINE,
                NO_ANSWER,
                UNKNOWN_INSTANCE
            ],
            [
                "moved",
                "open on the PC",
                "unread",
                "instance offline",
                "Live did not answer",
                "unknown instance"
            ]
        );
        assert_eq!(
            [NO_WINDOW, SEVERAL, NO_PICTURE, STOPPED, LEFT_OPEN],
            [
                "no window",
                "several windows",
                "the window has no Pro-Q picture (FF_UIWindow)",
                "the window worker stopped",
                "left open in Live"
            ]
        );
        assert_eq!(NOT_ON_TOP, "the window did not come on top");
        assert_eq!(TOO_DEEP, "the devices nest too deep");
        assert_eq!(HOLD_ON, "press-and-hold could not be turned off");
        assert_eq!(IN_USE, "in use");
        // PR P2a: an undo's refusals.
        assert_eq!(
            [NO_EDITOR, UNDO_BUSY, UNDO_SOON, UNDO_MISSING],
            [
                "no such editor",
                "the editor is busy",
                "too soon after the last undo",
                "Pro-Q's undo is not where it was measured"
            ]
        );
    }

    #[test]
    fn an_undo_is_done_or_nothing_and_two_taps_are_700_ms_apart() {
        // PR P2a: Windows' double-click time is 500 ms by default.
        assert_eq!(UNDO_APART_MS, 700.0);
        assert_eq!([UNDO_DONE, UNDO_NOTHING], ["done", "nothing"]);
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
        let area = Area { w: 390.5, h: 765.0 };
        let wire = json!({"w": 390.5, "h": 765.0});
        assert_eq!(serde_json::to_value(area).unwrap(), wire);
        assert_eq!(serde_json::from_value::<Area>(wire).unwrap(), area);
    }
}
