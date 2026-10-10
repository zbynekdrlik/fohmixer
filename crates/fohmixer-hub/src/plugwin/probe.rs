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
//!    [`super::RESIZE_MS`] (`min=`: Pro-Q's minimum), then its own size
//!    back (`min_restore=`);
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
//! 7. a double tap at `--to` (on the band: its frequency text field opens,
//!    Pro-Q 4.02's close-crash state);
//! 8. the close guard: a resized picture's own size back first, as the
//!    hub's guard does (`restore=ok client= ms=`; not back in time: the
//!    probe fails and taps nothing), then a tap on the inert spot
//!    (`--inert`, else the hub's own of the original width,
//!    [`super::inert_spot`]), then the guard's wait;
//! 9. the release (the z-order as it was) and, with `--close`, a close
//!    request to the window (Carla's bridge ends with `0xC0000005` on it
//!    whatever the plug-in's state).
//!
//! Once the window is taken, a step that fails still ends a contact it left
//! down (a cancel at its last point), posts a changed window's own
//! rectangle back (not waited for) and hands the window back.
//!
//! With `--out` every picture is saved there as a JPEG (`frame-NN.jpg`,
//! then `after-band.jpg`, `after-drag.jpg`, `field-open.jpg`,
//! `after-guard.jpg`; with `--size` first `after-resize.jpg`): the field
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
    Backend, GUARD_TAP, NOT_ON_TOP, Phase, Pixels, QUALITY, Rect, STEP, Taken, encode, inert_spot,
    inside, millis, placed, settled, window_size,
};

/// The CLI's usage line.
pub const USAGE: &str = "usage: fohmixer-hub eq-probe --pid <pid> [--frames <n>] [--size <w>x<h>] [--min-probe] [--band <x,y>] [--to <x,y>] [--inert <x,y>] [--out <folder>] [--close] [--sim]";
/// The picture `--min-probe` asks for (px): far below any Pro-Q size, so
/// the editor takes its own minimum.
pub const MIN_PROBE: (u32, u32) = (160, 120);
/// The looks at a resized picture's size, [`STEP`] apart: past
/// `RESIZE_MS` (1 s) the last one settles it ([`settled`]).
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
            "--pid" | "--frames" | "--band" | "--to" | "--inert" | "--out" | "--size" => {
                let value = rest.next().ok_or(format!("{flag} needs a value"))?;
                let bad = || format!("{flag} {value:?}");
                match *flag {
                    "--pid" => pid = Some(value.parse::<u32>().map_err(|_| bad())?),
                    "--frames" => parsed.frames = value.parse().map_err(|_| bad())?,
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
/// posts a changed picture's own size back and hands the window back (its
/// z-order as it was), unless the release ran.
struct Held<'a> {
    backend: &'a mut dyn Backend,
    taken: Taken,
    /// The contact down, at its last point.
    contact: Option<(i32, i32)>,
    released: bool,
    /// Whether the picture's size was changed and is not back yet (PR G).
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

    /// `rect` posted to the window and its picture read until it is
    /// `asked` ([`Held::resize`]).
    fn post(&mut self, rect: Rect, asked: (u32, u32)) -> Result<Settle, Settle> {
        self.backend
            .resize(&self.taken, rect)
            .map_err(|why| Settle {
                client: (0, 0),
                ms: 0.0,
                why: Some(why),
            })?;
        self.changed = asked != self.original();
        let started = Instant::now();
        for _ in 0..=RESIZE_LOOKS {
            let ms = millis(started.elapsed());
            let client = self.backend.client(&self.taken).unwrap_or((0, 0));
            match settled(client, asked, ms) {
                Some(true) => {
                    return Ok(Settle {
                        client,
                        ms,
                        why: None,
                    });
                }
                Some(false) => {
                    return Err(Settle {
                        client,
                        ms,
                        why: None,
                    });
                }
                None => std::thread::sleep(STEP),
            }
        }
        let client = self.backend.client(&self.taken).unwrap_or((0, 0));
        let ms = millis(started.elapsed());
        Err(Settle {
            client,
            ms,
            why: None,
        })
    }

    /// The window handed back (its z-order as it was).
    fn release(&mut self) {
        self.released = true;
        self.backend.release(&self.taken);
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(at) = self.contact.take() {
            let _ = self.backend.touch(&self.taken, Phase::Cancel, at);
        }
        if self.changed {
            let _ = self.backend.resize(&self.taken, self.taken.rect);
        }
        if !self.released {
            self.backend.release(&self.taken);
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
    double_tap(&mut held, to, "field")?;
    report.line("field=ok")?;
    settle(&mut held, "field-open.jpg", &mut report)?;
    // The guard: the picture's own size back first, as the hub's guard
    // does (the inert spot is known only at that size); not back, no tap.
    if held.changed {
        let back = held
            .restore()
            .map_err(|seen| seen.failure("restore", original))?;
        report.line(&format!("restore=ok {}", back.figures()))?;
    }
    let spot = args.inert.unwrap_or_else(|| inert_spot(original.0));
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
