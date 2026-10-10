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
//!   it: a picture equal to the last one is skipped, another goes to the
//!   encoder, a thread of its own: it makes the JPEG ([`QUALITY`]) and hands
//!   it to its [`FrameSink`] (the holder's socket, newest wins, and the
//!   card's last picture). One picture waits for the encoder at most, the
//!   newest (a slow encode drops pictures, never queues them), so its
//!   contacts are never held up by an encode. What blocks the worker: the
//!   grab, and the window list and its checks (an open's polls, a take);
//!   a z-order change (a take, a release) is posted. The counts go to the
//!   hub's log once a minute ([`Rate`]). A window that went away is
//!   reported [`PlugwinEvent::Lost`].
//! - **A contact:** one on the screen at a time, numbered by the router. A
//!   down goes only when no contact is down (else it ends at once, [`BUSY`]);
//!   any other phase only of the contact down ([`accepts`]). A down or an
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
//!   frame may have taken it and Pro-Q not), and both may still wait in
//!   Live's thread: until a read sees the window or its picture move from
//!   what they read then ([`Reading`], [`moved`]), the editor is in doubt
//!   and the guard trusts no size it reads. The capture goes on meanwhile,
//!   and the frames carry the picture's size. A grab of another size with
//!   no resize on its way (one that landed late, a window resized on the
//!   PC) is reported ([`PlugwinEvent::Sized`]).
//! - **The close guard** ([`Plugwin::guard`]): no resize reaches the editor
//!   from then on; the frames stop, its own contact ends at its last point
//!   (an up), another session's is cancelled there and reported
//!   ([`GUARD_CANCEL`]: the PC injects one contact, so the tap would fail or
//!   end the other engineer's drag). The inert spot is known only at
//!   [`KNOWN_SIZE`] (#74 review, I1): the guard reads the picture's size,
//!   posts the window's rectangle for that size where none is (once a
//!   resize on its way settled; at the take's place, moved into the work
//!   area when it does not fit there), and taps the inert spot only once
//!   the picture reads exactly that size and the editor is in no doubt
//!   ([`guard_step`]); not so within [`RESIZE_MS`] of its post, the guard
//!   fails and taps nothing (the router then leaves the editor open in
//!   Live). A value text field closes on a click elsewhere, and Pro-Q 4.02
//!   crashes Live when its editor closes with one open. A guard that cannot
//!   tap fails too.
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

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use fohmixer_proto::eq::{Area, reason};
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
/// How long the hub's stop waits for the worker to hand its windows back
/// and end ([`Plugwin::stopped`]): [`STOP_POLLS`] looks [`STOP_POLL`] apart.
pub const STOP_WAIT: Duration = Duration::from_secs(1);
pub const STOP_POLL: Duration = Duration::from_millis(10);
pub const STOP_POLLS: u32 = 100;
/// The worker's longest sleep between two looks at its clocks.
pub const STEP: Duration = Duration::from_millis(10);
/// How long the guard's finger stays on the inert spot.
pub const GUARD_TAP: Duration = Duration::from_millis(30);
/// The frames' JPEG quality.
pub const QUALITY: u8 = 70;
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
/// Why a down never went: another contact was still down.
pub const BUSY: &str = "busy";
/// Why a guard did not tap: its editor did not read [`KNOWN_SIZE`] in time,
/// or a resize might still land (the inert spot is known only at that
/// size).
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

/// How a resize to `asked`, posted `waited_ms` ago, its picture now
/// `client`, settles: landed (`true`), over without landing (`false`), or
/// not yet.
pub fn settled(client: (u32, u32), asked: (u32, u32), waited_ms: f64) -> Option<bool> {
    if lands(client, asked) {
        Some(true)
    } else {
        resize_over(waited_ms).then_some(false)
    }
}

/// A step of the close guard (PR G; #74 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStep {
    /// Look again at the next step.
    Wait,
    /// Post the window's rectangle for [`KNOWN_SIZE`].
    Post,
    /// The picture is at that size, nothing else on its way: tap the inert
    /// spot.
    Tap,
    /// Not so in time: no tap (the editor is left open).
    Fail,
}

/// What the guard does at a step: wait while a resize of its editor still
/// settles (posted sizes land in order, so its own goes after it); tap once
/// the picture reads exactly [`KNOWN_SIZE`] (`known`) and the editor is in
/// no doubt (`doubt`: a resize that settled unlanded may still land, and
/// nothing has moved since); else post that size (`posted`: none yet; else
/// the ms since it was posted) and fail once [`RESIZE_MS`] passed since
/// the post.
pub fn guard_step(settling: bool, doubt: bool, posted: Option<f64>, known: bool) -> GuardStep {
    let ready = known && !doubt;
    match posted {
        _ if settling => GuardStep::Wait,
        _ if ready => GuardStep::Tap,
        None => GuardStep::Post,
        Some(waited) if resize_over(waited) => GuardStep::Fail,
        Some(_) => GuardStep::Wait,
    }
}

/// Why a guard that waited its [`RESIZE_MS`] taps nothing ([`NOT_BACK`]):
/// the picture's size it read (none: unread), or that it read
/// [`KNOWN_SIZE`] in doubt.
pub fn not_known(client: Option<(u32, u32)>, doubt: bool) -> String {
    let (width, height) = client.unwrap_or((0, 0));
    if doubt && client == Some(KNOWN_SIZE) {
        format!("{NOT_BACK}: an earlier resize may still land (the picture reads {width}x{height})")
    } else {
        format!(
            "{NOT_BACK}: the picture is {width}x{height}, {}x{} asked",
            KNOWN_SIZE.0, KNOWN_SIZE.1
        )
    }
}

/// The window's rectangle for [`KNOWN_SIZE`] (#74 review): at the take's
/// place, moved into the work area `work` when it does not fit there.
pub fn known_rect(work: Rect, taken: &Taken) -> Rect {
    inside(work, taken.rect, window_size(taken, KNOWN_SIZE))
}

/// What an editor's window read when a resize settled unlanded (#74
/// review): its rectangle (none: unread) and its picture's size. Until a
/// read differs from it, the resize and its way back may both still wait
/// in the window's thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub window: Option<Rect>,
    pub client: (u32, u32),
}

/// Whether the window moved on from `reading`: its rectangle or its
/// picture's size read now differ (a read that failed tells nothing).
pub fn moved(reading: Reading, window: Option<Rect>, client: Option<(u32, u32)>) -> bool {
    let rect_moved = window
        .zip(reading.window)
        .is_some_and(|(now, then)| now != then);
    rect_moved || client.is_some_and(|now| now != reading.client)
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
}

/// Where a frame goes: its session and its JPEG.
pub type FrameSink = Arc<dyn Fn(u32, Bytes) + Send + Sync>;
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

/// Whether a contact last injected `since_ms` ago is due again.
pub fn keepalive_due(since_ms: f64) -> bool {
    since_ms >= KEEPALIVE_MS
}

/// Whether a capture last made `since_ms` ago is due.
pub fn capture_due(since_ms: f64) -> bool {
    since_ms >= CAPTURE_MS
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
    /// The last picture handed to the encoder, to skip an equal one.
    last: Option<Arc<Pixels>>,
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
    /// What its window read when a resize settled unlanded, until a read
    /// moves on from it (#74 review): meanwhile no size it reads is
    /// trusted.
    doubt: Option<Reading>,
}

/// A resize on its way: the size asked, when it was posted (worker ms),
/// the window's rectangle before it, and its caller, who hears how it
/// settled.
struct Resizing {
    asked: (u32, u32),
    posted: f64,
    before: Rect,
    reply: oneshot::Sender<Option<Resized>>,
}

/// A close guard waiting for its editor at [`KNOWN_SIZE`] (PR G; #74
/// review): when that size was posted (worker ms; none: not yet), and its
/// caller.
struct Guarding {
    session: u32,
    posted: Option<f64>,
    reply: oneshot::Sender<Result<(), String>>,
}

/// A picture on its way to its JPEG: its session, its pixels, its sink.
struct Job {
    session: u32,
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
        (job.sink)(job.session, Bytes::from(jpeg));
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

    /// `pixels` of `session` to encode for `sink`: it replaces a picture
    /// still waiting (the newest wins).
    fn put(&self, session: u32, pixels: Arc<Pixels>, sink: FrameSink) {
        self.shared.lock().waiting = Some(Job {
            session,
            pixels,
            sink,
        });
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
    /// contact's keep-alive, the opens' polls, the resizes, the editors in
    /// doubt and the guards, the captures and their minutes. The keep-alive
    /// is looked at first, after the polls (a list of the windows, a take,
    /// the pictures' sizes) and after each capture (a grab blocks for its
    /// time): it never waits for a whole step.
    fn step(&mut self) {
        self.count_encoded();
        self.keep_alive();
        self.find();
        self.resizes();
        self.doubts();
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
                self.editors.insert(
                    finding.session,
                    Editor {
                        taken,
                        sink: None,
                        last: None,
                        grabbed: f64::NEG_INFINITY,
                        minute: now,
                        counts: Counts::default(),
                        size,
                        resizing: None,
                        changed: false,
                        guarded: false,
                        doubt: None,
                    },
                );
                let _ = finding.reply.send(Ok(size));
            }
        }
    }

    /// A resize of `session`'s picture for a page's `area` (PR G): posted
    /// ([`Worker::post_size`]), then looked at every step until it settles
    /// ([`Worker::resizes`]). Nothing posted: its caller hears none at once.
    /// A resize still on its way is replaced (its caller hears nothing).
    fn resize(&mut self, session: u32, area: Area, reply: oneshot::Sender<Option<Resized>>) {
        let now = self.now();
        let Some((asked, before)) = self.post_size(session, area) else {
            let _ = reply.send(None);
            return;
        };
        if let Some(editor) = self.editors.get_mut(&session) {
            editor.resizing = Some(Resizing {
                asked,
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
            let Some(ok) = settled(client, resizing.asked, waited) else {
                continue;
            };
            editor.size = client;
            if let Some(resizing) = editor.resizing.take() {
                let reverted =
                    !ok && revert(self.backend.as_mut(), editor, resizing.before, client);
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

    /// The editors in doubt, each looked at: one whose window or picture
    /// reads otherwise than when its resize settled unlanded is in doubt no
    /// more ([`moved`]: Live's thread took what waited for it).
    fn doubts(&mut self) {
        for editor in self.editors.values_mut() {
            let Some(reading) = editor.doubt else {
                continue;
            };
            let window = self.backend.rect(&editor.taken).ok();
            let client = self.backend.client(&editor.taken).ok();
            if moved(reading, window, client) {
                editor.doubt = None;
            }
        }
    }

    /// The guards waiting for their editor at [`KNOWN_SIZE`], each looked
    /// at ([`Worker::guard_look`]).
    fn guards(&mut self) {
        let now = self.now();
        for guard in std::mem::take(&mut self.guarding) {
            self.guard_look(guard, now);
        }
    }

    /// A guard's look at its editor at `now` ([`guard_step`]): it waits
    /// while a resize of the editor settles; taps the inert spot once the
    /// picture reads exactly [`KNOWN_SIZE`] with the editor in no doubt;
    /// else posts that size ([`known_rect`]) and fails once its wait is over
    /// (no tap at a spot of an unknown layout: the router then leaves the
    /// editor open). A guard whose window went meanwhile fails.
    fn guard_look(&mut self, mut guard: Guarding, now: f64) {
        let Some(editor) = self.editors.get_mut(&guard.session) else {
            let _ = guard.reply.send(Err(NO_EDITOR.to_string()));
            return;
        };
        let client = self.backend.client(&editor.taken).ok();
        let (settling, doubt) = (editor.resizing.is_some(), editor.doubt.is_some());
        let posted = guard.posted.map(|at| now - at);
        match guard_step(settling, doubt, posted, client == Some(KNOWN_SIZE)) {
            GuardStep::Wait => self.guarding.push(guard),
            GuardStep::Post => {
                let taken = &editor.taken;
                let posting = match self.backend.work(taken) {
                    Ok(work) => self.backend.resize(taken, known_rect(work, taken)),
                    Err(why) => Err(why),
                };
                match posting {
                    Ok(()) => {
                        editor.changed = true;
                        guard.posted = Some(now);
                        self.guarding.push(guard);
                    }
                    Err(why) => {
                        let _ = guard.reply.send(Err(format!("{NOT_BACK}: {why}")));
                    }
                }
            }
            GuardStep::Tap => {
                editor.size = KNOWN_SIZE;
                // Its rectangle at the take again (an editor taken at the
                // known size where it stands): nothing for the release to
                // put back.
                editor.changed = self.backend.rect(&editor.taken).ok() != Some(editor.taken.rect);
                let taken = editor.taken.clone();
                tracing::info!(
                    session = guard.session,
                    "a Pro-Q 4 editor is at the size its inert spot is known at: the guard taps"
                );
                let tapped = guard_tap(self.backend.as_mut(), &taken, inert_spot(KNOWN_SIZE.0));
                let _ = guard.reply.send(tapped);
            }
            GuardStep::Fail => {
                let _ = guard.reply.send(Err(not_known(client, doubt)));
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
            self.editors.remove(&session);
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
        if editor.last.as_deref() == Some(&pixels) {
            return;
        }
        let pixels = Arc::new(pixels);
        editor.last = Some(Arc::clone(&pixels));
        self.encoder.put(session, pixels, sink);
    }

    /// A phase of contact `contact` on `session`'s window: one contact on
    /// the screen ([`accepts`]); stamped when it goes, its gap since the
    /// contact's last injection counted; a refused down or update ends the
    /// contact and says so.
    fn touch(&mut self, session: u32, contact: u32, phase: Phase, at: (i32, i32)) {
        let Some(taken) = self.editors.get(&session).map(|e| e.taken.clone()) else {
            return;
        };
        let held = self.contact.map(|held| (held.session, held.contact));
        if !accepts(held, session, contact, phase) {
            // A down while another contact is still down (the router let it
            // go after it ended that one, before this worker did): it never
            // goes, and the router hears its contact ended. Any other phase
            // dropped is of a contact already ended.
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
            }
            Err(why) => {
                if let Some(held) = self.contact.take()
                    && phase.checked()
                {
                    let _ = self.backend.touch(&taken, Phase::Cancel, held.at);
                }
                (self.events)(PlugwinEvent::ContactEnded {
                    session,
                    contact,
                    why,
                });
            }
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
        (self.events)(PlugwinEvent::ContactEnded {
            session: held.session,
            contact: held.contact,
            why: GUARD_CANCEL.to_string(),
        });
    }

    /// The close guard of `session`: its editor looked up first (a guard
    /// of a window already lost fails and ends no contact), then marked (no
    /// resize reaches it any more), no more frames, its contact ended
    /// (another session's cancelled); then its first look
    /// ([`Worker::guard_look`]): an editor that reads [`KNOWN_SIZE`] in no
    /// doubt with no resize on its way gets the tap at once, any other
    /// waits for the steps (the size posted, read back).
    fn guard(&mut self, session: u32, reply: oneshot::Sender<Result<(), String>>) {
        let Some(editor) = self.editors.get_mut(&session) else {
            let _ = reply.send(Err(NO_EDITOR.to_string()));
            return;
        };
        editor.guarded = true;
        editor.sink = None;
        self.encoder.forget(session);
        self.end_contact(session);
        self.cancel_other(session);
        let guard = Guarding {
            session,
            posted: None,
            reply,
        };
        let client = self.editors.get(&session).map(|editor| {
            let settling = editor.resizing.is_some() || editor.doubt.is_some();
            (settling, self.backend.client(&editor.taken).ok())
        });
        match client {
            Some((false, Some(KNOWN_SIZE))) => {
                let now = self.now();
                self.guard_look(guard, now);
            }
            _ => self.guarding.push(guard),
        }
    }

    /// `session`'s window handed back: its contact ended, its rectangle at
    /// the take (place and size) posted again when it was changed and Live
    /// keeps the editor open (best effort: a guard that failed, the stop;
    /// #74 review), its z-order as it was; `closed`: Live closed the editor.
    fn release(&mut self, session: u32, closed: bool) {
        self.end_contact(session);
        self.encoder.forget(session);
        if let Some(editor) = self.editors.remove(&session) {
            if editor.changed
                && !closed
                && let Err(why) = self.backend.resize(&editor.taken, editor.taken.rect)
            {
                tracing::warn!(session, why = %why, "a Pro-Q 4 editor's own size could not be posted at its release");
            }
            self.backend.release(&editor.taken);
            if closed {
                self.backend.live_closed(&editor.taken);
            }
        }
    }

    /// The worker ends: every window handed back (the editors stay open in
    /// Live; a window still waiting for its place on top too, its take
    /// unanswered; a guard still waiting for its restore is not answered),
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

/// A resize of `editor` that settled unlanded (#74 review): what its
/// window reads now is kept (its doubt, until a read moves on from it), and
/// its rectangle `before` the resize is posted back, which Live's thread
/// lands after the resize if that still waits there. Whether it went.
fn revert(
    backend: &mut dyn Backend,
    editor: &mut Editor,
    before: Rect,
    client: (u32, u32),
) -> bool {
    editor.doubt = Some(Reading {
        window: backend.rect(&editor.taken).ok(),
        client,
    });
    match backend.resize(&editor.taken, before) {
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
    ) -> impl Future<Output = Option<Resized>> + Send + 'static {
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

    /// The close guard of `session`: the command goes to the worker now,
    /// in the caller's order (as [`Plugwin::resize`]); its answer is
    /// awaited (a stopped worker's: [`STOPPED`]).
    pub fn guard(&self, session: u32) -> impl Future<Output = Result<(), String>> + Send + 'static {
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
