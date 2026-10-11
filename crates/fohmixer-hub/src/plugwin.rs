//! A plug-in's editor window on the PC's screen (#71 PR E, D17, F28; what
//! the PC tests established: `.claude/rules/plugin-window.md`).
//!
//! A platform backend behind [`Backend`] does what touches a window: the
//! Windows one (`win.rs`, compiled on Windows only) and the simulated one
//! ([`sim::Sim`], the hub's backend elsewhere, in the E2E harness and in the
//! tests). One worker thread owns the backend ([`Plugwin::spawn`]): nothing
//! runs on Live's thread, and nothing in the router waits for a window.
//!
//! - **Finding an editor:** Live does not make a new editor window the
//!   foreground one, so the router lists the editor windows ([`Plugwin::list`]),
//!   sets `is_editor_open = true`, and the worker takes the window that is new
//!   in the list ([`pick`]), polling every [`FIND_POLL_MS`] for up to
//!   [`FIND_MS`]; several new ones are refused (an open of the PC's own may
//!   have come at once). A new window the backend cannot take yet
//!   ([`Backend::ready`]: Live shows it before Pro-Q attaches its picture)
//!   is awaited the same way, and refused only when the wait is over.
//! - **Taking it:** on top of every window (no move or size; the z-order
//!   change is posted, never waited for), its picture located (Pro-Q's own
//!   child window). The change lands once the window's thread (Live's) takes
//!   it, so the take answers only once the window is on top
//!   ([`Backend::on_top`], a read of its style at every step, no message):
//!   until then another window may cover it, and a grab or a touch would
//!   meet that window. One not on top within [`FIND_MS`] of its take is
//!   handed back, and the open fails ([`NOT_ON_TOP`], [`placed`]).
//! - **The capture:** every [`CAPTURE_MS`] (25 fps) while its holder views
//!   it: a picture equal to the last one is skipped (unless it shows a newer
//!   input of the page, PR J: [`Shown`], the input the router last handed
//!   over as handled at least `PAINT_MS` before the grab), another goes to
//!   the encoder, a thread of its own: it makes the JPEG ([`QUALITY`]) and hands
//!   it to its [`FrameSink`] (the holder's socket, newest wins, and the
//!   card's last picture). One picture waits for the encoder at most, the
//!   newest (a slow encode drops pictures, never queues them), so its
//!   contacts are never held up by an encode. What blocks the worker: the
//!   grab, and the window list and its checks (an open's polls, a take);
//!   a z-order change (a take, a release) is posted. The counts go to the
//!   hub's log once a minute ([`Rate`]). A window that went away is
//!   reported [`PlugwinEvent::Lost`].
//! - **A contact:** one on the screen at a time, numbered by the router. A
//!   down goes only when no contact is down and no resize of its editor is
//!   on its way (else it ends at once, [`BUSY`]; [`waits_out`]); any other
//!   phase only of the contact down ([`accepts`]). A down or an
//!   update lands only when the point's window is the editor's; else
//!   nothing is injected and the contact ends
//!   ([`PlugwinEvent::ContactEnded`], with its number).
//! - **The keep-alive:** Windows ends an injected contact that gets no frame
//!   for 100 ms, so the worker injects a resting contact's last point again
//!   once [`KEEPALIVE_MS`] passed since its last injection. Each step looks
//!   at it first, again after the opens' polls and again after each grab,
//!   every time on a fresh read of its clock: a grab blocks the worker for
//!   its time, and two engineers' editors are grabbed one after the other.
//!   An injection is stamped when it goes. The largest gap between two
//!   injections of a contact goes into the minute's counts ([`Rate`]).
//! - **The device's shape (PR G)** ([`Plugwin::resize`]): a page's picture
//!   area gives the picture a size ([`crate::eq::size::editor_size`] in the
//!   room of the window's work area, [`Backend::work`], less its frame,
//!   [`room`]; each side at least [`Backend::min_size`], none when the room
//!   is smaller); the window's rectangle for it is posted ([`Backend::resize`]:
//!   the frame beyond the picture, measured at the take, added, the window
//!   kept where it stands when it fits there, else moved into the work
//!   area, [`inside`]; #74 review) and the picture's size is read every step
//!   ([`Backend::client`], no message) until it lands within
//!   [`RESIZE_SLACK`] or [`RESIZE_MS`] passed ([`settled`]): the caller hears
//!   how it settled ([`Resized`]). One that settled unlanded gets the
//!   window's rectangle before it posted back at once (#74 review: Live's
//!   frame may have taken it and Pro-Q not); both may still wait in Live's
//!   thread, and land in order before anything posted after them (the
//!   guard's placement, below). The capture goes on meanwhile, and the
//!   frames carry the picture's size. A grab of another size with
//!   no resize on its way (one that landed late, a window resized on the
//!   PC) is reported ([`PlugwinEvent::Sized`]). A resize asked while a
//!   contact of the editor's session is down waits for its end, the newest
//!   only (#74 review: Pro-Q never lays itself out under a finger), and
//!   then [`QUIET_MS`] more (round 3: Live's thread takes a posted
//!   rectangle before injected input, so it would land before the
//!   injected end).
//! - **The close guard** ([`Plugwin::guard`]): no resize reaches the editor
//!   from then on; the frames stop, its own contact ends at its last point
//!   (an up), another session's is cancelled there and reported
//!   ([`GUARD_CANCEL`]: the PC injects one contact, so the tap would fail or
//!   end the other engineer's drag). The inert spot is known only at
//!   [`KNOWN_SIZE`] (#74 review, I1), and Live's thread lands a posted
//!   rectangle before injected input, so the guard places the editor there
//!   in two steps, each seen to land (#74 review, round 2; [`guard_step`],
//!   [`Placement`]), once a resize on its way settled and [`QUIET_MS`]
//!   passed since its contact ended (round 3): the window's
//!   rectangle for that size (at the take's place, moved into the work area
//!   when it does not fit there, [`known_rect`]) a pixel aside first
//!   ([`aside`]: a rectangle the window cannot have before it lands, so
//!   reading it proves that every rectangle posted before it landed), then
//!   that rectangle itself, read back with the picture at exactly that
//!   size. Only then the tap on the inert spot; a step not seen to land
//!   within [`RESIZE_MS`] of its post fails the guard, which taps nothing
//!   (the router then leaves the editor open in Live). A value text field
//!   closes on a click elsewhere, and Pro-Q 4.02 crashes Live when its
//!   editor closes with one open. A guard that cannot tap fails too.
//! - **The release** ([`Plugwin::release`]): the window's z-order as it
//!   was; unless Live closed the editor, its rectangle at the take (place
//!   and size) posted again when it was changed (best effort, not waited
//!   for); when Live closed it, [`Backend::live_closed`]. The stop ends the
//!   contact and releases every window (the editors stay open in Live); the
//!   hub's stop waits for that, bounded ([`Plugwin::stopped`]).

pub mod probe;
pub mod sim;
#[cfg(windows)]
pub mod win;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use fohmixer_proto::eq::{Area, Crop, FrameHead, PAINT_MS, Part, reason};
use tokio::sync::oneshot;

use crate::eq::size::editor_size;

/// The capture's period (ms): 25 frames a second at most.
pub const CAPTURE_MS: f64 = 40.0;
/// How often the editor windows are listed while one is awaited (ms).
pub const FIND_POLL_MS: f64 = 50.0;
/// How long a new editor window is awaited after `is_editor_open = true` (ms).
pub const FIND_MS: f64 = 3000.0;
/// How often a capture's counts are logged (ms).
pub const RATE_MS: f64 = 60_000.0;
/// A contact down this long (ms) since its last injection is injected
/// again at its last point (Windows cancels one silent for 100 ms).
pub const KEEPALIVE_MS: f64 = 50.0;
/// How long (ms) after a contact of an editor's session ended nothing is
/// posted to that editor's window: no resize, no guard placement (#74
/// review, round 3). Live's thread takes posted messages before injected
/// input, so a move posted right after an injected up could lay Pro-Q out
/// anew before the up lands, at the old screen point.
pub const QUIET_MS: f64 = 100.0;
/// How long the hub's stop waits for the worker to hand its windows back
/// and end ([`Plugwin::stopped`]): [`STOP_POLLS`] looks [`STOP_POLL`] apart.
pub const STOP_WAIT: Duration = Duration::from_secs(1);
pub const STOP_POLL: Duration = Duration::from_millis(10);
pub const STOP_POLLS: u32 = 100;
/// The worker's longest sleep between two looks at its clocks.
pub const STEP: Duration = Duration::from_millis(10);
/// How long the guard's finger stays on the inert spot.
pub const GUARD_TAP: Duration = Duration::from_millis(30);
/// The frames' JPEG quality (#71 PR I: 70 before). A real Pro-Q 4
/// screenshot re-encoded at 60 is about 15 % smaller than at 70 (42.7 →
/// 36.4 KB, 44.6 → 38.1 KB, 4:2:0 as this encoder) with its scale labels,
/// curve and grid lines as readable when magnified twice (the page zooms
/// up to 4×); at 55 the faint grid lines start to fade. The picture keeps its size: a finger maps to
/// its pixels.
pub const QUALITY: u8 = 60;
/// The picture size (px) at which the inert spot was verified on the PC
/// (Pro-Q 4 at 100 %, #71): the guard taps only at exactly this size (#74
/// review: at 760 px wide the same share of the width lies by Undo).
pub const KNOWN_SIZE: (u32, u32) = (1349, 809);
/// The inert spot: this share of the picture's width, in the top bar's empty
/// middle (Pro-Q 4 at 100 %: x 546 of 1349, right of the logo's panel, left
/// of the undo arrows; confirmed on the PC, #71), at [`KNOWN_SIZE`] only.
pub const INERT_X: f64 = 0.405;
/// The inert spot's height in the picture (px): the top bar's middle.
pub const INERT_Y: i32 = 15;
/// How long a resize, or the guard's restore, may take to land (ms): the
/// window's size is posted to its thread (Live's), which lands it when it
/// can.
pub const RESIZE_MS: f64 = 1000.0;
/// How far a picture's side may lie from the size asked (px) and still
/// count as landed.
pub const RESIZE_SLACK: u32 = 2;
/// The smallest picture the hub asks for (px): Pro-Q 4's own minimum stands
/// in its way below it (the PC check reads it, `eq-probe --min-probe`).
pub const MIN_PICTURE: (u32, u32) = (600, 400);

/// Why an open found no window (a reason its page reads).
pub const NO_WINDOW: &str = reason::NO_WINDOW;
/// Why an open found several (a reason its page reads).
pub const SEVERAL: &str = reason::SEVERAL;
/// Why an open failed when its window never came on top (a reason its
/// page reads).
pub const NOT_ON_TOP: &str = reason::NOT_ON_TOP;
/// Why a command found no editor of its session.
pub const NO_EDITOR: &str = "no such editor";
/// Why a command found no worker (a reason a page reads).
pub const STOPPED: &str = reason::STOPPED;
/// Why the close guard ended another session's contact.
pub const GUARD_CANCEL: &str = "the close guard of another editor";
/// Why a down never went: another contact was still down, or a resize of
/// its editor was on its way.
pub const BUSY: &str = "busy";
/// Why a guard did not tap: its editor's placement at [`KNOWN_SIZE`] was
/// not seen to land in time (the inert spot is known only at that size).
pub const NOT_BACK: &str = "the editor is not at the size its inert spot is known at";

/// A window's handle as a number (a Win32 `HWND` is a pointer, not `Send`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// One phase of a contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Down,
    Update,
    Up,
    Cancel,
}

impl Phase {
    /// Its name in the records.
    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Update => "update",
            Self::Up => "up",
            Self::Cancel => "cancel",
        }
    }

    /// Whether it lands only on the editor's own window (a down or an
    /// update; an up or a cancel always goes: a contact must end).
    pub fn checked(self) -> bool {
        matches!(self, Self::Down | Self::Update)
    }

    /// Whether it ends the contact.
    pub fn ends(self) -> bool {
        matches!(self, Self::Up | Self::Cancel)
    }
}

/// A rectangle on the screen (physical px): its top-left corner and its
/// size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

/// An editor window the backend took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The top-level window.
    pub window: WindowId,
    /// The window whose client area is the picture: Pro-Q's own child,
    /// else the window itself.
    pub picture: WindowId,
    /// Its process.
    pub pid: u32,
    /// Whether it was on top of every window before (its z-order put back).
    pub was_topmost: bool,
    /// The picture's size when it was taken: the editor's own size, which
    /// the guard and the release put back (PR G).
    pub width: u32,
    pub height: u32,
    /// The window's rectangle when it was taken: what it has beyond the
    /// picture (its frame) is added to a picture size asked for.
    pub rect: Rect,
}

/// What the taken window has beyond its picture (px): its frame and title,
/// as measured at the take.
fn frame(taken: &Taken) -> (i32, i32) {
    let side = |window: i32, picture: u32| window - picture as i32;
    (
        side(taken.rect.width, taken.width),
        side(taken.rect.height, taken.height),
    )
}

/// The window's size that gives its picture `client` (PR G): the frame
/// beyond the picture added.
pub fn window_size(taken: &Taken, client: (u32, u32)) -> (i32, i32) {
    let (width, height) = frame(taken);
    (client.0 as i32 + width, client.1 as i32 + height)
}

/// The room for the taken window's picture (px, PR G): its work area
/// (`work`: its screen less the taskbar) less the frame beyond the picture
/// (a resize moves the window into the work area when it does not fit
/// where it stands, [`inside`]); none for a frame larger than the area.
pub fn room(work: Rect, taken: &Taken) -> (u32, u32) {
    let (width, height) = frame(taken);
    let side = |room: i32, frame: i32| u32::try_from(room - frame).unwrap_or(0);
    (side(work.width, width), side(work.height, height))
}

/// The window's rectangle of `size` (px, PR G; #74 review): where `from`
/// stands when it fits in the work area there, else moved in on each side
/// it would cross (a window larger than the area at its near edge).
pub fn inside(work: Rect, from: Rect, size: (i32, i32)) -> Rect {
    let side = |at: i32, near: i32, room: i32, long: i32| at.min(near + room - long).max(near);
    Rect {
        left: side(from.left, work.left, work.width, size.0),
        top: side(from.top, work.top, work.height, size.1),
        width: size.0,
        height: size.1,
    }
}

/// Whether a picture of `client` is the size `asked` (PR G), within
/// [`RESIZE_SLACK`] each side.
pub fn lands(client: (u32, u32), asked: (u32, u32)) -> bool {
    client.0.abs_diff(asked.0) <= RESIZE_SLACK && client.1.abs_diff(asked.1) <= RESIZE_SLACK
}

/// Whether a resize, posted `waited_ms` ago, waited its [`RESIZE_MS`].
pub fn resize_over(waited_ms: f64) -> bool {
    waited_ms >= RESIZE_MS
}

/// How a resize to `asked` from a picture of `from`, posted `waited_ms`
/// ago, its picture now `client`, settles: landed (`true`), over without
/// landing (`false`), or not yet. Over its [`RESIZE_MS`], a picture that
/// left `from` landed at the size the editor took (Live keeps a plug-in's
/// window within its own bounds: on the PC an upright phone's 760 × 1361
/// came as 760 × 1275, 0.1.0-dev.50); one still at `from` never landed.
pub fn settled(
    client: (u32, u32),
    asked: (u32, u32),
    from: (u32, u32),
    waited_ms: f64,
) -> Option<bool> {
    if lands(client, asked) {
        Some(true)
    } else {
        resize_over(waited_ms).then(|| !lands(client, from))
    }
}

/// A placement of the close guard (#74 review, round 2): the editor's
/// window put at the rectangle for [`KNOWN_SIZE`] ([`known_rect`]) in two
/// steps, each posted once the one before it was seen to land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The first: the known rectangle a pixel aside ([`aside`]), a
    /// rectangle the window does not have before it lands. A window's
    /// thread takes its posted messages in order, so reading it proves that
    /// every rectangle posted before it (a resize, its way back) landed.
    Aside(Rect),
    /// The second: the known rectangle itself, landed once the window reads
    /// it with its picture at exactly [`KNOWN_SIZE`].
    Known(Rect),
}

impl Placement {
    /// The rectangle it posts.
    pub fn rect(self) -> Rect {
        match self {
            Self::Aside(rect) | Self::Known(rect) => rect,
        }
    }

    /// Whether it landed: the window's rectangle (`window`) and its
    /// picture's size (`client`) as read now (none: unread).
    pub fn landed(self, window: Option<Rect>, client: Option<(u32, u32)>) -> bool {
        match self {
            Self::Aside(rect) => window == Some(rect),
            Self::Known(rect) => window == Some(rect) && client == Some(KNOWN_SIZE),
        }
    }
}

/// The known rectangle `known` a pixel aside (#74 review, round 2), inward
/// (round 3): one to the right when it still lies inside the work area
/// `work`'s right edge there, else one to the left (`known` stands against
/// that edge, moved in by [`known_rect`]); and the other way when the
/// window reads that one already (`now`): a rectangle it does not have
/// before the post lands. That other way may lie one pixel past the work
/// area's edge (`known` against the left edge and the window a pixel to
/// its right, or against the right edge and the window a pixel to its
/// left; #74's final review, its second note): harmless, it is only the
/// proof that what was posted before it landed, the known rectangle
/// follows it before the tap, and no input goes in between.
pub fn aside(known: Rect, now: Rect, work: Rect) -> Rect {
    let by = |dx: i32| Rect {
        left: known.left + dx,
        top: known.top,
        width: known.width,
        height: known.height,
    };
    let fits = known.left + 1 + known.width <= work.left + work.width;
    let (inward, other) = if fits {
        (by(1), by(-1))
    } else {
        (by(-1), by(1))
    };
    if now == inward { other } else { inward }
}

/// A step of the close guard (PR G; #74 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStep {
    /// Look again at the next step.
    Wait,
    /// Post the known rectangle a pixel aside ([`Placement::Aside`]).
    Aside,
    /// Post the known rectangle ([`Placement::Known`]).
    Known,
    /// The window reads the known rectangle with its picture at
    /// [`KNOWN_SIZE`], and nothing posted before it waits: tap the inert
    /// spot.
    Tap,
    /// This placement was not seen to land in time: no tap (the editor is
    /// left open).
    Fail(Placement),
}

/// What the guard does at a step (#74 review, round 2): wait while
/// `settling`: a resize of its editor still settles (one unlanded posts
/// its way back as it settles, which must go before the guard's
/// placement), or a contact of its session ended less than [`QUIET_MS`]
/// ago (round 3: the guard's own up included; Live's thread would land
/// the placement before that injected up); then post its
/// first placement (`posted`: none yet; else its placement posted last and
/// the ms since); once that is seen to land ([`Placement::landed`] with the
/// window's rectangle and picture's size read now) the second; once that
/// is, the tap. A placement not seen to land within [`RESIZE_MS`] of its
/// post fails the guard.
pub fn guard_step(
    settling: bool,
    posted: Option<(Placement, f64)>,
    window: Option<Rect>,
    client: Option<(u32, u32)>,
) -> GuardStep {
    match posted {
        _ if settling => GuardStep::Wait,
        None => GuardStep::Aside,
        Some((placement, _)) if placement.landed(window, client) => match placement {
            Placement::Aside(_) => GuardStep::Known,
            Placement::Known(_) => GuardStep::Tap,
        },
        Some((placement, waited)) if resize_over(waited) => GuardStep::Fail(placement),
        Some(_) => GuardStep::Wait,
    }
}

/// Why a guard whose `placement` was not seen to land in time taps nothing
/// ([`NOT_BACK`]): what the window read last (`window`, `client`; none:
/// unread) and what was posted.
pub fn not_landed(
    placement: Placement,
    window: Option<Rect>,
    client: Option<(u32, u32)>,
) -> String {
    let shown = |rect: Rect| {
        format!(
            "{}x{} at {},{}",
            rect.width, rect.height, rect.left, rect.top
        )
    };
    let read = window.map_or_else(|| "unread".to_string(), shown);
    match placement {
        Placement::Aside(posted) => format!(
            "{NOT_BACK}: its first placement did not land (the window is {read}, {} posted)",
            shown(posted)
        ),
        Placement::Known(posted) => {
            let (width, height) = client.unwrap_or((0, 0));
            format!(
                "{NOT_BACK}: its placement did not land (the window is {read} with a {width}x{height} picture, {} posted for {}x{})",
                shown(posted),
                KNOWN_SIZE.0,
                KNOWN_SIZE.1
            )
        }
    }
}

/// A taken editor's picture size when it is not [`KNOWN_SIZE`] (#74
/// review: Live or Pro-Q kept its last size, or it was given one on the
/// PC; the guard puts it at the known size before its tap): the hub's log
/// says so at the take.
pub fn other_size(size: (u32, u32)) -> Option<(u32, u32)> {
    (size != KNOWN_SIZE).then_some(size)
}

/// The window's rectangle for [`KNOWN_SIZE`] (#74 review): at the take's
/// place, moved into the work area `work` when it does not fit there.
pub fn known_rect(work: Rect, taken: &Taken) -> Rect {
    inside(work, taken.rect, window_size(taken, KNOWN_SIZE))
}

/// How a resize of an editor's picture settled (PR G): the size asked, the
/// picture's size then, whether it landed ([`lands`]), how long it took
/// (ms), and whether, unlanded, the window's rectangle before it was posted
/// back (#74 review).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resized {
    pub asked: (u32, u32),
    pub client: (u32, u32),
    pub ok: bool,
    pub ms: f64,
    pub reverted: bool,
}

/// One picture: top-down BGRA rows, 4 bytes a pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// What touches a window.
pub trait Backend: Send {
    /// Readies the calling thread (the worker's, or the probe's): the
    /// Windows backend makes it per-monitor aware and sets up touch
    /// injection.
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// The plug-in editor windows on the screen now (Live's).
    fn editors(&mut self) -> Result<Vec<WindowId>, String>;
    /// The visible top-level windows of process `pid` (the probe's host).
    fn windows_of(&mut self, pid: u32) -> Result<Vec<WindowId>, String>;
    /// Live was asked to open an editor (the simulated backend opens its
    /// window; a real one has nothing to do).
    fn live_opened(&mut self) {}
    /// Whether `window` can be taken now: the hub's Windows backend wants
    /// Pro-Q's own child window, which may come after the window itself.
    fn ready(&mut self, window: WindowId) -> bool;
    /// Takes `window`: on top of every window (no move, no size), its
    /// picture located. The z-order change may land later
    /// ([`Backend::on_top`]).
    fn take(&mut self, window: WindowId) -> Result<Taken, String>;
    /// Whether the taken window is on top of every window now: the take's
    /// change is posted to the window's thread, which lands it when it can.
    fn on_top(&mut self, taken: &Taken) -> bool;
    /// Whether the window is still there.
    fn alive(&mut self, taken: &Taken) -> bool;
    /// The work area of the taken window's monitor (px, PR G): its screen
    /// less the taskbar.
    fn work(&mut self, taken: &Taken) -> Result<Rect, String>;
    /// The taken window's rectangle now (px, PR G; #74 review): a read that
    /// sends the window no message.
    fn rect(&mut self, taken: &Taken) -> Result<Rect, String>;
    /// The smallest picture the hub asks for (px, PR G).
    fn min_size(&self) -> (u32, u32) {
        MIN_PICTURE
    }
    /// Asks the taken window for the rectangle `rect` (PR G; #74 review: its
    /// place and its size, the frame included, [`window_size`], [`inside`]),
    /// posted to its thread, no z-order change, no activation. It lands
    /// when that thread takes it ([`Backend::client`]).
    fn resize(&mut self, taken: &Taken, rect: Rect) -> Result<(), String>;
    /// The picture's size now (PR G): a read that sends the window no
    /// message.
    fn client(&mut self, taken: &Taken) -> Result<(u32, u32), String>;
    /// The picture now.
    fn grab(&mut self, taken: &Taken) -> Result<Pixels, String>;
    /// A contact phase at picture point `at`: a down or an update lands only
    /// when the point's window is the editor's (else an error, nothing
    /// injected); an up or a cancel always goes. The system cursor is put
    /// back when the contact ends.
    fn touch(&mut self, taken: &Taken, phase: Phase, at: (i32, i32)) -> Result<(), String>;
    /// The window's z-order as it was.
    fn release(&mut self, taken: &Taken);
    /// Live closed the editor (`is_editor_open = false` went through) after
    /// its window was handed back: the simulated backend closes the window
    /// then; a real one has nothing to do (Live closes it).
    fn live_closed(&mut self, _taken: &Taken) {}
    /// Asks the window to close (the probe's `--close` only).
    fn close_window(&mut self, taken: &Taken);
    /// Turns Windows' press-and-hold (a still contact's up made a right
    /// click) off (`quiet`) or back on for the taken window and its
    /// picture (#71 PR N, the probe's `--hold`: the
    /// `MicrosoftTabletPenServiceProperty` window property).
    fn press_and_hold(&mut self, _taken: &Taken, _quiet: bool) -> Result<(), String> {
        Ok(())
    }
}

/// A frame the encoder made: its session, the last of the page's inputs
/// its picture shows (PR J, [`Shown`]) and its JPEG.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub session: u32,
    pub shown: u32,
    /// The part of the picture its JPEG holds (PR L; none: the whole
    /// picture), and the whole picture's size.
    pub crop: Option<Crop>,
    pub size: (u32, u32),
    pub jpeg: Bytes,
}

impl Frame {
    /// Its header for the page (PR L: the part's corner, the whole
    /// picture's size); the flight numbers it as it goes (`seq` 0 here).
    pub fn head(&self) -> FrameHead {
        let (x, y) = self.crop.map_or((0, 0), |crop| (crop.x, crop.y));
        FrameHead {
            session: self.session,
            seq: 0,
            shown: self.shown,
            x,
            y,
            width: self.size.0,
            height: self.size.1,
        }
    }

    /// Whether it holds the whole picture (a card's last picture is only
    /// that).
    pub fn whole(&self) -> bool {
        self.crop.is_none()
    }
}

/// The part `crop` of a picture of `size` a frame carries (PR L): clamped
/// to the picture; none when that holds the whole picture or nothing.
pub fn crop_within(crop: Crop, size: (u32, u32)) -> Option<Crop> {
    let x = crop.x.min(size.0);
    let y = crop.y.min(size.1);
    let within = Crop {
        x,
        y,
        w: crop.w.min(size.0 - x),
        h: crop.h.min(size.1 - y),
    };
    let whole = (x, y, within.w, within.h) == (0, 0, size.0, size.1);
    (within.w.min(within.h) > 0 && !whole).then_some(within)
}

/// The pixels of `crop` (one [`crop_within`] gave) of `pixels`.
pub fn cropped(pixels: &Pixels, crop: Crop) -> Pixels {
    let stride = pixels.width as usize * 4;
    let from = crop.x as usize * 4;
    let to = from + crop.w as usize * 4;
    let bgra = pixels
        .bgra
        .chunks_exact(stride)
        .skip(crop.y as usize)
        .take(crop.h as usize)
        .flat_map(|row| &row[from..to])
        .copied()
        .collect();
    Pixels {
        width: crop.w,
        height: crop.h,
        bgra,
    }
}

/// Where a frame goes.
pub type FrameSink = Arc<dyn Fn(Frame) + Send + Sync>;

/// The handled inputs a [`Shown`] keeps waiting for their paint at most: a
/// page sends one a frame, so far fewer wait; past it the newest waiting
/// gives way to the next (its picture counts as showing it only later).
pub const SHOWN_KEPT: usize = 64;

/// How long an equal picture waits before it goes again anyway (ms, the PR
/// J re-review): a page whose last frame did not decode would otherwise
/// keep a still picture that does not show its last input, refusing every
/// finger.
pub const REFRESH_MS: f64 = 1000.0;

/// Whether an input handled at `at` is painted in a picture grabbed at `t`
/// (worker ms): Pro-Q had [`PAINT_MS`] for it.
pub fn painted(at: f64, t: f64) -> bool {
    t - at >= PAINT_MS
}

/// The page's inputs an editor's pictures show (PR J): each input the
/// router handed over as handled, with when; a picture grabbed at `t` shows
/// the last one handled at least [`PAINT_MS`] before (0: none yet).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shown {
    waiting: VecDeque<(u32, f64)>,
    shown: u32,
}

impl Shown {
    /// The page's input `n` handled at `at` (worker ms). With
    /// [`SHOWN_KEPT`] waiting, the newest waiting one gives way to it: a
    /// picture then counts as showing it only once `n` is painted (the
    /// later time; the PR J review: counting the oldest as painted was not
    /// conservative).
    pub fn handled(&mut self, n: u32, at: f64) {
        if self.waiting.len() >= SHOWN_KEPT {
            self.waiting.pop_back();
        }
        self.waiting.push_back((n, at));
    }

    /// The last input a picture grabbed at `t` shows.
    pub fn at_grab(&mut self, t: f64) -> u32 {
        let done = self
            .waiting
            .iter()
            .take_while(|(_, at)| painted(*at, t))
            .count();
        if let Some((n, _)) = self.waiting.drain(..done).next_back() {
            self.shown = n;
        }
        self.shown
    }
}
/// What hears the worker.
pub type Events = Arc<dyn Fn(PlugwinEvent) + Send + Sync>;

/// A capture's counts over its last minute.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rate {
    /// Pictures grabbed.
    pub grabs: u32,
    /// Frames sent (a picture unlike the last one).
    pub sent: u32,
    /// Grabs that failed.
    pub failed: u32,
    /// The mean grab and encode time (ms) and frame size (bytes).
    pub grab_ms: f64,
    pub encode_ms: f64,
    pub bytes: f64,
    /// The last picture's size.
    pub width: u32,
    pub height: u32,
    /// The largest gap (ms) between two injections of a contact.
    pub gap_ms: f64,
}

/// What the worker tells the router.
#[derive(Debug, Clone, PartialEq)]
pub enum PlugwinEvent {
    /// The window of `session` went away.
    Lost { session: u32 },
    /// Contact `contact` on `session` ended: its point was not the
    /// editor's.
    ContactEnded {
        session: u32,
        contact: u32,
        why: String,
    },
    /// The capture of `session` over the last minute.
    Rate { session: u32, rate: Rate },
    /// The picture of `session` was grabbed at another size than the worker
    /// knew, with no resize on its way (PR G: one that landed after its
    /// wait, a window resized on the PC): its points are of this size now.
    Sized {
        session: u32,
        width: u32,
        height: u32,
    },
}

/// `elapsed` in milliseconds.
pub fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// Whether a phase of contact `contact` on `session` goes to the screen
/// while `held` (its session and number) is down: a down only when none is
/// (the router ends a contact before its next down), any other phase only
/// of the contact held (one the worker already ended, or another session's,
/// is dropped).
pub fn accepts(held: Option<(u32, u32)>, session: u32, contact: u32, phase: Phase) -> bool {
    match held {
        None => phase == Phase::Down,
        Some(down) => phase != Phase::Down && down == (session, contact),
    }
}

/// Whether a phase waits out a resize of its editor on its way
/// (`resizing`: posted, not yet settled; #74 review, round 2): a down does
/// (Live lands the posted size before injected input, so Pro-Q would lay
/// itself out under it); any other phase is of a contact already down and
/// goes.
pub fn waits_out(phase: Phase, resizing: bool) -> bool {
    phase == Phase::Down && resizing
}

/// Whether a contact last injected `since_ms` ago is due again.
pub fn keepalive_due(since_ms: f64) -> bool {
    since_ms >= KEEPALIVE_MS
}

/// Whether an editor whose last contact ended `since_ms` ago is quiet
/// again ([`QUIET_MS`]): something may be posted to its window. One whose
/// contact never ended is (`+∞`); a time that is no number is not.
pub fn quiet(since_ms: f64) -> bool {
    since_ms >= QUIET_MS
}

/// What the release of an editor posts to its window (#74's final review,
/// its first note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repost {
    /// Nothing: its rectangle was never changed, or Live closed the editor.
    Nothing,
    /// The take's rectangle, its place and its size.
    Take,
    /// Nothing, although its rectangle was changed: a contact of its
    /// session ended less than [`QUIET_MS`] ago (the release's own end of a
    /// contact down included), and Live's thread would land the rectangle
    /// before that injected up. The editor stays where it is (a stop or a
    /// failed guard with a finger down: this path leaves it open in Live
    /// already).
    NotQuiet,
}

/// What the release of an editor whose rectangle was `changed` since its
/// take posts, `closed`: Live closed the editor; `since_ms`: since the
/// last contact of its session ended ([`quiet`]).
pub fn repost(changed: bool, closed: bool, since_ms: f64) -> Repost {
    if !changed || closed {
        Repost::Nothing
    } else if quiet(since_ms) {
        Repost::Take
    } else {
        Repost::NotQuiet
    }
}

/// Whether a capture last made `since_ms` ago is due.
pub fn capture_due(since_ms: f64) -> bool {
    since_ms >= CAPTURE_MS
}

/// Whether an equal picture last handed to the encoder `since_ms` ago goes
/// again ([`REFRESH_MS`]).
pub fn refresh_due(since_ms: f64) -> bool {
    since_ms >= REFRESH_MS
}

/// Whether the windows last listed `since_ms` ago for an open are due.
pub fn poll_due(since_ms: f64) -> bool {
    since_ms >= FIND_POLL_MS
}

/// Whether an open's wait for its window, `waited_ms` so far, is over.
pub fn find_over(waited_ms: f64) -> bool {
    waited_ms >= FIND_MS
}

/// Whether a capture's minute is over, `since_ms` after it began.
pub fn rate_due(since_ms: f64) -> bool {
    since_ms >= RATE_MS
}

/// The inert spot of a picture `width` wide (see [`INERT_X`]).
pub fn inert_spot(width: u32) -> (i32, i32) {
    ((f64::from(width) * INERT_X).round() as i32, INERT_Y)
}

/// Whether a grab of a picture of `size` tells the router of a new size
/// (PR G): not the size the worker knew (`known`), with no resize on its way
/// (`resizing`: that one's settling says it).
pub fn regrown(resizing: bool, size: (u32, u32), known: (u32, u32)) -> bool {
    !resizing && size != known
}

/// The windows in the list `now` that the list `before` lacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    None,
    One(WindowId),
    Several,
}

/// The new window of an open: the one in `now` not in `before`.
pub fn pick(before: &[WindowId], now: &[WindowId]) -> Pick {
    let new: Vec<WindowId> = now
        .iter()
        .filter(|w| !before.contains(w))
        .copied()
        .collect();
    match new.as_slice() {
        [] => Pick::None,
        [one] => Pick::One(*one),
        _ => Pick::Several,
    }
}

/// What an open does with what it found after `waited_ms` (`ready`: the
/// one new window can be taken now, its picture there): take it, fail once
/// the wait is over without exactly one that is ready, or wait on.
pub fn found(pick: &Pick, ready: bool, waited_ms: f64) -> Option<Result<WindowId, &'static str>> {
    match pick {
        Pick::One(window) if ready => Some(Ok(*window)),
        Pick::One(_) => find_over(waited_ms).then_some(Err(reason::NO_PICTURE)),
        Pick::None => find_over(waited_ms).then_some(Err(NO_WINDOW)),
        Pick::Several => find_over(waited_ms).then_some(Err(SEVERAL)),
    }
}

/// What a taken window does `waited_ms` after its take (`on_top`: its
/// posted z-order change landed): its take answers, fails once the wait is
/// over, or waits on.
pub fn placed(on_top: bool, waited_ms: f64) -> Option<Result<(), &'static str>> {
    if on_top {
        Some(Ok(()))
    } else {
        find_over(waited_ms).then_some(Err(NOT_ON_TOP))
    }
}

/// A picture as a JPEG.
pub fn encode(pixels: &Pixels, quality: u8) -> Result<Vec<u8>, String> {
    let width = u16::try_from(pixels.width).map_err(|_| "a picture over 65535 px wide")?;
    let height = u16::try_from(pixels.height).map_err(|_| "a picture over 65535 px high")?;
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, quality)
        .encode(&pixels.bgra, width, height, jpeg_encoder::ColorType::Bgra)
        .map_err(|e| e.to_string())?;
    Ok(jpeg)
}

/// A capture's running counts.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Counts {
    grabs: u32,
    sent: u32,
    failed: u32,
    grab_ms: f64,
    encode_ms: f64,
    bytes: f64,
    width: u32,
    height: u32,
    gap_ms: f64,
}

impl Counts {
    /// A contact injected `gap_ms` after its last injection.
    fn injected(&mut self, gap_ms: f64) {
        self.gap_ms = self.gap_ms.max(gap_ms);
    }

    /// A grab that took `ms` (`ok`: it gave a picture).
    fn grabbed(&mut self, ms: f64, ok: bool) {
        self.grabs += 1;
        self.grab_ms += ms;
        if !ok {
            self.failed += 1;
        }
    }

    /// A picture the encoder took `ms` for: its JPEG's size, none when it
    /// failed.
    fn encoded(&mut self, ms: f64, bytes: Option<usize>) {
        match bytes {
            Some(bytes) => {
                self.sent += 1;
                self.encode_ms += ms;
                self.bytes += bytes as f64;
            }
            None => self.failed += 1,
        }
    }

    /// The minute's rate: means over the grabs and the frames sent.
    fn rate(&self) -> Rate {
        let mean = |total: f64, n: u32| if n == 0 { 0.0 } else { total / f64::from(n) };
        Rate {
            grabs: self.grabs,
            sent: self.sent,
            failed: self.failed,
            grab_ms: mean(self.grab_ms, self.grabs),
            encode_ms: mean(self.encode_ms, self.sent),
            bytes: mean(self.bytes, self.sent),
            width: self.width,
            height: self.height,
            gap_ms: self.gap_ms,
        }
    }
}

/// An editor the worker holds.
struct Editor {
    taken: Taken,
    /// Where its frames go (none: not captured).
    sink: Option<FrameSink>,
    /// The last picture handed to the encoder, to skip an equal one, and
    /// the input it showed (PR J: an equal picture that shows a newer input
    /// goes again, so the page learns its picture caught up).
    last: Option<Arc<Pixels>>,
    last_shown: u32,
    /// When the last picture went to the encoder (worker ms): an equal one
    /// goes again [`REFRESH_MS`] after it.
    put: f64,
    /// The page's inputs its pictures show (PR J).
    shown: Shown,
    /// The part of the picture its page shows (PR L; none: the whole), the
    /// part the last frame carried (a new part of equal pixels goes), and
    /// whether a part ever went to the encoder (its guard sends the whole
    /// picture for its card).
    part: Option<Part>,
    last_crop: Option<Crop>,
    parted: bool,
    /// When it was last grabbed and when its minute began (worker ms).
    grabbed: f64,
    minute: f64,
    counts: Counts,
    /// Its picture's size as the worker last knew it (the take's, a
    /// resize's as it settled, a grab's).
    size: (u32, u32),
    /// A resize on its way (PR G).
    resizing: Option<Resizing>,
    /// Whether its size was changed since its take (a resize posted, a grab
    /// of another size): the guard and the release put it back.
    changed: bool,
    /// Whether its close guard began: no resize reaches it any more (#74
    /// review: never cleared, the guard is the editor's last step).
    guarded: bool,
    /// A resize asked while its session's contact is down, or less than
    /// [`QUIET_MS`] after one ended, posted once the contact ended and that
    /// time passed (#74 review; round 3): its area and its caller.
    deferred: Option<(Area, oneshot::Sender<Option<Resized>>)>,
    /// When the last contact of its session ended (worker ms; minus
    /// infinity: none since its take): nothing is posted to its window for
    /// [`QUIET_MS`] after it ([`quiet`]; #74 review, round 3).
    ended: f64,
}

/// A resize on its way: the size asked, when it was posted (worker ms),
/// the window's rectangle before it, and its caller, who hears how it
/// settled.
struct Resizing {
    asked: (u32, u32),
    /// The picture's size when it was posted.
    from: (u32, u32),
    posted: f64,
    before: Rect,
    reply: oneshot::Sender<Option<Resized>>,
}

/// A close guard placing its editor at [`KNOWN_SIZE`] (PR G; #74 review,
/// round 2): its placement posted last and when (worker ms; none: none
/// yet), and its caller.
struct Guarding {
    session: u32,
    posted: Option<(Placement, f64)>,
    reply: oneshot::Sender<Result<(), String>>,
}

/// A picture on its way to its JPEG: its session, its pixels, its sink.
struct Job {
    session: u32,
    shown: u32,
    /// The part of the picture it is (PR L), and the whole picture's size.
    crop: Option<Crop>,
    size: (u32, u32),
    pixels: Arc<Pixels>,
    sink: FrameSink,
}

/// What the encoder made of one picture: its session, how long it took
/// (ms), and its JPEG's size (none: it failed).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Encoded {
    session: u32,
    ms: f64,
    bytes: Option<usize>,
}

/// What the worker and its encoder share: the one picture waiting (a newer
/// one replaces it: the newest wins), what the encoder made since the
/// worker last looked, the sessions whose frames may still go, and the
/// stop.
#[derive(Default)]
struct Handoff {
    waiting: Option<Job>,
    done: Vec<Encoded>,
    captured: BTreeSet<u32>,
    stopped: bool,
}

/// Whether the encoder has nothing to do yet: no picture waiting, no stop.
fn idle(handoff: &mut Handoff) -> bool {
    handoff.waiting.is_none() && !handoff.stopped
}

/// The hand-off and the encoder's wake-up.
#[derive(Default)]
struct Shared {
    handoff: Mutex<Handoff>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Handoff> {
        self.handoff.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next picture to encode (it waits for one); none once stopped.
    fn wait_job(&self) -> Option<Job> {
        let mut handoff = self
            .ready
            .wait_while(self.lock(), idle)
            .unwrap_or_else(PoisonError::into_inner);
        if handoff.stopped {
            return None;
        }
        handoff.waiting.take()
    }
}

/// The encoder thread: each picture handed to it as a JPEG to its sink,
/// until the stop.
fn encode_frames(shared: &Shared) {
    while let Some(job) = shared.wait_job() {
        deliver(shared, job);
    }
}

/// One picture as a JPEG to its sink, while its session is still captured:
/// one the guard, a release or a lost window forgot while it was encoded
/// gets no frame, and nothing is counted. What it made is reported before
/// the sink gets it, so the worker's counts hold every frame a sink got.
/// The check, the report and the sink run under the hand-off's lock, so a
/// forget never lands between the check and the frame (the sink only
/// writes an outbox slot and the card's picture).
fn deliver(shared: &Shared, job: Job) {
    let started = Instant::now();
    let jpeg = encode(&job.pixels, QUALITY);
    let ms = millis(started.elapsed());
    if let Err(why) = &jpeg {
        tracing::warn!(session = job.session, why = %why, "a plug-in editor's picture could not be encoded");
    }
    let mut handoff = shared.lock();
    if !handoff.captured.contains(&job.session) {
        return;
    }
    handoff.done.push(Encoded {
        session: job.session,
        ms,
        bytes: jpeg.as_ref().ok().map(Vec::len),
    });
    if let Ok(jpeg) = jpeg {
        (job.sink)(Frame {
            session: job.session,
            shown: job.shown,
            crop: job.crop,
            size: job.size,
            jpeg: Bytes::from(jpeg),
        });
    }
}

/// The JPEG encoder: its thread and the hand-off. Dropped, it stops.
struct Encoder {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Encoder {
    /// Starts the encoder's thread.
    fn spawn() -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let theirs = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("plugwin-jpeg".to_string())
            .spawn(move || encode_frames(&theirs))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// `session`'s frames may go to its sink from now on.
    fn capture(&self, session: u32) {
        self.shared.lock().captured.insert(session);
    }

    /// A picture to encode for its sink: it replaces one still waiting
    /// (the newest wins).
    fn put(&self, job: Job) {
        self.shared.lock().waiting = Some(job);
        self.shared.ready.notify_one();
    }

    /// `session`'s frames stop: a picture of it still waiting is dropped,
    /// and one being encoded never reaches its sink.
    fn forget(&self, session: u32) {
        let mut handoff = self.shared.lock();
        handoff.captured.remove(&session);
        if handoff
            .waiting
            .as_ref()
            .is_some_and(|job| job.session == session)
        {
            handoff.waiting = None;
        }
    }

    /// What the encoder made since the last look.
    fn done(&self) -> Vec<Encoded> {
        std::mem::take(&mut self.shared.lock().done)
    }

    /// The encoder ends (a picture still waiting is dropped) and its thread
    /// is joined, bounded ([`STOP_POLLS`] looks [`STOP_POLL`] apart): a
    /// thread that never ends (it should not) cannot hold the worker's end
    /// nor the hub's stop, and a mutant that keeps it waiting fails its
    /// tests instead of hanging them.
    fn stop(&mut self) {
        {
            let mut handoff = self.shared.lock();
            handoff.stopped = true;
            handoff.waiting = None;
        }
        self.shared.ready.notify_one();
        let Some(thread) = self.thread.take() else {
            return;
        };
        for _ in 0..STOP_POLLS {
            if thread.is_finished() {
                let _ = thread.join();
                return;
            }
            std::thread::sleep(STOP_POLL);
        }
        tracing::warn!("the JPEG encoder's thread did not end in time: it is left behind");
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// An open waiting for its window, and then for it to come on top.
struct Finding {
    session: u32,
    before: Vec<WindowId>,
    started: f64,
    polled: Option<f64>,
    /// The window taken, waiting for its place on top, and when it was
    /// taken (worker ms).
    placing: Option<(Taken, f64)>,
    reply: oneshot::Sender<Result<(u32, u32), String>>,
}

enum Command {
    List(oneshot::Sender<Result<Vec<WindowId>, String>>),
    Take {
        session: u32,
        before: Vec<WindowId>,
        reply: oneshot::Sender<Result<(u32, u32), String>>,
    },
    Capture {
        session: u32,
        sink: FrameSink,
    },
    Resize {
        session: u32,
        area: Area,
        reply: oneshot::Sender<Option<Resized>>,
    },
    Touch {
        session: u32,
        contact: u32,
        phase: Phase,
        at: (i32, i32),
    },
    Handled {
        session: u32,
        n: u32,
    },
    View {
        session: u32,
        part: Option<Part>,
    },
    Guard {
        session: u32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Release {
        session: u32,
        closed: bool,
        reply: oneshot::Sender<()>,
    },
    Stop,
}

/// The contact on the screen: its session, its number (the router's), its
/// last point and when it was last injected (worker ms).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    session: u32,
    contact: u32,
    at: (i32, i32),
    sent: f64,
}

/// The worker's clock: milliseconds since it started (a test's stands
/// where the test set it).
type Clock = Box<dyn Fn() -> f64 + Send>;

/// The worker: its backend, the editors it holds by session, the opens
/// waiting for their windows, the guards waiting for their editors'
/// original size, the contact on the screen, the encoder and its clock.
struct Worker {
    backend: Box<dyn Backend>,
    events: Events,
    editors: BTreeMap<u32, Editor>,
    finding: Vec<Finding>,
    guarding: Vec<Guarding>,
    contact: Option<Held>,
    encoder: Encoder,
    clock: Clock,
}

impl Worker {
    /// Its clock now (ms): read afresh at every look and every injection.
    fn now(&self) -> f64 {
        (self.clock)()
    }

    /// Serves commands until `Stop` or the last handle is gone, looking at
    /// its clocks at least every [`STEP`]; then hands every window back.
    fn run(mut self, rx: &mpsc::Receiver<Command>) {
        if let Err(why) = self.backend.start() {
            tracing::error!(why = %why, "the plug-in window worker could not set up its thread: touches will fail");
        }
        loop {
            match rx.recv_timeout(STEP) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(command) => self.command(command),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.step();
        }
        self.shutdown();
    }

    /// One command.
    fn command(&mut self, command: Command) {
        match command {
            Command::List(reply) => {
                let _ = reply.send(self.backend.editors());
            }
            Command::Take {
                session,
                before,
                reply,
            } => {
                self.backend.live_opened();
                let started = self.now();
                self.finding.push(Finding {
                    session,
                    before,
                    started,
                    polled: None,
                    placing: None,
                    reply,
                });
            }
            Command::Capture { session, sink } => {
                if let Some(editor) = self.editors.get_mut(&session) {
                    editor.sink = Some(sink);
                    self.encoder.capture(session);
                }
            }
            Command::Resize {
                session,
                area,
                reply,
            } => self.resize(session, area, reply),
            Command::Touch {
                session,
                contact,
                phase,
                at,
            } => self.touch(session, contact, phase, at),
            Command::Handled { session, n } => {
                let now = self.now();
                if let Some(editor) = self.editors.get_mut(&session) {
                    editor.shown.handled(n, now);
                }
            }
            Command::View { session, part } => {
                if let Some(editor) = self.editors.get_mut(&session) {
                    editor.part = part;
                }
            }
            Command::Guard { session, reply } => self.guard(session, reply),
            Command::Release {
                session,
                closed,
                reply,
            } => {
                self.release(session, closed);
                let _ = reply.send(());
            }
            Command::Stop => {}
        }
    }

    /// The clocks, each read afresh: the encoder's frames counted, the
    /// contact's keep-alive, the opens' polls, the resizes and the guards,
    /// the captures and their minutes. The keep-alive
    /// is looked at first, after the polls (a list of the windows, a take,
    /// the pictures' sizes) and after each capture (a grab blocks for its
    /// time): it never waits for a whole step.
    fn step(&mut self) {
        self.count_encoded();
        self.keep_alive();
        self.find();
        self.deferred();
        self.resizes();
        self.guards();
        self.keep_alive();
        let sessions: Vec<u32> = self.editors.keys().copied().collect();
        for session in sessions {
            self.capture(session);
            self.keep_alive();
        }
    }

    /// The contact down, injected again at its last point once
    /// [`KEEPALIVE_MS`] passed since its last injection.
    fn keep_alive(&mut self) {
        let now = self.now();
        let Some(held) = self.contact.filter(|held| keepalive_due(now - held.sent)) else {
            return;
        };
        self.touch(held.session, held.contact, Phase::Update, held.at);
    }

    /// What the encoder made since the last look, into its editors' counts.
    fn count_encoded(&mut self) {
        for done in self.encoder.done() {
            if let Some(editor) = self.editors.get_mut(&done.session) {
                editor.counts.encoded(done.ms, done.bytes);
                // A picture that never became a frame is no last picture:
                // the next grab goes again, with what it shows (PR J review).
                if done.bytes.is_none() {
                    editor.last = None;
                }
            }
        }
    }

    /// The opens whose poll is due look for their window; a window taken
    /// already is looked at every step until it is on top.
    fn find(&mut self) {
        let now = self.now();
        for mut finding in std::mem::take(&mut self.finding) {
            if let Some((taken, at)) = finding.placing.take() {
                self.place(finding, taken, at);
                continue;
            }
            // A poll is due on its interval, and always once the wait is
            // over (its answer never waits for the next poll).
            if !(finding.polled.is_none_or(|at| poll_due(now - at))
                || find_over(now - finding.started))
            {
                self.finding.push(finding);
                continue;
            }
            let decision = match self.backend.editors() {
                Ok(list) => {
                    let new = pick(&finding.before, &list);
                    let ready = match new {
                        Pick::One(window) => self.backend.ready(window),
                        Pick::None | Pick::Several => false,
                    };
                    found(&new, ready, now - finding.started)
                        .map(|found| found.map_err(str::to_string))
                }
                Err(why) => Some(Err(why)),
            };
            match decision {
                None => {
                    finding.polled = Some(now);
                    self.finding.push(finding);
                }
                Some(Err(why)) => {
                    let _ = finding.reply.send(Err(why));
                }
                Some(Ok(window)) => match self.backend.take(window) {
                    Ok(taken) => self.place(finding, taken, now),
                    Err(why) => {
                        let _ = finding.reply.send(Err(why));
                    }
                },
            }
        }
    }

    /// A window taken at `at`, looked at: once it is on top the editor is
    /// the worker's and its take answers its picture's size; one not on top
    /// once the wait is over is handed back and its take fails; else it
    /// waits on.
    fn place(&mut self, mut finding: Finding, taken: Taken, at: f64) {
        let now = self.now();
        match placed(self.backend.on_top(&taken), now - at) {
            None => {
                finding.placing = Some((taken, at));
                self.finding.push(finding);
            }
            Some(Err(why)) => {
                self.backend.release(&taken);
                let _ = finding.reply.send(Err(why.to_string()));
            }
            Some(Ok(())) => {
                let size = (taken.width, taken.height);
                if let Some((width, height)) = other_size(size) {
                    tracing::info!(
                        session = finding.session,
                        width,
                        height,
                        "a Pro-Q 4 editor was taken at another size than its inert spot's: its guard puts it at 1349 x 809 first"
                    );
                }
                self.editors.insert(
                    finding.session,
                    Editor {
                        taken,
                        sink: None,
                        last: None,
                        last_shown: 0,
                        put: f64::NEG_INFINITY,
                        shown: Shown::default(),
                        part: None,
                        last_crop: None,
                        parted: false,
                        grabbed: f64::NEG_INFINITY,
                        minute: now,
                        counts: Counts::default(),
                        size,
                        resizing: None,
                        changed: false,
                        guarded: false,
                        deferred: None,
                        ended: f64::NEG_INFINITY,
                    },
                );
                let _ = finding.reply.send(Ok(size));
            }
        }
    }

    /// A resize of `session`'s picture for a page's `area` (PR G): posted
    /// ([`Worker::start_resize`]), then looked at every step until it
    /// settles ([`Worker::resizes`]). While its editor is held off
    /// ([`Worker::holds_off`]: a contact of `session` down, or ended less
    /// than [`QUIET_MS`] ago) it waits, and a newer one replaces it (its
    /// caller hears none; [`Worker::deferred`]).
    fn resize(&mut self, session: u32, area: Area, reply: oneshot::Sender<Option<Resized>>) {
        let now = self.now();
        if self.holds_off(session, now)
            && let Some(editor) = self.editors.get_mut(&session)
        {
            if let Some((_, older)) = editor.deferred.replace((area, reply)) {
                let _ = older.send(None);
            }
            return;
        }
        self.start_resize(session, area, reply);
    }

    /// Whether nothing may be posted to `session`'s editor at `now` (#74
    /// review): a contact of `session` is down (Pro-Q never lays itself out
    /// under a finger), or one ended less than [`QUIET_MS`] ago (round 3:
    /// Live's thread lands a posted rectangle before the injected end). No
    /// editor: nothing holds off.
    fn holds_off(&self, session: u32, now: f64) -> bool {
        let touched = self.contact.is_some_and(|held| held.session == session);
        self.editors
            .get(&session)
            .is_some_and(|editor| touched || !quiet(now - editor.ended))
    }

    /// A resize of `session`'s picture for `area` posted now
    /// ([`Worker::post_size`]) and looked at every step until it settles
    /// ([`Worker::resizes`]). Nothing posted: its caller hears none at once.
    /// A resize still on its way is replaced (its caller hears nothing).
    fn start_resize(&mut self, session: u32, area: Area, reply: oneshot::Sender<Option<Resized>>) {
        let now = self.now();
        let Some((asked, before)) = self.post_size(session, area) else {
            let _ = reply.send(None);
            return;
        };
        if let Some(editor) = self.editors.get_mut(&session) {
            editor.resizing = Some(Resizing {
                asked,
                from: editor.size,
                posted: now,
                before,
                reply,
            });
        }
    }

    /// Posts the picture size for `area` to `session`'s window: the size
    /// asked and the window's rectangle before. None for no editor, one
    /// whose guard began (its tap needs the size it puts back, and Live
    /// closes the editor at the size it has: #74 review), an area with no
    /// shape, a room smaller than the minimum, or a work area, a window or a
    /// size the backend could not read or post. The window stays where it
    /// stands when it fits there, else it moves into the work area
    /// ([`inside`]).
    fn post_size(&mut self, session: u32, area: Area) -> Option<((u32, u32), Rect)> {
        let editor = self.editors.get_mut(&session)?;
        if editor.guarded {
            return None;
        }
        let taken = &editor.taken;
        let read = |what: Result<Rect, String>| {
            what.map_err(|why| tracing::warn!(session, why = %why, "a Pro-Q 4 editor's place on the screen could not be read: its size stays"))
                .ok()
        };
        let work = read(self.backend.work(taken))?;
        let now = read(self.backend.rect(taken))?;
        let asked = editor_size(area, room(work, taken), self.backend.min_size())?;
        let rect = inside(work, now, window_size(taken, asked));
        self.backend
            .resize(taken, rect)
            .map_err(|why| tracing::warn!(session, why = %why, "a Pro-Q 4 editor's new size could not be posted"))
            .ok()?;
        editor.changed = true;
        Some((asked, now))
    }

    /// The resizes on their way, each looked at: one whose picture is the
    /// size asked, or whose wait is over, settles ([`settled`]): its
    /// picture's size is the editor's, and its caller hears how. One that
    /// settled unlanded gets the window's rectangle before it posted back
    /// ([`revert`]; #74 review).
    fn resizes(&mut self) {
        let now = self.now();
        for (session, editor) in &mut self.editors {
            let Some(resizing) = editor.resizing.as_ref() else {
                continue;
            };
            let client = self.backend.client(&editor.taken).unwrap_or(editor.size);
            let waited = now - resizing.posted;
            let Some(ok) = settled(client, resizing.asked, resizing.from, waited) else {
                continue;
            };
            editor.size = client;
            if let Some(resizing) = editor.resizing.take() {
                let reverted = !ok && revert(self.backend.as_mut(), &editor.taken, resizing.before);
                tracing::info!(
                    session = *session,
                    asked_w = resizing.asked.0,
                    asked_h = resizing.asked.1,
                    width = client.0,
                    height = client.1,
                    ok,
                    reverted,
                    ms = waited,
                    "a Pro-Q 4 editor's resize settled"
                );
                let _ = resizing.reply.send(Some(Resized {
                    asked: resizing.asked,
                    client,
                    ok,
                    ms: waited,
                    reverted,
                }));
            }
        }
    }

    /// The resizes waiting for a contact to end, each posted once its
    /// editor is no longer held off ([`Worker::holds_off`]: no contact of
    /// its session down, and [`QUIET_MS`] passed since the last one ended;
    /// #74 review, round 3).
    fn deferred(&mut self) {
        let now = self.now();
        let due: Vec<u32> = self
            .editors
            .iter()
            .filter(|(session, editor)| {
                editor.deferred.is_some() && !self.holds_off(**session, now)
            })
            .map(|(session, _)| *session)
            .collect();
        for session in due {
            let waiting = self
                .editors
                .get_mut(&session)
                .and_then(|editor| editor.deferred.take());
            if let Some((area, reply)) = waiting {
                self.start_resize(session, area, reply);
            }
        }
    }

    /// The guards placing their editor at [`KNOWN_SIZE`], each looked at
    /// ([`Worker::guard_look`]).
    fn guards(&mut self) {
        let now = self.now();
        for guard in std::mem::take(&mut self.guarding) {
            self.guard_look(guard, now);
        }
    }

    /// A guard's look at its editor at `now` ([`guard_step`], #74 review,
    /// round 2): it waits while a resize of the editor settles, and until
    /// [`QUIET_MS`] after a contact of its session ended (round 3: its own
    /// up, injected as it began; the wait passes on its own); posts its
    /// first placement, the known rectangle ([`known_rect`]) a pixel aside,
    /// inward and never where the window stands ([`aside`]); once the
    /// window reads that, the known rectangle; once the window reads that
    /// with its picture at exactly [`KNOWN_SIZE`], it taps the inert spot.
    /// A placement not seen to land in time, or one that could not be
    /// worked out or posted, fails the guard (no tap at a spot of an
    /// unknown layout: the router then leaves the editor open). A guard
    /// whose window went meanwhile fails.
    fn guard_look(&mut self, mut guard: Guarding, now: f64) {
        let Some(editor) = self.editors.get_mut(&guard.session) else {
            let _ = guard.reply.send(Err(NO_EDITOR.to_string()));
            return;
        };
        let taken = &editor.taken;
        let window = self.backend.rect(taken);
        let read = window.as_ref().ok().copied();
        let client = self.backend.client(taken).ok();
        let posted = guard.posted.map(|(placement, at)| (placement, now - at));
        let settling = editor.resizing.is_some() || !quiet(now - editor.ended);
        let placement = match guard_step(settling, posted, read, client) {
            GuardStep::Wait => {
                self.guarding.push(guard);
                return;
            }
            GuardStep::Aside => window.and_then(|then| {
                let work = self.backend.work(taken)?;
                Ok(Placement::Aside(aside(known_rect(work, taken), then, work)))
            }),
            GuardStep::Known => self
                .backend
                .work(taken)
                .map(|work| Placement::Known(known_rect(work, taken))),
            GuardStep::Tap => {
                editor.size = KNOWN_SIZE;
                // At the take's rectangle again (an editor taken at the
                // known size where it stands): nothing for the release to
                // put back.
                editor.changed = read != Some(editor.taken.rect);
                let taken = editor.taken.clone();
                tracing::info!(
                    session = guard.session,
                    "a Pro-Q 4 editor is at the size its inert spot is known at, nothing posted before it waits: the guard taps"
                );
                let tapped = guard_tap(self.backend.as_mut(), &taken, inert_spot(KNOWN_SIZE.0));
                let _ = guard.reply.send(tapped);
                return;
            }
            GuardStep::Fail(placement) => {
                let _ = guard.reply.send(Err(not_landed(placement, read, client)));
                return;
            }
        };
        let posting = placement.and_then(|placement| {
            self.backend.resize(taken, placement.rect())?;
            Ok(placement)
        });
        match posting {
            Ok(placement) => {
                editor.changed = true;
                guard.posted = Some((placement, now));
                self.guarding.push(guard);
            }
            Err(why) => {
                let _ = guard.reply.send(Err(format!("{NOT_BACK}: {why}")));
            }
        }
    }

    /// One capture of `session` when due: a window gone is lost; a picture
    /// like the last one is skipped; another goes to the sink. The minute's
    /// counts when it is over.
    fn capture(&mut self, session: u32) {
        let now = self.now();
        let Some(editor) = self.editors.get_mut(&session) else {
            return;
        };
        if !self.backend.alive(&editor.taken) {
            let gone = editor.taken.clone();
            if let Some(editor) = self.editors.remove(&session) {
                self.last_rate(session, &editor.counts);
            }
            self.encoder.forget(session);
            if let Some(held) = self.contact.filter(|held| held.session == session) {
                self.contact = None;
                // The window is gone, so nothing lands: the cancel ends the
                // backend's contact (the Windows one puts the cursor back).
                let _ = self.backend.touch(&gone, Phase::Cancel, held.at);
            }
            (self.events)(PlugwinEvent::Lost { session });
            return;
        }
        if rate_due(now - editor.minute) {
            let rate = editor.counts.rate();
            editor.counts = Counts::default();
            editor.minute = now;
            (self.events)(PlugwinEvent::Rate { session, rate });
        }
        let Some(sink) = editor.sink.clone() else {
            return;
        };
        if !capture_due(now - editor.grabbed) {
            return;
        }
        editor.grabbed = now;
        let started = Instant::now();
        let grabbed = self.backend.grab(&editor.taken);
        editor
            .counts
            .grabbed(millis(started.elapsed()), grabbed.is_ok());
        let pixels = match grabbed {
            Ok(pixels) => pixels,
            Err(why) => {
                tracing::debug!(session, why = %why, "a picture of a plug-in editor could not be grabbed");
                return;
            }
        };
        editor.counts.width = pixels.width;
        editor.counts.height = pixels.height;
        let size = (pixels.width, pixels.height);
        if regrown(editor.resizing.is_some(), size, editor.size) {
            editor.size = size;
            editor.changed = true;
            (self.events)(PlugwinEvent::Sized {
                session,
                width: size.0,
                height: size.1,
            });
        }
        // The part the page shows (PR L), or the whole picture. A part is
        // of the picture's size the page drew: a picture of another size
        // (a turned screen, a new area, Pro-Q's own resize) goes whole until
        // the page's part for it lands, and one back at that size is cut
        // again (the PR L re-review: the page never saw the size between).
        let crop = editor
            .part
            .filter(|part| part.picture == size)
            .and_then(|part| crop_within(part.crop, size));
        let pixels = match crop {
            Some(crop) => cropped(&pixels, crop),
            None => pixels,
        };
        let shown = editor.shown.at_grab(now);
        if editor.last.as_deref() == Some(&pixels)
            && crop == editor.last_crop
            && shown == editor.last_shown
            && !refresh_due(now - editor.put)
        {
            return;
        }
        let pixels = Arc::new(pixels);
        editor.last = Some(Arc::clone(&pixels));
        editor.last_shown = shown;
        editor.last_crop = crop;
        editor.parted |= crop.is_some();
        editor.put = now;
        self.encoder.put(Job {
            session,
            shown,
            crop,
            size,
            pixels,
            sink,
        });
    }

    /// A phase of contact `contact` on `session`'s window: one contact on
    /// the screen ([`accepts`]), no down while a resize of the editor is
    /// on its way ([`waits_out`]); stamped when it goes, its gap since the
    /// contact's last injection counted; a refused down or update ends the
    /// contact and says so. A contact that ends holds its editor off for
    /// [`QUIET_MS`] ([`Worker::contact_ended`]).
    fn touch(&mut self, session: u32, contact: u32, phase: Phase, at: (i32, i32)) {
        let Some((taken, resizing)) = self
            .editors
            .get(&session)
            .map(|e| (e.taken.clone(), e.resizing.is_some()))
        else {
            return;
        };
        let held = self.contact.map(|held| (held.session, held.contact));
        if !accepts(held, session, contact, phase) || waits_out(phase, resizing) {
            // A down while another contact is still down (the router let it
            // go after it ended that one, before this worker did), or while
            // its editor's resize is on its way (#74 review, round 2): it
            // never goes, and the router hears its contact ended. Any other
            // phase dropped is of a contact already ended.
            if phase == Phase::Down {
                (self.events)(PlugwinEvent::ContactEnded {
                    session,
                    contact,
                    why: BUSY.to_string(),
                });
            }
            return;
        }
        // Accepted: a down has no contact before it, any other phase is of
        // the contact held. The clock is read as it goes.
        let now = self.now();
        let gap = self.contact.map(|held| now - held.sent);
        match self.backend.touch(&taken, phase, at) {
            Ok(()) => {
                self.contact = (!phase.ends()).then_some(Held {
                    session,
                    contact,
                    at,
                    sent: now,
                });
                if let (Some(gap), Some(editor)) = (gap, self.editors.get_mut(&session)) {
                    editor.counts.injected(gap);
                }
                if phase.ends() {
                    self.contact_ended(session);
                }
            }
            Err(why) => {
                // A failed phase ends the contact held (a down has none
                // before it): a refused update with a cancel at its last
                // point.
                if let Some(held) = self.contact.take() {
                    if phase.checked() {
                        let _ = self.backend.touch(&taken, Phase::Cancel, held.at);
                    }
                    self.contact_ended(session);
                }
                (self.events)(PlugwinEvent::ContactEnded {
                    session,
                    contact,
                    why,
                });
            }
        }
    }

    /// A contact of `session` ended now: nothing is posted to its editor's
    /// window for [`QUIET_MS`] ([`Editor::ended`]; #74 review, round 3).
    fn contact_ended(&mut self, session: u32) {
        let now = self.now();
        if let Some(editor) = self.editors.get_mut(&session) {
            editor.ended = now;
        }
    }

    /// The contact on `session`, ended at its last point.
    fn end_contact(&mut self, session: u32) {
        let Some(held) = self.contact.filter(|held| held.session == session) else {
            return;
        };
        self.contact = None;
        if let Some(editor) = self.editors.get(&session) {
            let _ = self.backend.touch(&editor.taken, Phase::Up, held.at);
        }
        self.contact_ended(session);
    }

    /// Another session's contact down, cancelled at its last point and
    /// reported: the close guard's tap needs the PC's one injected contact.
    fn cancel_other(&mut self, session: u32) {
        let Some(held) = self.contact.filter(|held| held.session != session) else {
            return;
        };
        self.contact = None;
        if let Some(editor) = self.editors.get(&held.session) {
            let _ = self.backend.touch(&editor.taken, Phase::Cancel, held.at);
        }
        self.contact_ended(held.session);
        (self.events)(PlugwinEvent::ContactEnded {
            session: held.session,
            contact: held.contact,
            why: GUARD_CANCEL.to_string(),
        });
    }

    /// The close guard of `session`: its editor looked up first (a guard
    /// of a window already lost fails and ends no contact), then marked (no
    /// resize reaches it any more), no more frames but its whole picture for
    /// its card, its contact ended (another session's cancelled); then its
    /// first look
    /// ([`Worker::guard_look`]): its first placement posted at once unless
    /// a resize is on its way or a contact of its session ended less than
    /// [`QUIET_MS`] ago (its own just did, when one was down), and the rest
    /// at the steps (each placement seen to land, then the tap).
    fn guard(&mut self, session: u32, reply: oneshot::Sender<Result<(), String>>) {
        let Some(editor) = self.editors.get_mut(&session) else {
            let _ = reply.send(Err(NO_EDITOR.to_string()));
            return;
        };
        editor.guarded = true;
        // No picture of it reaches its sink any more: one waiting or being
        // encoded would land after the whole one below, an older picture
        // over the card's (the PR L re-review).
        self.encoder.forget(session);
        // A cropped editor's card gets its whole picture now, before the
        // guard moves it (PR L review: the edits at 2× and more were only
        // ever in parts, so the card kept the picture from before them; and
        // the whole frame after CELÝ EQ may be the one just forgotten).
        // Counted as the worker's frames are (its last rate holds it). One
        // that was whole all along sends nothing more: under 2× nothing
        // edits it, so the frame just forgotten showed no edit.
        let cropped = editor.part.is_some() || editor.parted;
        if let Some(sink) = editor.sink.take().filter(|_| cropped) {
            let started = Instant::now();
            let grabbed = self.backend.grab(&editor.taken);
            editor
                .counts
                .grabbed(millis(started.elapsed()), grabbed.is_ok());
            if let Ok(pixels) = grabbed {
                let started = Instant::now();
                let jpeg = encode(&pixels, QUALITY);
                let ms = millis(started.elapsed());
                editor.counts.encoded(ms, jpeg.as_ref().ok().map(Vec::len));
                if let Ok(jpeg) = jpeg {
                    sink(Frame {
                        session,
                        shown: editor.last_shown,
                        crop: None,
                        size: (pixels.width, pixels.height),
                        jpeg: Bytes::from(jpeg),
                    });
                }
            }
        }
        if let Some((_, waiting)) = editor.deferred.take() {
            let _ = waiting.send(None);
        }
        self.end_contact(session);
        self.cancel_other(session);
        let guard = Guarding {
            session,
            posted: None,
            reply,
        };
        let now = self.now();
        self.guard_look(guard, now);
    }

    /// The counts of an editor the worker lets go (released, its window
    /// gone) since its last minute, as its last `rate` (the PR I review: a
    /// session's last minute was lost); nothing when it grabbed nothing
    /// since.
    fn last_rate(&self, session: u32, counts: &Counts) {
        if counts.grabs > 0 {
            (self.events)(PlugwinEvent::Rate {
                session,
                rate: counts.rate(),
            });
        }
    }

    /// `session`'s window handed back: its contact ended, its rectangle at
    /// the take (place and size) posted again when it was changed and Live
    /// keeps the editor open (best effort: a guard that failed, the stop;
    /// #74 review), unless a contact of its session ended less than
    /// [`QUIET_MS`] ago (its own just did, when one was down: nothing is
    /// posted then, [`repost`]), its z-order as it was; `closed`: Live
    /// closed the editor.
    fn release(&mut self, session: u32, closed: bool) {
        self.end_contact(session);
        self.encoder.forget(session);
        let now = self.now();
        if let Some(editor) = self.editors.remove(&session) {
            self.last_rate(session, &editor.counts);
            match repost(editor.changed, closed, now - editor.ended) {
                Repost::Take => {
                    if let Err(why) = self.backend.resize(&editor.taken, editor.taken.rect) {
                        tracing::warn!(session, why = %why, "a Pro-Q 4 editor's own size could not be posted at its release");
                    }
                }
                Repost::NotQuiet => tracing::warn!(
                    session,
                    "a Pro-Q 4 editor's own size was not posted at its release: a contact of its session ended less than 100 ms before (the editor stays where it is)"
                ),
                Repost::Nothing => {}
            }
            self.backend.release(&editor.taken);
            if closed {
                self.backend.live_closed(&editor.taken);
            }
        }
    }

    /// The worker ends: every window handed back (the editors stay open in
    /// Live; a window still waiting for its place on top too, its take
    /// unanswered; a guard still placing its editor is not answered),
    /// and its encoder with it.
    fn shutdown(&mut self) {
        for finding in std::mem::take(&mut self.finding) {
            if let Some((taken, _)) = finding.placing {
                self.backend.release(&taken);
            }
        }
        self.guarding.clear();
        let sessions: Vec<u32> = self.editors.keys().copied().collect();
        for session in sessions {
            self.release(session, false);
        }
        self.encoder.stop();
    }
}

/// A resize of the `taken` window that settled unlanded (#74 review): its
/// rectangle `before` the resize posted back, which Live's thread lands
/// after the resize if that still waits there (the guard's placement lands
/// after both). Whether it went.
fn revert(backend: &mut dyn Backend, taken: &Taken, before: Rect) -> bool {
    match backend.resize(taken, before) {
        Ok(()) => true,
        Err(why) => {
            tracing::warn!(why = %why, "a Pro-Q 4 editor's size before an unlanded resize could not be posted back");
            false
        }
    }
}

/// A tap at `spot` (the guard's): down, a short hold, up. A refused down
/// taps nothing.
pub fn guard_tap(backend: &mut dyn Backend, taken: &Taken, spot: (i32, i32)) -> Result<(), String> {
    backend.touch(taken, Phase::Down, spot)?;
    std::thread::sleep(GUARD_TAP);
    backend.touch(taken, Phase::Up, spot)
}

/// The worker's thread, until a stop's wait joined it. An async lock: a
/// stop's wait holds it while it polls ([`Plugwin::stopped`]).
type Thread = tokio::sync::Mutex<Option<std::thread::JoinHandle<()>>>;

/// Whether the worker's thread ended: joined (its handle taken) once it
/// finished, or joined already.
fn joined(thread: &mut Option<std::thread::JoinHandle<()>>) -> bool {
    match thread.take() {
        None => true,
        Some(handle) if handle.is_finished() => {
            let _ = handle.join();
            true
        }
        Some(handle) => {
            *thread = Some(handle);
            false
        }
    }
}

/// The handle of the window worker (cheap to clone).
#[derive(Clone)]
pub struct Plugwin {
    tx: mpsc::Sender<Command>,
    thread: Arc<Thread>,
}

impl Plugwin {
    /// Starts the worker thread (and its encoder's) on `backend`; `events`
    /// hears it.
    pub fn spawn(backend: Box<dyn Backend>, events: Events) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        let worker = Worker {
            backend,
            events,
            editors: BTreeMap::new(),
            finding: Vec::new(),
            guarding: Vec::new(),
            contact: None,
            encoder: Encoder::spawn()?,
            clock: Box::new(move || millis(started.elapsed())),
        };
        let thread = std::thread::Builder::new()
            .name("plugwin".to_string())
            .spawn(move || worker.run(&rx))?;
        Ok(Self {
            tx,
            thread: Arc::new(tokio::sync::Mutex::new(Some(thread))),
        })
    }

    /// The editor windows on the screen now.
    pub async fn list(&self) -> Result<Vec<WindowId>, String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::List(reply))
            .map_err(|_| STOPPED.to_string())?;
        answer.await.map_err(|_| STOPPED.to_string())?
    }

    /// Takes the editor window new since `before` as `session` (Live was
    /// just asked to open it): its picture's size.
    pub async fn take(&self, session: u32, before: Vec<WindowId>) -> Result<(u32, u32), String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Take {
                session,
                before,
                reply,
            })
            .map_err(|_| STOPPED.to_string())?;
        answer.await.map_err(|_| STOPPED.to_string())?
    }

    /// `session`'s frames go to `sink` from now on.
    pub fn capture(&self, session: u32, sink: FrameSink) {
        let _ = self.tx.send(Command::Capture { session, sink });
    }

    /// Gives `session`'s editor the shape of a page's `area` (PR G): how
    /// the resize settled. None when nothing was asked (no such editor, its
    /// guard began, an area with no shape, a size the backend could not
    /// post), when a newer resize replaced it, or when the worker stopped.
    /// The command goes to the worker now, in the caller's order (#74
    /// review: the router's resize and close reach it as it handled them);
    /// the answer is awaited.
    pub fn resize(
        &self,
        session: u32,
        area: Area,
    ) -> impl Future<Output = Option<Resized>> + Send + 'static + use<> {
        let (reply, answer) = oneshot::channel();
        let _ = self.tx.send(Command::Resize {
            session,
            area,
            reply,
        });
        async move { answer.await.ok().flatten() }
    }

    /// A phase of contact `contact` on `session`'s window.
    pub fn touch(&self, session: u32, contact: u32, phase: Phase, at: (i32, i32)) {
        let _ = self.tx.send(Command::Touch {
            session,
            contact,
            phase,
            at,
        });
    }

    /// The page's input `n` on `session` was handled (PR J): after the
    /// phases it made, in the router's order.
    pub fn handled(&self, session: u32, n: u32) {
        let _ = self.tx.send(Command::Handled { session, n });
    }

    /// The part of `session`'s picture its page shows (PR L; none: the
    /// whole picture): its frames of that size carry only that from the
    /// next grab.
    pub fn view(&self, session: u32, part: Option<Part>) {
        let _ = self.tx.send(Command::View { session, part });
    }

    /// The close guard of `session`: the command goes to the worker now,
    /// in the caller's order (as [`Plugwin::resize`]); its answer is
    /// awaited (a stopped worker's: [`STOPPED`]).
    pub fn guard(
        &self,
        session: u32,
    ) -> impl Future<Output = Result<(), String>> + Send + 'static + use<> {
        let (reply, answer) = oneshot::channel();
        let _ = self.tx.send(Command::Guard { session, reply });
        async move { answer.await.map_err(|_| STOPPED.to_string())? }
    }

    /// `session`'s window handed back; `closed`: Live closed the editor.
    pub async fn release(&self, session: u32, closed: bool) {
        let (reply, answer) = oneshot::channel();
        let command = Command::Release {
            session,
            closed,
            reply,
        };
        if self.tx.send(command).is_ok() {
            let _ = answer.await;
        }
    }

    /// The worker hands every window back and ends.
    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    /// Waits, bounded ([`STOP_WAIT`]), for the worker's thread to end after
    /// a stop (its windows handed back, its contact ended): whether it did.
    /// The handle stays under the lock while it is polled, so a second
    /// caller waits for the first one's answer; once the thread ended, a
    /// later call answers at once.
    pub async fn stopped(&self) -> bool {
        let mut thread = self.thread.lock().await;
        for _ in 0..STOP_POLLS {
            if joined(&mut thread) {
                return true;
            }
            tokio::time::sleep(STOP_POLL).await;
        }
        false
    }
}

#[cfg(test)]
mod tests;
