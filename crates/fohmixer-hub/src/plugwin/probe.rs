//! `fohmixer-hub eq-probe` (#71 PR E, D17): the window backend driven
//! against a window of one process, the isolated check on the PC against
//! Carla's bridge (`.claude/rules/plugin-window.md`) before the first open
//! in a running Live. It runs the steps the hub runs, in order, and prints
//! one `name=value` line per finding (never a window title: a Live window's
//! holds a track name):
//!
//! 1. the process's visible top-level windows; the first one is taken (on
//!    top, its picture found; the probe waits for the posted z-order change
//!    to land, as the hub's take does, at most `FIND_MS`): `picture`,
//!    `child` (Pro-Q's own child found);
//! 2. with `--min-probe` (PR G): a picture of [`MIN_PROBE`] asked, far below
//!    Pro-Q's own minimum, and the size the editor took after
//!    [`super::RESIZE_MS`] (`min=`: Pro-Q's minimum), then the take's
//!    rectangle back (`min_restore=`; the size stays changed until a
//!    restore landed, #74 review);
//! 3. with `--size WxH` (PR G): the picture resized to it as the hub does
//!    (posted, its size read until it lands, at most `RESIZE_MS`; one that
//!    does not land fails the probe): `resize=ok client= ms=`; the gestures
//!    below then run at the new size, `--band` and `--to` scaled by it
//!    ([`scaled`]);
//! 4. `--frames` captures at the hub's rate: their sizes, how many differed,
//!    the grab and encode times, the frame sizes, the rate reached;
//! 5. a double tap at `--band` (on a fresh Pro-Q's curve: a new band);
//! 6. a drag from `--band` to `--to` (updates every 16 ms, a rest with the
//!    resends, the up at the last point);
//! 7. with `--hold` (#71 PR N): what Windows makes of a contact on the
//!    band at `--to` (the first PR M run found a still contact's up made a
//!    right click: Pro-Q's context menu). A still contact held each of
//!    [`HOLDS`] in turn, lifted, then a look for a new window of the
//!    editor's process within [`HOLD_SHOW`] (the menu): `hold=` each hold
//!    and `no` or `menu`, up to the first `menu`, which the inert spot's tap
//!    closes (`menu_closed=ok`; the picture at its known size only); the
//!    drag threshold, a down, one update [`DRAG_PXS`] px up, a rest, the up,
//!    and whether the picture changed (`drag_px=`, the band then dragged
//!    back); a contact that dragged 32 px then held still [`HOLD_LONG`]
//!    (`hold_after_drag=`); and a still contact held [`HOLD_LONG`] with
//!    press-and-hold off for the window (`hold_quiet=`,
//!    [`Backend::press_and_hold`], on again after). Every two contacts are
//!    [`TAP_APART`] apart (never a double click). The picture after each
//!    hold is saved (`hold-<ms>.jpg`, `hold-dragged.jpg`,
//!    `hold-quiet.jpg`: a menu over it shows); `hold_windows=` the count
//!    the menu is looked for beyond, `band_back=same|moved` the picture
//!    after the band's way back against the one before the threshold's
//!    moves, `drag_px=menu` when a threshold move's rest opened a menu,
//!    `hold_quiet=refused:<why>` when Windows refused the property. Only at
//!    the editor's known size (no `--size`);
//! 8. with `--paint <n>` (#71 PR M): how long Pro-Q takes to paint an
//!    input. A contact down on the band at `--to`, dragged past the drag
//!    threshold and back ([`PAINT_ENTER`]: a contact that never left it is
//!    no drag, PR N), then `n` trials. Each
//!    trial rests [`PAINT_REST`], then waits for a still picture (two
//!    grabs [`PAINT_GAP`] apart equal; none within [`PAINT_WAIT`]: the
//!    trial is `unsettled`), injects one update ([`paint_point`]: 8 px up
//!    and back in turn) and grabs every [`PAINT_POLL`] until the picture
//!    differs (none within [`PAINT_WAIT`]: `lost`): the time from the
//!    update to the start of the first grab that differs. The point is
//!    resent once `KEEPALIVE_MS` passed, checked around every sleep and
//!    grab (Windows cancels a contact silent for 100 ms). Output:
//!    `paint=ok n= lost= unsettled=`, `paint_ms_min= paint_ms_p50=
//!    paint_ms_p95= paint_ms_max= grab_ms_mean=` (`-` with no sample; the
//!    grabs' mean is the measure's resolution); then the contact back at
//!    `--to` and its up. Its figure sets `fohmixer_proto::eq::PAINT_MS`;
//!    Carla's bridge runs on the dummy engine, so no analyzer moves the
//!    picture meanwhile;
//! 9. a double tap at `--to` (on the band: its frequency text field opens,
//!    Pro-Q 4.02's close-crash state);
//! 10. the close guard, as the hub's (#74 review, round 2): the picture put
//!     at [`super::KNOWN_SIZE`] (the size the inert spot was verified at),
//!     whatever size it has, in two placements each seen to land (the
//!     window's rectangle for it at the take's place, moved into the work
//!     area when it does not fit there, a pixel aside first, read back until
//!     the window is there; then that rectangle, read back until the window
//!     is there with its picture at exactly that size: `restore=ok client=
//!     ms=`; a placement not seen in time: the probe fails and taps
//!     nothing), then a tap on the inert spot (`--inert`, else the hub's
//!     own, [`super::inert_spot`] of the known width), then the guard's
//!     wait;
//! 11. the release (the take's rectangle back when it was changed, the
//!     z-order as it was) and, with `--close`, a close request to the window
//!     (Carla's bridge ends with `0xC0000005` on it whatever the plug-in's
//!     state).
//!
//! Once the window is taken, a step that fails still ends a contact it left
//! down (a cancel at its last point), posts a changed window's own
//! rectangle back (not waited for) and hands the window back.
//!
//! With `--out` every picture is saved there as a JPEG (`frame-NN.jpg`,
//! then `after-band.jpg`, `after-drag.jpg`, `field-open.jpg`,
//! `after-guard.jpg`; with `--size` first `after-resize.jpg`, with
//! `--hold` the holds' and `after-hold.jpg` after `after-drag.jpg`, with
//! `--paint` `after-paint.jpg` after those): the field
//! must be open in `field-open` and closed in `after-guard`. `--sim` runs
//! it on the simulated backend (the CLI's tests). Exit 0, 1 when a step
//! failed (its line says which), 2 for bad arguments.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::sim::Sim;
#[cfg(windows)]
use super::win::probe_backend as platform;
use super::{
    Backend, GUARD_TAP, KNOWN_SIZE, NOT_ON_TOP, Phase, Pixels, Placement, QUALITY, Rect, STEP,
    Taken, aside, encode, inert_spot, inside, keepalive_due, known_rect, millis, not_landed,
    placed, resize_over, settled, window_size,
};

/// The CLI's usage line.
pub const USAGE: &str = "usage: fohmixer-hub eq-probe --pid <pid> [--frames <n>] [--size <w>x<h>] [--min-probe] [--hold] [--paint <n>] [--band <x,y>] [--to <x,y>] [--inert <x,y>] [--out <folder>] [--close] [--sim]";
/// The picture `--min-probe` asks for (px): far below any Pro-Q size, so
/// the editor takes its own minimum.
pub const MIN_PROBE: (u32, u32) = (160, 120);
/// The looks at a resized picture's size, or at a guard's placement,
/// [`STEP`] apart: past `RESIZE_MS` (1 s) the last one settles it
/// ([`settled`], [`resize_over`]).
pub const RESIZE_LOOKS: u32 = 100;
/// Captures when `--frames` is not given.
pub const FRAMES: u32 = 50;
/// A point on a fresh Pro-Q 4's curve at 100 % (its 0 dB line near
/// 130 Hz): the band the probe makes.
pub const BAND: (i32, i32) = (431, 321);
/// Where the drag takes it (about +3 dB near 200 Hz).
pub const TO: (i32, i32) = (511, 281);
/// The drag's updates.
pub const DRAG_STEPS: i32 = 25;
/// Between two updates of the drag.
pub const DRAG_STEP: Duration = Duration::from_millis(16);
/// A finger resting at the drag's end: its point re-sent this many times,
/// [`RESEND`] apart, as the hub's window worker keeps a resting contact
/// alive (`super::KEEPALIVE_MS`; Windows cancels a contact silent for
/// 100 ms): a rest of 300 ms.
pub const RESENDS: u32 = 6;
pub const RESEND: Duration = Duration::from_millis(50);
/// A tap's finger down, and the gap between a double tap's two taps.
pub const TAP: Duration = Duration::from_millis(60);
pub const TAP_GAP: Duration = Duration::from_millis(120);
/// The wait for the editor to draw what a gesture did.
pub const SETTLE: Duration = Duration::from_millis(400);
/// The looks at the taken window's place on top, [`STEP`] apart: past
/// `FIND_MS` (3 s) the last one fails ([`placed`]).
pub const TOP_LOOKS: u32 = 300;
/// `--paint` (#71 PR M): between two grabs of a trial (a grab that takes
/// longer is followed by the next at once).
pub const PAINT_POLL: Duration = Duration::from_millis(5);
/// A trial whose picture did not change within this is lost.
pub const PAINT_WAIT: Duration = Duration::from_millis(500);
/// How far an update moves the band (px up, then back).
pub const PAINT_STEP: i32 = 8;
/// The rest before each trial (the contact kept alive), so the picture
/// before it shows the last update.
pub const PAINT_REST: Duration = Duration::from_millis(150);
/// Two grabs this far apart that are equal: the picture before a trial
/// is still (no animation or late paint left, the PR M review).
pub const PAINT_GAP: Duration = Duration::from_millis(20);
/// How far the paint contact goes up and back before its trials (px, PR
/// N): past the touch drag threshold (as `--hold`'s `drag_px=` measures
/// it; a threshold beyond this loses every trial), so Windows makes it a
/// drag, in
/// [`PAINT_ENTER_STEPS`] updates each way, [`DRAG_STEP`] apart (never
/// through a trial's point).
pub const PAINT_ENTER: i32 = 24;
pub const PAINT_ENTER_STEPS: i32 = 4;
/// `--hold` (#71 PR N): the still holds tried in turn (ms), up to the
/// first that opens a menu.
pub const HOLDS: [u64; 5] = [400, 800, 1200, 1600, 2400];
/// The hold after a drag, and with press-and-hold off (ms).
pub const HOLD_LONG: u64 = 2400;
/// How far the contact that drags first goes (px).
pub const HOLD_DRAG: i32 = 32;
/// How long after a contact's up a menu may take to show.
pub const HOLD_SHOW: Duration = Duration::from_millis(500);
/// Between two contacts: longer than Windows' double-click time (500 ms
/// by default), so two never make a double click (a band's text field).
pub const TAP_APART: Duration = Duration::from_millis(700);
/// The single moves tried for the drag threshold (px up), and the rest
/// after each before its up.
pub const DRAG_PXS: [i32; 8] = [2, 4, 6, 8, 12, 16, 24, 32];
pub const DRAG_REST: Duration = Duration::from_millis(300);

/// The probe's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub pid: u32,
    pub frames: u32,
    pub band: (i32, i32),
    pub to: (i32, i32),
    pub inert: Option<(i32, i32)>,
    pub out: Option<PathBuf>,
    pub close: bool,
    pub sim: bool,
    /// The picture size to resize to (PR G).
    pub size: Option<(u32, u32)>,
    /// Whether to read Pro-Q's own minimum size (PR G).
    pub min_probe: bool,
    /// The paint trials (#71 PR M; 0: none).
    pub paint: u32,
    /// Whether to see what Windows makes of a held contact (#71 PR N).
    pub hold: bool,
}

/// `x,y` as a point.
fn point(text: &str) -> Option<(i32, i32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// `WxH` as a size, each side at least 1 px.
fn size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('x')?;
    let side = |v: &str| v.trim().parse::<u32>().ok().filter(|v| *v > 0);
    Some((side(w)?, side(h)?))
}

/// A point of a picture of `from` at the same place of a picture of `to`
/// (PR G: the gestures at a resized picture), rounded.
pub fn scaled(at: (i32, i32), from: (u32, u32), to: (u32, u32)) -> (i32, i32) {
    let side = |v: i32, from: u32, to: u32| {
        (f64::from(v) * f64::from(to) / f64::from(from.max(1))).round() as i32
    };
    (side(at.0, from.0, to.0), side(at.1, from.1, to.1))
}

/// The arguments after `eq-probe`.
pub fn parse(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args {
        pid: 0,
        frames: FRAMES,
        band: BAND,
        to: TO,
        inert: None,
        out: None,
        close: false,
        sim: false,
        size: None,
        min_probe: false,
        paint: 0,
        hold: false,
    };
    let mut pid = None;
    let mut rest = args.iter();
    // Each pass takes at least one argument: the list's length bounds it.
    for _ in 0..=args.len() {
        let Some(flag) = rest.next() else {
            break;
        };
        match *flag {
            "--close" => parsed.close = true,
            "--sim" => parsed.sim = true,
            "--min-probe" => parsed.min_probe = true,
            "--hold" => parsed.hold = true,
            "--pid" | "--frames" | "--band" | "--to" | "--inert" | "--out" | "--size"
            | "--paint" => {
                let value = rest.next().ok_or(format!("{flag} needs a value"))?;
                let bad = || format!("{flag} {value:?}");
                match *flag {
                    "--pid" => pid = Some(value.parse::<u32>().map_err(|_| bad())?),
                    "--frames" => parsed.frames = value.parse().map_err(|_| bad())?,
                    "--paint" => parsed.paint = value.parse().map_err(|_| bad())?,
                    "--band" => parsed.band = point(value).ok_or_else(bad)?,
                    "--to" => parsed.to = point(value).ok_or_else(bad)?,
                    "--inert" => parsed.inert = Some(point(value).ok_or_else(bad)?),
                    "--size" => parsed.size = Some(size(value).ok_or_else(bad)?),
                    _ => parsed.out = Some(PathBuf::from(*value)),
                }
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    parsed.pid = pid.ok_or("--pid is required")?;
    Ok(parsed)
}

/// The backend the arguments ask for: the simulated one (`--sim`, with one
/// window of the process), else the platform's.
pub fn backend(args: &Args) -> Result<Box<dyn Backend>, String> {
    if args.sim {
        let (sim, handle) = Sim::new(None);
        handle.auto_open(false);
        handle.add_window(args.pid);
        return Ok(Box::new(sim));
    }
    platform()
}

#[cfg(not(windows))]
fn platform() -> Result<Box<dyn Backend>, String> {
    Err("eq-probe drives Windows windows: run it on the PC (or with --sim)".to_string())
}

/// The mean and the largest of `values` (0 for none).
pub fn stats(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let sum: f64 = values.iter().sum();
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (sum / values.len() as f64, max)
}

/// The captures' period: the hub's ([`super::CAPTURE_MS`]).
pub const PERIOD: Duration = Duration::from_millis(40);

/// When capture `n` is due after the first.
pub fn due_at(n: u32) -> Duration {
    PERIOD * n
}

/// Frames a second: `frames` in `seconds` (0 for no time).
pub fn rate(frames: u32, seconds: f64) -> f64 {
    if seconds > 0.0 {
        f64::from(frames) / seconds
    } else {
        0.0
    }
}

/// The point of paint trial `n` (from 0) on a band at `at`: [`PAINT_STEP`]
/// px above it, then back at it, in turn (each update moves the curve).
pub fn paint_point(at: (i32, i32), n: u32) -> (i32, i32) {
    if n.is_multiple_of(2) {
        (at.0, at.1 - PAINT_STEP)
    } else {
        at
    }
}

/// The points of the paint contact's way past the drag threshold and back
/// (PR N): [`PAINT_ENTER`] px up from `at` in [`PAINT_ENTER_STEPS`]
/// updates, then back to `at`.
pub fn enter_points(at: (i32, i32)) -> Vec<(i32, i32)> {
    let step = PAINT_ENTER / PAINT_ENTER_STEPS;
    let up = (1..=PAINT_ENTER_STEPS).map(|i| (at.0, at.1 - step * i));
    let back = (0..PAINT_ENTER_STEPS)
        .rev()
        .map(|i| (at.0, at.1 - step * i));
    up.chain(back).collect()
}

/// `--hold`'s line of holds tried: `<ms>:no` or `<ms>:menu` each.
pub fn hold_text(holds: &[(u64, bool)]) -> String {
    holds
        .iter()
        .map(|(ms, menu)| format!("{ms}:{}", if *menu { "menu" } else { "no" }))
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether a wait `since_ms` along has lasted `long` (PR N: every wait of
/// `--hold`).
pub fn over(since_ms: f64, long: Duration) -> bool {
    since_ms >= millis(long)
}

/// Whether a paint trial `since_ms` after its update is over
/// ([`PAINT_WAIT`]).
pub fn paint_over(since_ms: f64) -> bool {
    since_ms >= millis(PAINT_WAIT)
}

/// Whether a contact `since_ms` into its rest before a trial rested
/// [`PAINT_REST`].
pub fn rested(since_ms: f64) -> bool {
    since_ms >= millis(PAINT_REST)
}

/// The value at quantile `q` (0 to 1) of `values`, by nearest rank (none
/// for no value).
pub fn percentile(values: &[f64], q: f64) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted
        .get(rank.saturating_sub(1).min(sorted.len().saturating_sub(1)))
        .copied()
}

/// A paint figure: ms to a tenth, `-` for none (no sample: never 0).
pub fn ms_text(ms: Option<f64>) -> String {
    ms.map_or_else(|| "-".to_string(), |ms| format!("{ms:.1}"))
}

/// The points of a drag from `from` to `to` in `steps` updates, the last
/// one exactly at `to`.
pub fn drag_points(from: (i32, i32), to: (i32, i32), steps: i32) -> Vec<(i32, i32)> {
    let at = |a: i32, b: i32, i: i32| a + (b - a) * i / steps.max(1);
    (1..=steps.max(1))
        .map(|i| (at(from.0, to.0, i), at(from.1, to.1, i)))
        .collect()
}

/// What the probe found, line by line, and where its pictures go.
struct Report<'a> {
    out: &'a mut dyn Write,
    folder: Option<PathBuf>,
}

impl Report<'_> {
    fn line(&mut self, text: &str) -> Result<(), String> {
        writeln!(self.out, "{text}").map_err(|e| format!("stdout: {e}"))
    }

    /// Saves a picture as `name` in the folder, if there is one.
    fn save(&mut self, name: &str, pixels: &Pixels) -> Result<(), String> {
        let Some(folder) = &self.folder else {
            return Ok(());
        };
        let jpeg = encode(pixels, QUALITY)?;
        std::fs::write(folder.join(name), jpeg).map_err(|e| format!("saving {name}: {e}"))
    }
}

/// The probe's hold on the window it took: every touch goes through it, so
/// a step that fails, on any path out, leaves no contact down and no window
/// on top. Dropped, it cancels a contact still down at its last point,
/// posts a changed window's own rectangle back and hands the window back
/// (its z-order as it was), unless the release ran.
struct Held<'a> {
    backend: &'a mut dyn Backend,
    taken: Taken,
    /// The contact down, at its last point.
    contact: Option<(i32, i32)>,
    released: bool,
    /// Whether the window's rectangle was changed and is not the take's
    /// again (PR G): set at each post, cleared only once one of the take's
    /// rectangle landed (#74 review).
    changed: bool,
}

impl Held<'_> {
    /// A gesture's touch, its failure named by `step`.
    fn touch(&mut self, phase: Phase, at: (i32, i32), step: &str) -> Result<(), String> {
        self.backend
            .touch(&self.taken, phase, at)
            .map_err(|why| format!("{step}: {} at {},{}: {why}", phase.name(), at.0, at.1))?;
        self.contact = (!phase.ends()).then_some(at);
        Ok(())
    }

    /// The guard's tap at `spot`: down, a short hold, up.
    fn guard_tap(&mut self, spot: (i32, i32)) -> Result<(), String> {
        self.backend.touch(&self.taken, Phase::Down, spot)?;
        self.contact = Some(spot);
        std::thread::sleep(GUARD_TAP);
        self.backend.touch(&self.taken, Phase::Up, spot)?;
        self.contact = None;
        Ok(())
    }

    /// The picture now.
    fn grab(&mut self) -> Result<Pixels, String> {
        self.backend.grab(&self.taken)
    }

    /// The picture's own size (the take's).
    fn original(&self) -> (u32, u32) {
        (self.taken.width, self.taken.height)
    }

    /// The picture resized to `asked` as the hub does (PR G): the window's
    /// rectangle for it where it stands, else moved into the work area
    /// (#74 review, [`inside`]), posted, its size read every [`STEP`] until
    /// it lands or `RESIZE_MS` passed. Its size then and the ms it took;
    /// `Err` with them when it did not land.
    fn resize(&mut self, asked: (u32, u32)) -> Result<Settle, Settle> {
        let unread = |why: String| Settle {
            client: (0, 0),
            ms: 0.0,
            why: Some(why),
        };
        let work = self.backend.work(&self.taken).map_err(unread)?;
        let now = self.backend.rect(&self.taken).map_err(unread)?;
        let rect = inside(work, now, window_size(&self.taken, asked));
        self.post(rect, asked)
    }

    /// The window's own rectangle back (the take's: its place and its
    /// size), as [`Held::resize`] waits for it.
    fn restore(&mut self) -> Result<Settle, Settle> {
        let (rect, original) = (self.taken.rect, self.original());
        self.post(rect, original)
    }

    /// The picture at [`KNOWN_SIZE`], the guard's, as the hub's (#74
    /// review, round 2): the window's rectangle for it at the take's place,
    /// moved into the work area when it does not fit there ([`known_rect`]),
    /// a pixel aside first ([`aside`]: read back, it proves that every
    /// rectangle posted before it landed), then that rectangle itself, read
    /// back with the picture at exactly that size ([`Held::place`]). The
    /// picture's size and the ms since the first post; else why not.
    fn known(&mut self) -> Result<Settle, String> {
        let started = Instant::now();
        let work = self.backend.work(&self.taken)?;
        let now = self.backend.rect(&self.taken)?;
        let known = known_rect(work, &self.taken);
        self.place(Placement::Aside(aside(known, now, work)), started)?;
        self.place(Placement::Known(known), started)
    }

    /// `placement` posted to the window, and the window read every [`STEP`]
    /// until it landed ([`Placement::landed`]) or `RESIZE_MS` passed since
    /// the post: the picture's size then and the ms since `started`; else
    /// what the window read last ([`not_landed`]). The window counts as
    /// changed from the post until one of the take's rectangle landed.
    fn place(&mut self, placement: Placement, started: Instant) -> Result<Settle, String> {
        let rect = placement.rect();
        self.backend.resize(&self.taken, rect)?;
        self.changed = true;
        let posted = Instant::now();
        let (mut window, mut client) = (None, None);
        for _ in 0..=RESIZE_LOOKS {
            window = self.backend.rect(&self.taken).ok();
            client = self.backend.client(&self.taken).ok();
            if placement.landed(window, client) {
                self.changed = rect != self.taken.rect;
                return Ok(Settle {
                    client: client.unwrap_or((0, 0)),
                    ms: millis(started.elapsed()),
                    why: None,
                });
            }
            if resize_over(millis(posted.elapsed())) {
                break;
            }
            std::thread::sleep(STEP);
        }
        Err(not_landed(placement, window, client))
    }

    /// `rect` posted to the window and its picture read every [`STEP`] until
    /// it is `asked` within the slack ([`settled`]) or `RESIZE_MS` passed:
    /// the last look's size and time. The window counts as changed from the
    /// post until one of the take's rectangle landed.
    fn post(&mut self, rect: Rect, asked: (u32, u32)) -> Result<Settle, Settle> {
        self.backend
            .resize(&self.taken, rect)
            .map_err(|why| Settle {
                client: (0, 0),
                ms: 0.0,
                why: Some(why),
            })?;
        self.changed = true;
        let started = Instant::now();
        let mut seen = Settle {
            client: (0, 0),
            ms: 0.0,
            why: None,
        };
        for _ in 0..=RESIZE_LOOKS {
            let ms = millis(started.elapsed());
            let client = self.backend.client(&self.taken).unwrap_or((0, 0));
            seen = Settle {
                client,
                ms,
                why: None,
            };
            // Strict: the probe reports a size the editor would not take
            // (`--min-probe`), so only `asked` lands here.
            match settled(client, asked, client, ms) {
                Some(true) => {
                    self.changed = rect != self.taken.rect;
                    return Ok(seen);
                }
                Some(false) => return Err(seen),
                None => std::thread::sleep(STEP),
            }
        }
        Err(seen)
    }

    /// The window handed back: the take's rectangle again when it was
    /// changed (not waited for), its z-order as it was.
    fn hand_back(&mut self) {
        if self.changed {
            let _ = self.backend.resize(&self.taken, self.taken.rect);
        }
        self.backend.release(&self.taken);
    }

    /// The window handed back ([`Held::hand_back`]) at the probe's end.
    fn release(&mut self) {
        self.released = true;
        self.hand_back();
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(at) = self.contact.take() {
            let _ = self.backend.touch(&self.taken, Phase::Cancel, at);
        }
        if !self.released {
            self.hand_back();
        }
    }
}

/// A resize as the probe saw it: the picture's size then, the ms it took,
/// and why it could not be posted (none: it was).
#[derive(Debug, Clone, PartialEq)]
struct Settle {
    client: (u32, u32),
    ms: f64,
    why: Option<String>,
}

impl Settle {
    /// Its line's figures: `client=WxH ms=N`.
    fn figures(&self) -> String {
        format!(
            "client={}x{} ms={:.0}",
            self.client.0, self.client.1, self.ms
        )
    }

    /// Why the resize to `asked` failed, for `step`.
    fn failure(&self, step: &str, asked: (u32, u32)) -> String {
        match &self.why {
            Some(why) => format!("{step}: {}x{} could not be posted: {why}", asked.0, asked.1),
            None => format!(
                "{step}: {}x{} asked, the picture is {}x{} after {:.0} ms",
                asked.0, asked.1, self.client.0, self.client.1, self.ms
            ),
        }
    }
}

/// Waits for the taken window to be on top (the take posts its z-order
/// change, which the window's thread lands later; the hub's take waits the
/// same way): looked at every [`STEP`], at most `FIND_MS`.
fn on_top(held: &mut Held<'_>) -> Result<(), String> {
    let started = Instant::now();
    for _ in 0..=TOP_LOOKS {
        match placed(held.backend.on_top(&held.taken), millis(started.elapsed())) {
            Some(answer) => return answer.map_err(str::to_string),
            None => std::thread::sleep(STEP),
        }
    }
    Err(NOT_ON_TOP.to_string())
}

/// A double tap at `at`.
fn double_tap(held: &mut Held<'_>, at: (i32, i32), step: &str) -> Result<(), String> {
    for gap in [TAP_GAP, Duration::ZERO] {
        held.touch(Phase::Down, at, step)?;
        std::thread::sleep(TAP);
        held.touch(Phase::Up, at, step)?;
        std::thread::sleep(gap);
    }
    Ok(())
}

/// A drag from `from` to `to`: the down, the updates, the rest with its
/// resends, the up at the last point.
fn drag(held: &mut Held<'_>, from: (i32, i32), to: (i32, i32)) -> Result<usize, String> {
    held.touch(Phase::Down, from, "drag")?;
    let points = drag_points(from, to, DRAG_STEPS);
    for at in &points {
        std::thread::sleep(DRAG_STEP);
        held.touch(Phase::Update, *at, "drag")?;
    }
    for _ in 0..RESENDS {
        std::thread::sleep(RESEND);
        held.touch(Phase::Update, to, "drag")?;
    }
    held.touch(Phase::Up, to, "drag")?;
    Ok(points.len())
}

/// Grabs `frames` pictures at the hub's rate and reports them.
fn capture(held: &mut Held<'_>, frames: u32, report: &mut Report<'_>) -> Result<(), String> {
    let (mut grabs, mut encodes, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
    let mut last: Option<Pixels> = None;
    let mut same = 0_u32;
    let started = Instant::now();
    for n in 0..frames {
        std::thread::sleep(due_at(n).saturating_sub(started.elapsed()));
        let at = Instant::now();
        let pixels = held.grab().map_err(|why| format!("capture: {why}"))?;
        grabs.push(millis(at.elapsed()));
        if last.as_ref() == Some(&pixels) {
            same += 1;
            continue;
        }
        let at = Instant::now();
        let jpeg = encode(&pixels, QUALITY)?;
        encodes.push(millis(at.elapsed()));
        sizes.push(jpeg.len() as f64);
        report.save(&format!("frame-{n:02}.jpg"), &pixels)?;
        last = Some(pixels);
    }
    let seconds = started.elapsed().as_secs_f64();
    let (grab_mean, grab_max) = stats(&grabs);
    let (encode_mean, encode_max) = stats(&encodes);
    let (bytes_mean, _) = stats(&sizes);
    report.line(&format!("frames={frames} sent={} same={same}", sizes.len()))?;
    report.line(&format!(
        "grab_ms_mean={grab_mean:.1} grab_ms_max={grab_max:.1}"
    ))?;
    report.line(&format!(
        "encode_ms_mean={encode_mean:.1} encode_ms_max={encode_max:.1} bytes_mean={bytes_mean:.0}"
    ))?;
    report.line(&format!("fps={:.1}", rate(frames, seconds)))?;
    Ok(())
}

/// The contact at `at` resent when `KEEPALIVE_MS` passed since `sent`,
/// its last injection (a silent contact is cancelled after 100 ms).
fn keep(held: &mut Held<'_>, at: (i32, i32), sent: &mut Instant) -> Result<(), String> {
    if keepalive_due(millis(sent.elapsed())) {
        held.touch(Phase::Update, at, "paint")?;
        *sent = Instant::now();
    }
    Ok(())
}

/// A sleep of `long` with the contact at `at` kept alive.
fn sleep_kept(
    held: &mut Held<'_>,
    at: (i32, i32),
    long: Duration,
    sent: &mut Instant,
) -> Result<(), String> {
    keep(held, at, sent)?;
    std::thread::sleep(long);
    keep(held, at, sent)
}

/// The picture with the contact at `at` kept alive.
fn grab_kept(held: &mut Held<'_>, at: (i32, i32), sent: &mut Instant) -> Result<Pixels, String> {
    let pixels = held.grab().map_err(|why| format!("paint: {why}"))?;
    keep(held, at, sent)?;
    Ok(pixels)
}

/// The contact at `at` kept alive for [`PAINT_REST`], then the picture
/// once two grabs [`PAINT_GAP`] apart are equal; none within
/// [`PAINT_WAIT`] (the picture never came to rest).
fn still_picture(
    held: &mut Held<'_>,
    at: (i32, i32),
    sent: &mut Instant,
) -> Result<Option<Pixels>, String> {
    let started = Instant::now();
    while !rested(millis(started.elapsed())) {
        sleep_kept(held, at, PAINT_POLL, sent)?;
    }
    let waited = Instant::now();
    let mut last = grab_kept(held, at, sent)?;
    while !paint_over(millis(waited.elapsed())) {
        sleep_kept(held, at, PAINT_GAP, sent)?;
        let now = grab_kept(held, at, sent)?;
        if now == last {
            return Ok(Some(now));
        }
        last = now;
    }
    Ok(None)
}

/// One paint trial: the update to `at` (the contact down), then the
/// picture grabbed every [`PAINT_POLL`] until it differs from `before`:
/// the ms from the update to the start of that grab; none within
/// [`PAINT_WAIT`]. Each grab's own ms goes to `grabs`.
fn paint_trial(
    held: &mut Held<'_>,
    at: (i32, i32),
    before: &Pixels,
    sent: &mut Instant,
    grabs: &mut Vec<f64>,
) -> Result<Option<f64>, String> {
    held.touch(Phase::Update, at, "paint")?;
    let injected = Instant::now();
    *sent = injected;
    while !paint_over(millis(injected.elapsed())) {
        let since = millis(injected.elapsed());
        let grabbed = Instant::now();
        let pixels = grab_kept(held, at, sent)?;
        grabs.push(millis(grabbed.elapsed()));
        if pixels != *before {
            return Ok(Some(since));
        }
        sleep_kept(held, at, PAINT_POLL.saturating_sub(grabbed.elapsed()), sent)?;
    }
    Ok(None)
}

/// `--paint`: `trials` paint trials on the band at `at` and their figures;
/// the contact ends back at `at`.
fn paint(
    held: &mut Held<'_>,
    at: (i32, i32),
    trials: u32,
    report: &mut Report<'_>,
) -> Result<(), String> {
    held.touch(Phase::Down, at, "paint")?;
    // Past the drag threshold and back: only a drag moves the band (PR N).
    for point in enter_points(at) {
        std::thread::sleep(DRAG_STEP);
        held.touch(Phase::Update, point, "paint")?;
    }
    let mut sent = Instant::now();
    let (mut samples, mut grabs) = (Vec::new(), Vec::new());
    let (mut lost, mut unsettled) = (0_u32, 0_u32);
    let mut last = at;
    // The updates injected: each one goes to the other point (a skipped
    // trial injects none, so the next one still moves the band).
    let mut injected = 0_u32;
    for _ in 0..trials {
        let Some(before) = still_picture(held, last, &mut sent)? else {
            unsettled += 1;
            continue;
        };
        last = paint_point(at, injected);
        injected += 1;
        match paint_trial(held, last, &before, &mut sent, &mut grabs)? {
            Some(ms) => samples.push(ms),
            None => lost += 1,
        }
    }
    if last != at {
        held.touch(Phase::Update, at, "paint")?;
    }
    held.touch(Phase::Up, at, "paint")?;
    let grab_mean = (!grabs.is_empty()).then(|| stats(&grabs).0);
    report.line(&format!(
        "paint=ok n={trials} lost={lost} unsettled={unsettled}"
    ))?;
    report.line(&format!(
        "paint_ms_min={} paint_ms_p50={} paint_ms_p95={} paint_ms_max={} grab_ms_mean={}",
        ms_text(percentile(&samples, 0.0)),
        ms_text(percentile(&samples, 0.5)),
        ms_text(percentile(&samples, 0.95)),
        ms_text(percentile(&samples, 1.0)),
        ms_text(grab_mean),
    ))
}

/// The editor's process's visible top-level windows now (a menu of its is
/// one more).
fn windows_now(held: &mut Held<'_>) -> Result<usize, String> {
    let pid = held.taken.pid;
    Ok(held.backend.windows_of(pid)?.len())
}

/// A contact at `at` held `hold_ms`: still, or first dragged [`HOLD_DRAG`]
/// px up and back (`drag`), kept alive throughout, then lifted. Whether a
/// window of the editor's process came beyond `base` within [`HOLD_SHOW`]
/// of the up (Windows made it a right click: a menu); the picture then is
/// saved as `name` (a menu over the picture shows in it).
fn held_contact(
    held: &mut Held<'_>,
    at: (i32, i32),
    hold_ms: u64,
    drag: bool,
    base: usize,
    name: &str,
    report: &mut Report<'_>,
) -> Result<bool, String> {
    held.touch(Phase::Down, at, "hold")?;
    let mut sent = Instant::now();
    let top = (at.0, at.1 - HOLD_DRAG);
    if drag {
        for point in drag_points(at, top, DRAG_STEPS / 4) {
            std::thread::sleep(DRAG_STEP);
            held.touch(Phase::Update, point, "hold")?;
            sent = Instant::now();
        }
    }
    let rest = if drag { top } else { at };
    let started = Instant::now();
    while !over(millis(started.elapsed()), Duration::from_millis(hold_ms)) {
        sleep_kept(held, rest, PAINT_POLL, &mut sent)?;
    }
    if drag {
        for point in drag_points(top, at, DRAG_STEPS / 4) {
            std::thread::sleep(DRAG_STEP);
            held.touch(Phase::Update, point, "hold")?;
        }
    }
    held.touch(Phase::Up, at, "hold")?;
    let lifted = Instant::now();
    let mut menu = false;
    while !over(millis(lifted.elapsed()), HOLD_SHOW) {
        if windows_now(held)? > base {
            menu = true;
            break;
        }
        std::thread::sleep(PAINT_POLL);
    }
    let pixels = held.grab().map_err(|why| format!("hold: {why}"))?;
    report.save(name, &pixels)?;
    Ok(menu)
}

/// A menu opened by a hold closed: a tap on the inert spot `spot` (the
/// picture at the size it is known at; a menu takes that tap and closes),
/// then the editor's process back to `base` windows.
fn close_menu(held: &mut Held<'_>, base: usize, spot: (i32, i32)) -> Result<(), String> {
    std::thread::sleep(TAP_APART);
    held.guard_tap(spot)
        .map_err(|why| format!("hold: closing the menu: {why}"))?;
    let tapped = Instant::now();
    while !over(millis(tapped.elapsed()), HOLD_SHOW) {
        if windows_now(held)? <= base {
            return Ok(());
        }
        std::thread::sleep(PAINT_POLL);
    }
    Err("hold: the menu stayed open".to_string())
}

/// What the drag threshold's moves found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Threshold {
    /// The first move that moved the band, and where the band is then.
    Moved(i32, (i32, i32)),
    /// None of [`DRAG_PXS`] moved it.
    Still,
    /// A move's contact opened a menu (its rest was a hold; the review of
    /// PR N): closed, and nothing is measured.
    Menu,
}

/// The drag threshold: for each of [`DRAG_PXS`] a down at `at`, one update
/// that far up, [`DRAG_REST`], the up, and the picture after it against
/// the one before (the system cursor is back where it was both times).
fn drag_threshold(
    held: &mut Held<'_>,
    at: (i32, i32),
    base: usize,
    spot: (i32, i32),
) -> Result<Threshold, String> {
    for px in DRAG_PXS {
        std::thread::sleep(TAP_APART);
        let before = held.grab().map_err(|why| format!("hold: {why}"))?;
        let to = (at.0, at.1 - px);
        held.touch(Phase::Down, at, "drag threshold")?;
        let mut sent = Instant::now();
        held.touch(Phase::Update, to, "drag threshold")?;
        let started = Instant::now();
        while !over(millis(started.elapsed()), DRAG_REST) {
            sleep_kept(held, to, PAINT_POLL, &mut sent)?;
        }
        held.touch(Phase::Up, to, "drag threshold")?;
        std::thread::sleep(SETTLE);
        if windows_now(held)? > base {
            close_menu(held, base, spot)?;
            return Ok(Threshold::Menu);
        }
        let after = held.grab().map_err(|why| format!("hold: {why}"))?;
        if after != before {
            return Ok(Threshold::Moved(px, to));
        }
    }
    Ok(Threshold::Still)
}

/// The threshold's line: `drag_px=<px>`, `-` (none moved) or `menu`.
fn threshold_text(threshold: Threshold) -> String {
    match threshold {
        Threshold::Moved(px, _) => format!("drag_px={px}"),
        Threshold::Still => "drag_px=-".to_string(),
        Threshold::Menu => "drag_px=menu".to_string(),
    }
}

/// The band dragged from `from` back to `at`.
fn drag_back(held: &mut Held<'_>, from: (i32, i32), at: (i32, i32)) -> Result<(), String> {
    if from == at {
        return Ok(());
    }
    std::thread::sleep(TAP_APART);
    held.touch(Phase::Down, from, "hold")?;
    for point in drag_points(from, at, DRAG_STEPS / 4) {
        std::thread::sleep(DRAG_STEP);
        held.touch(Phase::Update, point, "hold")?;
    }
    held.touch(Phase::Up, at, "hold")
}

/// `--hold` (#71 PR N) on the band at `at`, the inert spot `spot` closing
/// a menu, its figures to `report`. A step that fails (a touch refused)
/// ends the probe there: the window and any contact are handed back, but
/// a menu that step opened stays open (the probe's line says which step).
fn hold(
    held: &mut Held<'_>,
    at: (i32, i32),
    spot: (i32, i32),
    report: &mut Report<'_>,
) -> Result<(), String> {
    let base = windows_now(held)?;
    report.line(&format!("hold_windows={base}"))?;
    let mut tried = Vec::new();
    for ms in HOLDS {
        std::thread::sleep(TAP_APART);
        let menu = held_contact(held, at, ms, false, base, &format!("hold-{ms}.jpg"), report)?;
        tried.push((ms, menu));
        if menu {
            break;
        }
    }
    report.line(&format!("hold={}", hold_text(&tried)))?;
    if tried.iter().any(|(_, menu)| *menu) {
        close_menu(held, base, spot)?;
        report.line("menu_closed=ok")?;
    }
    // The band where it was is the picture before the threshold's moves.
    std::thread::sleep(SETTLE);
    let start = held.grab().map_err(|why| format!("hold: {why}"))?;
    let threshold = drag_threshold(held, at, base, spot)?;
    report.line(&threshold_text(threshold))?;
    if let Threshold::Moved(_, band) = threshold {
        drag_back(held, band, at)?;
    }
    std::thread::sleep(SETTLE);
    let back = held.grab().map_err(|why| format!("hold: {why}"))?;
    report.line(if back == start {
        "band_back=same"
    } else {
        "band_back=moved"
    })?;
    std::thread::sleep(TAP_APART);
    let dragged = held_contact(held, at, HOLD_LONG, true, base, "hold-dragged.jpg", report)?;
    report.line(&format!(
        "hold_after_drag={}",
        hold_text(&[(HOLD_LONG, dragged)])
    ))?;
    if dragged {
        close_menu(held, base, spot)?;
    }
    // Press-and-hold off (a property Windows may refuse to set on another
    // process's window): the same still hold, then on again.
    if let Err(why) = held.backend.press_and_hold(&held.taken, true) {
        let _ = held.backend.press_and_hold(&held.taken, false);
        report.line(&format!("hold_quiet=refused:{why}"))?;
    } else {
        std::thread::sleep(TAP_APART);
        let quiet = held_contact(held, at, HOLD_LONG, false, base, "hold-quiet.jpg", report);
        let on = held.backend.press_and_hold(&held.taken, false);
        let quiet = match (quiet, on) {
            (Ok(quiet), Ok(())) => quiet,
            (Err(why), Ok(())) => return Err(why),
            (Ok(_), Err(on)) => return Err(format!("hold: press-and-hold back on: {on}")),
            (Err(why), Err(on)) => {
                return Err(format!("{why}; hold: press-and-hold back on: {on}"));
            }
        };
        report.line(&format!("hold_quiet={}", hold_text(&[(HOLD_LONG, quiet)])))?;
        if quiet {
            close_menu(held, base, spot)?;
        }
    }
    std::thread::sleep(TAP_APART);
    Ok(())
}

/// After a gesture: the editor's wait to draw it, then its picture saved.
fn settle(held: &mut Held<'_>, name: &str, report: &mut Report<'_>) -> Result<(), String> {
    std::thread::sleep(SETTLE);
    let pixels = held.grab().map_err(|why| format!("{name}: {why}"))?;
    report.save(name, &pixels)
}

/// The probe's steps on `backend` (its thread started), the findings to
/// `out`; the first step that failed, if one did. Once the window is taken,
/// every way out ends a contact still down and hands the window back.
pub fn run(backend: &mut dyn Backend, args: &Args, out: &mut dyn Write) -> Result<(), String> {
    let mut report = Report {
        out,
        folder: args.out.clone(),
    };
    if let Some(folder) = args.out.as_deref() {
        std::fs::create_dir_all(folder).map_err(|e| format!("--out: {e}"))?;
    }
    let windows = backend.windows_of(args.pid)?;
    report.line(&format!("windows={}", windows.len()))?;
    let window = *windows
        .first()
        .ok_or(format!("no visible window of process {}", args.pid))?;
    let taken = backend.take(window)?;
    let mut held = Held {
        backend,
        taken,
        contact: None,
        released: false,
        changed: false,
    };
    on_top(&mut held)?;
    report.line(&format!(
        "picture={}x{} child={} was_topmost={}",
        held.taken.width,
        held.taken.height,
        held.taken.picture != held.taken.window,
        held.taken.was_topmost
    ))?;
    let original = held.original();
    if args.min_probe {
        // Pro-Q's own minimum: the size it takes for a far smaller ask (it
        // does not land, so the probe waits the whole `RESIZE_MS`).
        let (Ok(min) | Err(min)) = held.resize(MIN_PROBE);
        if let Some(why) = &min.why {
            return Err(format!("min-probe: {why}"));
        }
        report.line(&format!(
            "min={}x{} asked={}x{}",
            min.client.0, min.client.1, MIN_PROBE.0, MIN_PROBE.1
        ))?;
        let back = held
            .restore()
            .map_err(|seen| seen.failure("min_restore", original))?;
        report.line(&format!("min_restore=ok {}", back.figures()))?;
    }
    // The gestures' points at the picture's size now.
    let mut now = original;
    if let Some(asked) = args.size {
        let landed = held
            .resize(asked)
            .map_err(|seen| seen.failure("resize", asked))?;
        report.line(&format!("resize=ok {}", landed.figures()))?;
        now = landed.client;
        settle(&mut held, "after-resize.jpg", &mut report)?;
    }
    // `--hold` closes a menu with the inert spot's tap, known at one size:
    // refused before any gesture.
    if args.hold && now != KNOWN_SIZE {
        return Err(format!(
            "hold: at the editor's known size only ({}x{}, no --size)",
            KNOWN_SIZE.0, KNOWN_SIZE.1
        ));
    }
    let (band, to) = (
        scaled(args.band, original, now),
        scaled(args.to, original, now),
    );
    capture(&mut held, args.frames, &mut report)?;
    double_tap(&mut held, band, "band")?;
    report.line(&format!("band=ok at={},{}", band.0, band.1))?;
    settle(&mut held, "after-band.jpg", &mut report)?;
    let steps = drag(&mut held, band, to)?;
    report.line(&format!("drag=ok steps={steps} to={},{}", to.0, to.1))?;
    settle(&mut held, "after-drag.jpg", &mut report)?;
    if args.hold {
        let spot = args.inert.unwrap_or_else(|| inert_spot(KNOWN_SIZE.0));
        hold(&mut held, to, spot, &mut report)?;
        settle(&mut held, "after-hold.jpg", &mut report)?;
    }
    if args.paint > 0 {
        paint(&mut held, to, args.paint, &mut report)?;
        settle(&mut held, "after-paint.jpg", &mut report)?;
    }
    double_tap(&mut held, to, "field")?;
    report.line("field=ok")?;
    settle(&mut held, "field-open.jpg", &mut report)?;
    // The guard, as the hub's: the picture placed at the size the inert
    // spot is known at, whatever size it has, each placement seen to land
    // (#74 review, round 2); not so, no tap.
    let back = held.known().map_err(|why| format!("restore: {why}"))?;
    report.line(&format!("restore=ok {}", back.figures()))?;
    let spot = args.inert.unwrap_or_else(|| inert_spot(KNOWN_SIZE.0));
    held.guard_tap(spot)
        .map_err(|why| format!("guard at {},{}: {why}", spot.0, spot.1))?;
    report.line(&format!(
        "guard=ok at={},{} tap_ms={}",
        spot.0,
        spot.1,
        GUARD_TAP.as_millis()
    ))?;
    settle(&mut held, "after-guard.jpg", &mut report)?;
    held.release();
    report.line("release=ok")?;
    if args.close {
        held.backend.close_window(&held.taken);
        report.line("close=sent")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
