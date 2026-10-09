//! `fohmixer-hub eq-probe` (#71 PR E, D17): the window backend driven
//! against a window of one process, the isolated check on the PC against
//! Carla's bridge (`.claude/rules/plugin-window.md`) before the first open
//! in a running Live. It runs the steps the hub runs, in order, and prints
//! one `name=value` line per finding (never a window title: a Live window's
//! holds a track name):
//!
//! 1. the process's visible top-level windows; the first one is taken (on
//!    top, its picture found): `picture`, `child` (Pro-Q's own child found);
//! 2. `--frames` captures at the hub's rate: their sizes, how many differed,
//!    the grab and encode times, the frame sizes, the rate reached;
//! 3. a double tap at `--band` (on a fresh Pro-Q's curve: a new band);
//! 4. a drag from `--band` to `--to` (updates every 16 ms, a rest with the
//!    resends, the up at the last point);
//! 5. a double tap at `--to` (on the band: its frequency text field opens,
//!    Pro-Q 4.02's close-crash state);
//! 6. the close guard: a tap on the inert spot (`--inert`, else the hub's
//!    own, [`super::inert_spot`]), then the guard's wait;
//! 7. the release (the z-order as it was) and, with `--close`, a close
//!    request to the window (Carla's bridge ends with `0xC0000005` on it
//!    whatever the plug-in's state).
//!
//! With `--out` every picture is saved there as a JPEG (`frame-NN.jpg`,
//! then `after-band.jpg`, `after-drag.jpg`, `field-open.jpg`,
//! `after-guard.jpg`): the field must be open in the third and closed in
//! the fourth. `--sim` runs it on the simulated backend (the CLI's tests).
//! Exit 0, 1 when a step failed (its line says which), 2 for bad arguments.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::sim::Sim;
use super::{Backend, GUARD_TAP, Phase, Pixels, QUALITY, Taken, encode, guard_tap, inert_spot};

/// The CLI's usage line.
pub const USAGE: &str = "usage: fohmixer-hub eq-probe --pid <pid> [--frames <n>] [--band <x,y>] [--to <x,y>] [--inert <x,y>] [--out <folder>] [--close] [--sim]";
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
/// [`RESEND`] apart (the hub re-sends a resting contact every 100 ms).
pub const RESENDS: u32 = 3;
pub const RESEND: Duration = Duration::from_millis(100);
/// A tap's finger down, and the gap between a double tap's two taps.
pub const TAP: Duration = Duration::from_millis(60);
pub const TAP_GAP: Duration = Duration::from_millis(120);
/// The wait for the editor to draw what a gesture did.
pub const SETTLE: Duration = Duration::from_millis(400);

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
}

/// `x,y` as a point.
fn point(text: &str) -> Option<(i32, i32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
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
            "--pid" | "--frames" | "--band" | "--to" | "--inert" | "--out" => {
                let value = rest.next().ok_or(format!("{flag} needs a value"))?;
                let bad = || format!("{flag} {value:?}");
                match *flag {
                    "--pid" => pid = Some(value.parse::<u32>().map_err(|_| bad())?),
                    "--frames" => parsed.frames = value.parse().map_err(|_| bad())?,
                    "--band" => parsed.band = point(value).ok_or_else(bad)?,
                    "--to" => parsed.to = point(value).ok_or_else(bad)?,
                    "--inert" => parsed.inert = Some(point(value).ok_or_else(bad)?),
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

#[cfg(windows)]
fn platform() -> Result<Box<dyn Backend>, String> {
    Ok(Box::new(super::win::Win::new()))
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

/// A gesture's touch, its failure named by `step`.
fn touch(
    backend: &mut dyn Backend,
    taken: &Taken,
    phase: Phase,
    at: (i32, i32),
    step: &str,
) -> Result<(), String> {
    backend
        .touch(taken, phase, at)
        .map_err(|why| format!("{step}: {} at {},{}: {why}", phase.name(), at.0, at.1))
}

/// A double tap at `at`.
fn double_tap(
    backend: &mut dyn Backend,
    taken: &Taken,
    at: (i32, i32),
    step: &str,
) -> Result<(), String> {
    for gap in [TAP_GAP, Duration::ZERO] {
        touch(backend, taken, Phase::Down, at, step)?;
        std::thread::sleep(TAP);
        touch(backend, taken, Phase::Up, at, step)?;
        std::thread::sleep(gap);
    }
    Ok(())
}

/// A drag from `from` to `to`: the down, the updates, the rest with its
/// resends, the up at the last point.
fn drag(
    backend: &mut dyn Backend,
    taken: &Taken,
    from: (i32, i32),
    to: (i32, i32),
) -> Result<usize, String> {
    touch(backend, taken, Phase::Down, from, "drag")?;
    let points = drag_points(from, to, DRAG_STEPS);
    for at in &points {
        std::thread::sleep(DRAG_STEP);
        touch(backend, taken, Phase::Update, *at, "drag")?;
    }
    for _ in 0..RESENDS {
        std::thread::sleep(RESEND);
        touch(backend, taken, Phase::Update, to, "drag")?;
    }
    touch(backend, taken, Phase::Up, to, "drag")?;
    Ok(points.len())
}

/// Grabs `frames` pictures at the hub's rate and reports them.
fn capture(
    backend: &mut dyn Backend,
    taken: &Taken,
    frames: u32,
    report: &mut Report<'_>,
) -> Result<Option<Pixels>, String> {
    let (mut grabs, mut encodes, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
    let mut last: Option<Pixels> = None;
    let mut same = 0_u32;
    let started = Instant::now();
    for n in 0..frames {
        std::thread::sleep(due_at(n).saturating_sub(started.elapsed()));
        let at = Instant::now();
        let pixels = backend
            .grab(taken)
            .map_err(|why| format!("capture: {why}"))?;
        grabs.push(at.elapsed().as_secs_f64() * 1000.0);
        if last.as_ref() == Some(&pixels) {
            same += 1;
            continue;
        }
        let at = Instant::now();
        let jpeg = encode(&pixels, QUALITY)?;
        encodes.push(at.elapsed().as_secs_f64() * 1000.0);
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
    Ok(last)
}

/// After a gesture: the editor's wait to draw it, then its picture saved.
fn settle(
    backend: &mut dyn Backend,
    taken: &Taken,
    name: &str,
    report: &mut Report<'_>,
) -> Result<(), String> {
    std::thread::sleep(SETTLE);
    let pixels = backend
        .grab(taken)
        .map_err(|why| format!("{name}: {why}"))?;
    report.save(name, &pixels)
}

/// The probe's steps on `backend` (its thread started), the findings to
/// `out`; the first step that failed, if one did.
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
    report.line(&format!(
        "picture={}x{} child={} was_topmost={}",
        taken.width,
        taken.height,
        taken.picture != taken.window,
        taken.was_topmost
    ))?;
    let last = capture(backend, &taken, args.frames, &mut report)?;
    let width = last.as_ref().map_or(taken.width, |p| p.width);
    double_tap(backend, &taken, args.band, "band")?;
    report.line(&format!("band=ok at={},{}", args.band.0, args.band.1))?;
    settle(backend, &taken, "after-band.jpg", &mut report)?;
    let steps = drag(backend, &taken, args.band, args.to)?;
    report.line(&format!(
        "drag=ok steps={steps} to={},{}",
        args.to.0, args.to.1
    ))?;
    settle(backend, &taken, "after-drag.jpg", &mut report)?;
    double_tap(backend, &taken, args.to, "field")?;
    report.line("field=ok")?;
    settle(backend, &taken, "field-open.jpg", &mut report)?;
    let spot = args.inert.unwrap_or_else(|| inert_spot(width));
    guard_tap(backend, &taken, spot)
        .map_err(|why| format!("guard at {},{}: {why}", spot.0, spot.1))?;
    report.line(&format!(
        "guard=ok at={},{} tap_ms={}",
        spot.0,
        spot.1,
        GUARD_TAP.as_millis()
    ))?;
    settle(backend, &taken, "after-guard.jpg", &mut report)?;
    backend.release(&taken);
    report.line("release=ok")?;
    if args.close {
        backend.close_window(&taken);
        report.line("close=sent")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
