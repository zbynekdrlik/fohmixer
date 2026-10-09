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
//!   child window).
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
//!   down goes only when no contact is down; any other phase only of the
//!   contact down ([`accepts`]). A down or an update lands only when the
//!   point's window is the editor's; else nothing is injected and the
//!   contact ends ([`PlugwinEvent::ContactEnded`], with its number).
//! - **The keep-alive:** Windows ends an injected contact that gets no frame
//!   for 100 ms, so the worker injects a resting contact's last point again
//!   once [`KEEPALIVE_MS`] passed since its last injection. Each step looks
//!   at it first, again after the opens' polls and again after each grab,
//!   every time on a fresh read of its clock: a grab blocks the worker for
//!   its time, and two engineers' editors are grabbed one after the other.
//!   An injection is stamped when it goes. The largest gap between two
//!   injections of a contact goes into the minute's counts ([`Rate`]).
//! - **The close guard** ([`Plugwin::guard`]): the frames stop, its own
//!   contact ends at its last point (an up), another session's is cancelled
//!   there and reported ([`GUARD_CANCEL`]: the PC injects one contact, so
//!   the tap would fail or end the other engineer's drag), then a tap on
//!   the editor's inert spot
//!   ([`inert_spot`]): a value text field closes on a click elsewhere, and
//!   Pro-Q 4.02 crashes Live when its editor closes with one open. A guard
//!   that cannot tap fails: the router then leaves the editor open.
//! - **The release** ([`Plugwin::release`]): the window's z-order as it was,
//!   and, when Live closed the editor, [`Backend::live_closed`]. The stop
//!   ends the contact and releases every window (the editors stay open in
//!   Live); the hub's stop waits for that, bounded ([`Plugwin::stopped`]).

pub mod probe;
pub mod sim;
#[cfg(windows)]
pub mod win;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use fohmixer_proto::eq::reason;
use tokio::sync::oneshot;

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
/// The inert spot: this share of the picture's width, in the top bar's empty
/// middle (Pro-Q 4 at 100 %: x 546 of 1349, right of the logo's panel, left
/// of the undo arrows; the main session confirms it on the PC, #71).
pub const INERT_X: f64 = 0.405;
/// The inert spot's height in the picture (px): the top bar's middle.
pub const INERT_Y: i32 = 15;

/// Why an open found no window (a reason its page reads).
pub const NO_WINDOW: &str = reason::NO_WINDOW;
/// Why an open found several (a reason its page reads).
pub const SEVERAL: &str = reason::SEVERAL;
/// Why a command found no editor of its session.
pub const NO_EDITOR: &str = "no such editor";
/// Why a command found no worker (a reason a page reads).
pub const STOPPED: &str = reason::STOPPED;
/// Why the close guard ended another session's contact.
pub const GUARD_CANCEL: &str = "the close guard of another editor";

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
    /// The picture's size when it was taken.
    pub width: u32,
    pub height: u32,
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
    /// picture located.
    fn take(&mut self, window: WindowId) -> Result<Taken, String>;
    /// Whether the window is still there.
    fn alive(&mut self, taken: &Taken) -> bool;
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
/// worker last looked, and the stop.
#[derive(Default)]
struct Handoff {
    waiting: Option<Job>,
    done: Vec<Encoded>,
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
/// until the stop. What it made is reported before the sink gets it, so the
/// worker's counts hold every frame a sink got.
fn encode_frames(shared: &Shared) {
    while let Some(job) = shared.wait_job() {
        let started = Instant::now();
        let jpeg = encode(&job.pixels, QUALITY);
        let ms = millis(started.elapsed());
        if let Err(why) = &jpeg {
            tracing::warn!(session = job.session, why = %why, "a plug-in editor's picture could not be encoded");
        }
        shared.lock().done.push(Encoded {
            session: job.session,
            ms,
            bytes: jpeg.as_ref().ok().map(Vec::len),
        });
        if let Ok(jpeg) = jpeg {
            (job.sink)(job.session, Bytes::from(jpeg));
        }
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

    /// A picture of `session` still waiting is dropped (its frames stop).
    fn forget(&self, session: u32) {
        let mut handoff = self.shared.lock();
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
    /// is joined.
    fn stop(&mut self) {
        {
            let mut handoff = self.shared.lock();
            handoff.stopped = true;
            handoff.waiting = None;
        }
        self.shared.ready.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// An open waiting for its window.
struct Finding {
    session: u32,
    before: Vec<WindowId>,
    started: f64,
    polled: Option<f64>,
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
/// waiting for their windows, the contact on the screen, the encoder and
/// its clock.
struct Worker {
    backend: Box<dyn Backend>,
    events: Events,
    editors: BTreeMap<u32, Editor>,
    finding: Vec<Finding>,
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
                    reply,
                });
            }
            Command::Capture { session, sink } => {
                if let Some(editor) = self.editors.get_mut(&session) {
                    editor.sink = Some(sink);
                }
            }
            Command::Touch {
                session,
                contact,
                phase,
                at,
            } => self.touch(session, contact, phase, at),
            Command::Guard { session, reply } => {
                let _ = reply.send(self.guard(session));
            }
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
    /// contact's keep-alive, the opens' polls, the captures and their
    /// minutes. The keep-alive is looked at first, after the polls (a list
    /// of the windows, a take) and after each capture (a grab blocks for
    /// its time): it never waits for a whole step.
    fn step(&mut self) {
        self.count_encoded();
        self.keep_alive();
        self.find();
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

    /// The opens whose poll is due look for their window.
    fn find(&mut self) {
        let now = self.now();
        for mut finding in std::mem::take(&mut self.finding) {
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
                Some(Ok(window)) => {
                    let answer = self.backend.take(window).map(|taken| {
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
                            },
                        );
                        size
                    });
                    let _ = finding.reply.send(answer);
                }
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
    /// of a window already lost fails and ends no contact), then no more
    /// frames, its contact ended (another session's cancelled), a tap on the
    /// inert spot.
    fn guard(&mut self, session: u32) -> Result<(), String> {
        let editor = self.editors.get_mut(&session).ok_or(NO_EDITOR)?;
        editor.sink = None;
        let width = editor.last.as_ref().map_or(editor.taken.width, |p| p.width);
        let taken = editor.taken.clone();
        self.encoder.forget(session);
        self.end_contact(session);
        self.cancel_other(session);
        guard_tap(self.backend.as_mut(), &taken, inert_spot(width))
    }

    /// `session`'s window handed back: its contact ended, its z-order as it
    /// was; `closed`: Live closed the editor.
    fn release(&mut self, session: u32, closed: bool) {
        self.end_contact(session);
        self.encoder.forget(session);
        if let Some(editor) = self.editors.remove(&session) {
            self.backend.release(&editor.taken);
            if closed {
                self.backend.live_closed(&editor.taken);
            }
        }
    }

    /// The worker ends: every window handed back (the editors stay open in
    /// Live), and its encoder with it.
    fn shutdown(&mut self) {
        let sessions: Vec<u32> = self.editors.keys().copied().collect();
        for session in sessions {
            self.release(session, false);
        }
        self.encoder.stop();
    }
}

/// A tap at `spot` (the guard's): down, a short hold, up. A refused down
/// taps nothing.
pub fn guard_tap(backend: &mut dyn Backend, taken: &Taken, spot: (i32, i32)) -> Result<(), String> {
    backend.touch(taken, Phase::Down, spot)?;
    std::thread::sleep(GUARD_TAP);
    backend.touch(taken, Phase::Up, spot)
}

/// The worker's thread, until a stop's wait joined it.
type Thread = Mutex<Option<std::thread::JoinHandle<()>>>;

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
            contact: None,
            encoder: Encoder::spawn()?,
            clock: Box::new(move || millis(started.elapsed())),
        };
        let thread = std::thread::Builder::new()
            .name("plugwin".to_string())
            .spawn(move || worker.run(&rx))?;
        Ok(Self {
            tx,
            thread: Arc::new(Mutex::new(Some(thread))),
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

    /// A phase of contact `contact` on `session`'s window.
    pub fn touch(&self, session: u32, contact: u32, phase: Phase, at: (i32, i32)) {
        let _ = self.tx.send(Command::Touch {
            session,
            contact,
            phase,
            at,
        });
    }

    /// The close guard of `session`.
    pub async fn guard(&self, session: u32) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Guard { session, reply })
            .map_err(|_| STOPPED.to_string())?;
        answer.await.map_err(|_| STOPPED.to_string())?
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
    /// Once it did, a later call answers at once.
    pub async fn stopped(&self) -> bool {
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(thread) = thread else {
            return true;
        };
        for _ in 0..STOP_POLLS {
            if thread.is_finished() {
                let _ = thread.join();
                return true;
            }
            tokio::time::sleep(STOP_POLL).await;
        }
        *self.thread.lock().unwrap_or_else(PoisonError::into_inner) = Some(thread);
        false
    }
}

#[cfg(test)]
mod tests;
