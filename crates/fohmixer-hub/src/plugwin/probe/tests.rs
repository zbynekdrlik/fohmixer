use serde_json::Value;

use super::*;
use crate::plugwin::sim::{REFUSED, SimHandle};
use crate::plugwin::{CAPTURE_MS, KEEPALIVE_MS, NOT_BACK, WindowId};

fn args(pid: u32) -> Args {
    Args {
        pid,
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
    }
}

#[test]
fn the_arguments_have_their_defaults_and_every_flag_is_read() {
    assert_eq!(parse(&["--pid", "42"]), Ok(args(42)));
    assert_eq!(
        parse(&[
            "--sim",
            "--frames",
            "3",
            "--band",
            "10,20",
            "--to",
            " 30 , 40 ",
            "--inert",
            "500,12",
            "--out",
            "frames",
            "--close",
            "--size",
            " 760 x 1271 ",
            "--min-probe",
            "--paint",
            "20",
            "--pid",
            "7",
        ]),
        Ok(Args {
            pid: 7,
            frames: 3,
            band: (10, 20),
            to: (30, 40),
            inert: Some((500, 12)),
            out: Some(PathBuf::from("frames")),
            close: true,
            sim: true,
            size: Some((760, 1271)),
            min_probe: true,
            paint: 20,
        })
    );
    assert_eq!(
        parse(&["--pid", "1", "--paint", "x"]),
        Err("--paint \"x\"".to_string())
    );
    for bad in [
        "760",
        "0x1271",
        "760x0",
        "ax1271",
        "760x",
        "-760x1271",
        "760,1271",
    ] {
        assert_eq!(
            parse(&["--pid", "1", "--size", bad]),
            Err(format!("--size {bad:?}")),
            "{bad}"
        );
    }
    assert_eq!(
        parse(&["--pid", "1", "--size", "1x1"]).unwrap().size,
        Some((1, 1))
    );
    assert_eq!(parse(&[]), Err("--pid is required".to_string()));
    assert_eq!(parse(&["--pid"]), Err("--pid needs a value".to_string()));
    assert_eq!(parse(&["--pid", "x"]), Err("--pid \"x\"".to_string()));
    assert_eq!(
        parse(&["--pid", "1", "--frames", "-1"]),
        Err("--frames \"-1\"".to_string())
    );
    assert_eq!(
        parse(&["--pid", "1", "--band", "10"]),
        Err("--band \"10\"".to_string())
    );
    assert_eq!(
        parse(&["--pid", "1", "--to", "a,2"]),
        Err("--to \"a,2\"".to_string())
    );
    assert_eq!(
        parse(&["--pid", "1", "--inert", "1,b"]),
        Err("--inert \"1,b\"".to_string())
    );
    assert_eq!(
        parse(&["--pid", "1", "--loud"]),
        Err("unknown argument \"--loud\"".to_string())
    );
    assert_eq!(FRAMES, 50);
    assert_eq!((BAND, TO), ((431, 321), (511, 281)));
}

#[test]
fn the_probes_figures() {
    assert_eq!(stats(&[]), (0.0, 0.0));
    assert_eq!(stats(&[2.0, 6.0, 4.0]), (4.0, 6.0));
    assert_eq!(stats(&[-1.0]), (-1.0, -1.0));
    assert_eq!(due_at(0), Duration::ZERO);
    assert_eq!(due_at(3), Duration::from_millis(120));
    assert_eq!(PERIOD, Duration::from_millis(CAPTURE_MS as u64));
    assert_eq!(rate(25, 0.5), 50.0);
    assert_eq!(rate(25, 0.0), 0.0);
    let points = drag_points((431, 321), (511, 281), 25);
    assert_eq!(points.len(), 25);
    assert_eq!(points[0], (434, 320));
    assert_eq!(points[12], (472, 301));
    assert_eq!(points[24], (511, 281));
    assert_eq!(drag_points((0, 0), (10, 10), 0), vec![(10, 10)]);
    assert_eq!((RESENDS, RESEND), (6, Duration::from_millis(50)));
    assert_eq!(RESEND, Duration::from_millis(KEEPALIVE_MS as u64));
    // PR M: the paint trials' points and figures.
    assert_eq!(
        (PAINT_POLL, PAINT_WAIT, PAINT_STEP, PAINT_REST),
        (
            Duration::from_millis(5),
            Duration::from_millis(500),
            8,
            Duration::from_millis(150)
        )
    );
    assert!(paint_over(500.0));
    assert!(!paint_over(500.0_f64.next_down()));
    assert!(rested(150.0));
    assert!(!rested(150.0_f64.next_down()));
    assert_eq!(paint_point((511, 281), 0), (511, 273));
    assert_eq!(paint_point((511, 281), 1), (511, 281));
    assert_eq!(paint_point((511, 281), 2), (511, 273));
    assert_eq!(percentile(&[], 0.5), None);
    assert_eq!(percentile(&[3.0, 1.0, 2.0], 0.5), Some(2.0));
    assert_eq!(percentile(&[4.0, 1.0, 3.0, 2.0], 0.5), Some(2.0));
    let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
    assert_eq!(percentile(&twenty, 0.95), Some(19.0));
    assert_eq!(percentile(&twenty, 1.0), Some(20.0));
    assert_eq!(percentile(&twenty, 0.0), Some(1.0));
    assert_eq!(percentile(&[5.0], 0.95), Some(5.0));
    assert_eq!(ms_text(None), "-");
    assert_eq!(ms_text(Some(12.345)), "12.3");
    assert_eq!(PAINT_GAP, Duration::from_millis(20));
}

/// A sim with one still window of process `pid`.
fn one_window(pid: u32) -> (Sim, SimHandle, WindowId) {
    let (sim, handle) = Sim::new(None);
    handle.auto_open(false);
    handle.still(true);
    let window = handle.add_window(pid);
    (sim, handle, window)
}

/// The sim's records as op, phase, x, y.
fn ops(handle: &SimHandle) -> Vec<(String, String, i64, i64)> {
    handle
        .records()
        .iter()
        .map(|r| {
            let text = |v: &Value| v.as_str().unwrap_or("").to_string();
            (
                text(&r["op"]),
                text(&r["phase"]),
                r["x"].as_i64().unwrap_or(-1),
                r["y"].as_i64().unwrap_or(-1),
            )
        })
        .collect()
}

fn op(op: &str, phase: &str, x: i64, y: i64) -> (String, String, i64, i64) {
    (op.to_string(), phase.to_string(), x, y)
}

#[test]
fn a_probe_runs_the_hubs_steps_in_order_and_saves_its_pictures() {
    let (mut sim, handle, window) = one_window(42);
    handle.add_window(7);
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("frames");
    let mut probe = args(42);
    probe.frames = 3;
    probe.out = Some(folder.clone());
    probe.close = true;
    let mut out = Vec::new();
    run(&mut sim, &probe, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "windows=1");
    assert_eq!(lines[1], "picture=1349x809 child=false was_topmost=false");
    assert_eq!(lines[2], "frames=3 sent=1 same=2");
    assert!(lines[3].starts_with("grab_ms_mean="), "{}", lines[3]);
    assert!(lines[4].starts_with("encode_ms_mean="), "{}", lines[4]);
    assert!(lines[5].starts_with("fps="), "{}", lines[5]);
    assert_eq!(
        lines[6..],
        [
            "band=ok at=431,321",
            "drag=ok steps=25 to=511,281",
            "field=ok",
            lines[9],
            "guard=ok at=546,15 tap_ms=30",
            "release=ok",
            "close=sent",
        ]
    );
    // The guard's placement, at the known size already (#74 review, round
    // 2: whatever size it has, each placement seen to land).
    assert!(
        lines[9].starts_with("restore=ok client=1349x809 ms="),
        "{text}"
    );
    let ops = ops(&handle);
    let mut want = vec![op("take", "", -1, -1)];
    for _ in 0..2 {
        want.push(op("touch", "down", 431, 321));
        want.push(op("touch", "up", 431, 321));
    }
    want.push(op("touch", "down", 431, 321));
    for (x, y) in drag_points(BAND, TO, DRAG_STEPS) {
        want.push(op("touch", "update", x.into(), y.into()));
    }
    for _ in 0..RESENDS {
        want.push(op("touch", "update", 511, 281));
    }
    want.push(op("touch", "up", 511, 281));
    for _ in 0..2 {
        want.push(op("touch", "down", 511, 281));
        want.push(op("touch", "up", 511, 281));
    }
    // A pixel aside, then where it goes (the take's rectangle: the release
    // posts nothing).
    want.push(op("resize", "", -1, -1));
    want.push(op("resize", "", -1, -1));
    want.push(op("touch", "down", 546, 15));
    want.push(op("touch", "up", 546, 15));
    want.push(op("release", "", -1, -1));
    want.push(op("close", "", -1, -1));
    assert_eq!(ops, want);
    assert_eq!(handle.topmost(WindowId(2)), Some(false), "another window");
    assert!(
        !handle.windows().contains(&window),
        "the close request closed it"
    );
    let mut saved: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    saved.sort();
    assert_eq!(
        saved,
        [
            "after-band.jpg",
            "after-drag.jpg",
            "after-guard.jpg",
            "field-open.jpg",
            "frame-00.jpg"
        ]
    );
    let jpeg = std::fs::read(folder.join("after-guard.jpg")).unwrap();
    assert_eq!(jpeg[..2], [0xFF_u8, 0xD8]);
}

/// A probe's `--paint` lines on a still sim set up by `setup` (e.g. its
/// picture showing each update some time after it), the sim's handle and
/// the pictures saved.
fn painted(trials: u32, setup: impl Fn(&SimHandle)) -> (Vec<String>, SimHandle, Vec<String>) {
    let (mut sim, handle, _) = one_window(42);
    setup(&handle);
    let dir = tempfile::tempdir().unwrap();
    let mut probe = args(42);
    probe.frames = 1;
    probe.paint = trials;
    probe.out = Some(dir.path().to_path_buf());
    let mut out = Vec::new();
    run(&mut sim, &probe, &mut out).unwrap();
    let lines = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let mut saved: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    saved.sort();
    (lines, handle, saved)
}

/// The paint contact's touches as the sim got them (phase, x, y, sim ms):
/// from its down right after the drag's up to its own up.
fn paint_touches(handle: &SimHandle) -> Vec<(String, i64, i64, f64)> {
    let touches: Vec<(String, i64, i64, f64)> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "touch")
        .map(|r| {
            (
                r["phase"].as_str().unwrap().to_string(),
                r["x"].as_i64().unwrap(),
                r["y"].as_i64().unwrap(),
                r["t"].as_f64().unwrap(),
            )
        })
        .collect();
    let on_band =
        |t: &(String, i64, i64, f64), phase: &str| t.0 == phase && (t.1, t.2) == (511, 281);
    let start = touches
        .windows(2)
        .position(|w| on_band(&w[0], "up") && on_band(&w[1], "down"))
        .expect("the paint contact's down after the drag's up")
        + 1;
    let end = start
        + touches[start..]
            .iter()
            .position(|t| t.0 == "up")
            .expect("its up");
    touches[start..=end].to_vec()
}

/// The figure `name=` of a probe's line.
fn figure(line: &str, name: &str) -> f64 {
    line.split(' ')
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("{name} in {line}"))
}

#[test]
fn a_paint_probe_measures_how_long_the_picture_takes_to_show_an_update() {
    // #71 PR M: the sim shows each update 30 ms after it.
    let (lines, handle, saved) = painted(3, |h| h.paint_after(Some(Duration::from_millis(30))));
    let at = lines
        .iter()
        .position(|l| l.starts_with("paint=ok"))
        .expect("the paint lines");
    assert_eq!(lines[at - 1], "drag=ok steps=25 to=511,281");
    assert_eq!(lines[at], "paint=ok n=3 lost=0 unsettled=0");
    let figures = &lines[at + 1];
    for name in [
        "paint_ms_min",
        "paint_ms_p50",
        "paint_ms_p95",
        "paint_ms_max",
    ] {
        let ms = figure(figures, name);
        assert!((25.0..=130.0).contains(&ms), "{name} {ms}: {figures}");
    }
    // The grabs' mean is there (a number).
    let _ = figure(figures, "grab_ms_mean");
    assert_eq!(lines[at + 2], "field=ok");
    // The contact: down on the band, updates 8 px up and back, the up back
    // on the band; then the field's double tap there.
    let ops = ops(&handle);
    let paint_down = ops
        .windows(2)
        .position(|w| w[0] == op("touch", "up", 511, 281) && w[1] == op("touch", "down", 511, 281))
        .expect("the paint contact's down after the drag's up")
        + 1;
    let field = ops
        .iter()
        .rposition(|o| o.0 == "touch" && o.1 == "down" && (o.2, o.3) == (511, 281))
        .unwrap();
    // The field's double tap: down, up, down (the last down), up.
    let contact = &ops[paint_down..field - 2];
    assert_eq!(contact[0], op("touch", "down", 511, 281));
    assert_eq!(contact[contact.len() - 1], op("touch", "up", 511, 281));
    let moves = &contact[1..contact.len() - 1];
    assert!(
        moves
            .iter()
            .all(|o| o.1 == "update" && o.2 == 511 && (o.3 == 273 || o.3 == 281)),
        "{moves:?}"
    );
    assert!(
        moves.iter().filter(|o| o.3 == 273).count() >= 2,
        "{moves:?}"
    );
    // Three trials end 8 px up: the contact goes back to the band first.
    assert_eq!(moves[moves.len() - 1], op("touch", "update", 511, 281));
    // The contact rested its 150 ms before the first trial's update.
    let times: Vec<(i64, i64, f64)> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "touch")
        .map(|r| {
            (
                r["x"].as_i64().unwrap(),
                r["y"].as_i64().unwrap(),
                r["t"].as_f64().unwrap(),
            )
        })
        .collect();
    let touches_before = ops[..paint_down].iter().filter(|o| o.0 == "touch").count();
    let down_t = times[touches_before].2;
    let first_up = times
        .iter()
        .find(|(x, y, t)| (*x, *y) == (511, 273) && *t > down_t)
        .unwrap()
        .2;
    assert!(first_up - down_t >= 145.0, "{}", first_up - down_t);
    assert!(saved.contains(&"after-paint.jpg".to_string()), "{saved:?}");
}

#[test]
fn a_paint_trial_whose_picture_never_changes_is_lost_and_its_contact_kept_alive() {
    let (lines, handle, _) = painted(2, |_| ());
    let at = lines
        .iter()
        .position(|l| l.starts_with("paint=ok"))
        .expect("the paint lines");
    assert_eq!(lines[at], "paint=ok n=2 lost=2 unsettled=0");
    // No sample: no figure (never a 0 ms that reads as a measure).
    assert!(
        lines[at + 1].starts_with(
            "paint_ms_min=- paint_ms_p50=- paint_ms_p95=- paint_ms_max=- grab_ms_mean="
        ),
        "{}",
        lines[at + 1]
    );
    // Windows cancels a contact silent for 100 ms: none was (one gap of a
    // loaded runner's stall is let through; a keep-alive that does not run
    // leaves many).
    let touches = paint_touches(&handle);
    let gaps: Vec<f64> = touches.windows(2).map(|w| w[1].3 - w[0].3).collect();
    assert!(
        gaps.iter().filter(|gap| **gap >= 100.0).count() <= 1,
        "{gaps:?}"
    );
    // The trials grab every 5 ms (a grab that never waits would make
    // thousands in 500 ms).
    assert!(handle.grabs() <= 300, "{}", handle.grabs());
    // Each trial waited its 500 ms, the point resent every 50 ms: the
    // first trial's point (8 px up) went at least 8 times.
    let up = ops(&handle)
        .iter()
        .filter(|o| *o == &op("touch", "update", 511, 273))
        .count();
    assert!(up >= 8, "{up}");
}

#[test]
fn a_paint_trial_whose_picture_never_rests_is_unsettled() {
    // The PR M review: a picture still moving before the update would read
    // as an instant paint.
    let (lines, handle, _) = painted(2, |h| h.restless(true));
    let at = lines
        .iter()
        .position(|l| l.starts_with("paint=ok"))
        .expect("the paint lines");
    assert_eq!(lines[at], "paint=ok n=2 lost=0 unsettled=2");
    assert_eq!(
        lines[at + 1],
        "paint_ms_min=- paint_ms_p50=- paint_ms_p95=- paint_ms_max=- grab_ms_mean=-"
    );
    // Each trial looked for a still picture every 20 ms for 500 ms (a
    // wait that never sleeps would grab far more).
    assert!(handle.grabs() <= 120, "{}", handle.grabs());
    // No update went: the contact stayed on the band.
    let touches = paint_touches(&handle);
    assert!(
        touches.iter().all(|t| (t.1, t.2) == (511, 281)),
        "{touches:?}"
    );
}

#[test]
fn a_probe_names_the_step_that_failed() {
    let (mut sim, _handle, _) = one_window(42);
    let mut out = Vec::new();
    assert_eq!(
        run(&mut sim, &args(9), &mut out),
        Err("no visible window of process 9".to_string())
    );
    assert_eq!(String::from_utf8(out).unwrap(), "windows=0\n");
    let (mut sim, handle, _) = one_window(42);
    handle.refuse(true);
    let mut probe = args(42);
    probe.frames = 1;
    probe.inert = Some((1, 2));
    let mut out = Vec::new();
    assert_eq!(
        run(&mut sim, &probe, &mut out),
        Err(format!("band: down at 431,321: {REFUSED}"))
    );
    // The refused down left no contact; the window is handed back.
    assert_eq!(
        ops(&handle),
        vec![op("take", "", -1, -1), op("release", "", -1, -1)]
    );
}

#[test]
fn a_probe_waits_for_its_window_on_top_and_hands_back_one_that_never_comes() {
    // The take's z-order change is posted: the probe grabs nothing before
    // the window is on top, as the hub's take answers nothing before.
    assert_eq!(TOP_LOOKS, 300);
    let (mut sim, handle, _) = one_window(42);
    handle.topmost_late(true);
    let mut probe = args(42);
    probe.frames = 1;
    let mut out = Vec::new();
    let started = Instant::now();
    assert_eq!(
        run(&mut sim, &probe, &mut out),
        Err(crate::plugwin::NOT_ON_TOP.to_string())
    );
    let waited = started.elapsed();
    assert!(
        (Duration::from_millis(3000)..Duration::from_secs(10)).contains(&waited),
        "{waited:?}"
    );
    assert_eq!(String::from_utf8(out).unwrap(), "windows=1\n");
    assert_eq!(
        ops(&handle),
        vec![op("take", "", -1, -1), op("release", "", -1, -1)]
    );
}

#[test]
fn a_step_that_fails_ends_its_contact_and_hands_the_window_back() {
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    {
        let mut held = Held {
            backend: &mut sim,
            taken,
            contact: None,
            released: false,
            changed: false,
        };
        held.touch(Phase::Down, (10, 20), "drag").unwrap();
        held.touch(Phase::Update, (11, 21), "drag").unwrap();
        handle.refuse(true);
        assert_eq!(
            held.touch(Phase::Update, (12, 22), "drag"),
            Err(format!("drag: update at 12,22: {REFUSED}"))
        );
    }
    assert_eq!(
        ops(&handle),
        vec![
            op("take", "", -1, -1),
            op("touch", "down", 10, 20),
            op("touch", "update", 11, 21),
            op("touch", "cancel", 11, 21),
            op("release", "", -1, -1),
        ],
        "cancelled at its last point, then released"
    );
    // Released by the probe: once; an ended contact is not cancelled.
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    {
        let mut held = Held {
            backend: &mut sim,
            taken,
            contact: None,
            released: false,
            changed: false,
        };
        held.touch(Phase::Down, (1, 2), "tap").unwrap();
        held.touch(Phase::Up, (1, 2), "tap").unwrap();
        held.guard_tap((3, 4)).unwrap();
        held.release();
    }
    assert_eq!(
        ops(&handle),
        vec![
            op("take", "", -1, -1),
            op("touch", "down", 1, 2),
            op("touch", "up", 1, 2),
            op("touch", "down", 3, 4),
            op("touch", "up", 3, 4),
            op("release", "", -1, -1),
        ]
    );
    // A guard tap whose up fails leaves its down to cancel.
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    handle.refuse(true);
    {
        let mut held = Held {
            backend: &mut sim,
            taken,
            contact: Some((5, 6)),
            released: false,
            changed: false,
        };
        assert_eq!(held.guard_tap((3, 4)), Err(REFUSED.to_string()));
        assert_eq!(
            held.contact,
            Some((5, 6)),
            "the refused down changed nothing"
        );
    }
    assert_eq!(
        ops(&handle)[1..],
        [op("touch", "cancel", 5, 6), op("release", "", -1, -1)]
    );
}

#[test]
fn the_probes_backend_is_the_sim_or_the_platforms() {
    let mut probe = args(5);
    probe.sim = true;
    let mut backend = backend(&probe).unwrap();
    assert_eq!(backend.windows_of(5).unwrap().len(), 1);
    assert!(backend.editors().unwrap().len() == 1);
    probe.sim = false;
    if cfg!(windows) {
        assert!(backend_of(&probe).is_ok());
    } else {
        assert_eq!(
            backend_of(&probe).err(),
            Some("eq-probe drives Windows windows: run it on the PC (or with --sim)".to_string())
        );
    }
}

/// `backend` without `Debug` on its value.
fn backend_of(args: &Args) -> Result<(), String> {
    backend(args).map(|_| ())
}

#[test]
fn a_point_keeps_its_place_on_a_resized_picture() {
    assert_eq!(scaled((431, 321), (1349, 809), (760, 1271)), (243, 504));
    assert_eq!(scaled((511, 281), (1349, 809), (760, 1271)), (288, 441));
    assert_eq!(scaled((431, 321), (1349, 809), (1349, 809)), (431, 321));
    assert_eq!(scaled((1349, 809), (1349, 809), (760, 1271)), (760, 1271));
    // No picture to scale from: as from 1 px (never a division by 0).
    assert_eq!(scaled((100, 50), (0, 0), (200, 100)), (20_000, 5_000));
    assert_eq!(MIN_PROBE, (160, 120));
    assert_eq!(RESIZE_LOOKS, 100);
}

#[test]
fn a_probe_resizes_runs_its_gestures_at_the_new_size_and_restores_before_the_guard() {
    let (mut sim, handle, window) = one_window(42);
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("frames");
    let mut probe = args(42);
    probe.frames = 2;
    probe.size = Some((760, 1271));
    probe.out = Some(folder.clone());
    let mut out = Vec::new();
    run(&mut sim, &probe, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "picture=1349x809 child=false was_topmost=false");
    assert!(
        lines[2].starts_with("resize=ok client=760x1271 ms="),
        "{text}"
    );
    assert_eq!(lines[3], "frames=2 sent=1 same=1");
    assert_eq!(
        lines[7..],
        [
            "band=ok at=243,504",
            "drag=ok steps=25 to=288,441",
            "field=ok",
            lines[10],
            "guard=ok at=546,15 tap_ms=30",
            "release=ok",
        ]
    );
    assert!(
        lines[10].starts_with("restore=ok client=1349x809 ms="),
        "{text}"
    );
    let (band, to) = ((243, 504), (288, 441));
    let mut want = vec![op("take", "", -1, -1), op("resize", "", -1, -1)];
    for _ in 0..2 {
        want.push(op("touch", "down", band.0, band.1));
        want.push(op("touch", "up", band.0, band.1));
    }
    want.push(op("touch", "down", band.0, band.1));
    for (x, y) in drag_points((243, 504), (288, 441), DRAG_STEPS) {
        want.push(op("touch", "update", x.into(), y.into()));
    }
    for _ in 0..RESENDS {
        want.push(op("touch", "update", to.0, to.1));
    }
    want.push(op("touch", "up", to.0, to.1));
    for _ in 0..2 {
        want.push(op("touch", "down", to.0, to.1));
        want.push(op("touch", "up", to.0, to.1));
    }
    want.push(op("resize", "", -1, -1));
    want.push(op("resize", "", -1, -1));
    want.push(op("touch", "down", 546, 15));
    want.push(op("touch", "up", 546, 15));
    want.push(op("release", "", -1, -1));
    assert_eq!(ops(&handle), want);
    let sizes: Vec<(u64, u64)> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "resize")
        .map(|r| (r["w"].as_u64().unwrap(), r["h"].as_u64().unwrap()))
        .collect();
    assert_eq!(sizes, [(760, 1271), (1349, 809), (1349, 809)]);
    assert_eq!(handle.size(window), Some((1349, 809)));
    assert!(folder.join("after-resize.jpg").exists());
}

#[test]
fn a_min_probe_reads_the_editors_own_minimum_and_puts_its_size_back() {
    let (mut sim, handle, window) = one_window(42);
    let mut probe = args(42);
    probe.frames = 1;
    probe.min_probe = true;
    let mut out = Vec::new();
    let started = Instant::now();
    run(&mut sim, &probe, &mut out).unwrap();
    // It never lands: the probe waits the whole second.
    assert!(started.elapsed() >= Duration::from_millis(1000));
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[2], "min=320x240 asked=160x120", "{text}");
    assert!(
        lines[3].starts_with("min_restore=ok client=1349x809 ms="),
        "{text}"
    );
    assert_eq!(lines[4], "frames=1 sent=1 same=0");
    assert!(text.contains("\nband=ok at=431,321\n"), "{text}");
    // Back already, and the guard still places it (#74 review, round 2: a
    // restore read at its size may have landed before what was posted
    // first).
    assert!(text.contains("\nrestore=ok client=1349x809 ms="), "{text}");
    let resizes = ops(&handle).into_iter().filter(|o| o.0 == "resize").count();
    assert_eq!(resizes, 4);
    assert_eq!(handle.size(window), Some((1349, 809)));
}

#[test]
fn a_resize_that_does_not_land_fails_the_probe_and_its_size_is_put_back() {
    let (mut sim, handle, _) = one_window(42);
    handle.resize_refused(true);
    let mut probe = args(42);
    probe.frames = 1;
    probe.size = Some((760, 1271));
    let mut out = Vec::new();
    let failed = run(&mut sim, &probe, &mut out).unwrap_err();
    assert!(
        failed.starts_with("resize: 760x1271 asked, the picture is 1349x809 after "),
        "{failed}"
    );
    assert_eq!(
        ops(&handle),
        vec![
            op("take", "", -1, -1),
            op("resize", "", -1, -1),
            op("resize", "", -1, -1),
            op("release", "", -1, -1),
        ],
        "its own size posted again, then released"
    );
}

#[test]
fn a_resize_says_how_it_settled_or_why_it_was_not_posted() {
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    let mut held = Held {
        backend: &mut sim,
        taken,
        contact: None,
        released: false,
        changed: false,
    };
    let landed = held.resize((760, 1271)).unwrap();
    assert_eq!((landed.client, landed.why.as_deref()), ((760, 1271), None));
    assert!(held.changed);
    assert_eq!(
        landed.figures(),
        format!("client=760x1271 ms={:.0}", landed.ms)
    );
    let back = held.resize((1349, 809)).unwrap();
    assert_eq!(back.client, (1349, 809));
    assert!(!held.changed, "its own size again");
    handle.remove_window(window);
    let lost = held.resize((760, 1271)).unwrap_err();
    assert_eq!(lost.why.as_deref(), Some("no such window"));
    assert_eq!(
        lost.failure("resize", (760, 1271)),
        "resize: 760x1271 could not be posted: no such window"
    );
    let unlanded = Settle {
        client: (1349, 809),
        ms: 1000.4,
        why: None,
    };
    assert_eq!(
        unlanded.failure("restore", (1349, 809)),
        "restore: 1349x809 asked, the picture is 1349x809 after 1000 ms"
    );
    assert_eq!(unlanded.figures(), "client=1349x809 ms=1000");
}

#[test]
fn a_probe_taps_only_at_the_size_its_inert_spot_is_known_at() {
    // The review of PR #74 (I1): the probe's guard as the hub's. A window
    // taken at 760 × 1271 is put at 1349 × 809 (the size the inert spot was
    // verified at) before its tap, and its release puts the take's size
    // back.
    let (mut sim, handle, window) = one_window(42);
    handle.resize_window(window, (760, 1271));
    let mut probe = args(42);
    probe.frames = 1;
    let mut out = Vec::new();
    run(&mut sim, &probe, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\nrestore=ok client=1349x809 ms="), "{text}");
    assert!(text.contains("\nguard=ok at=546,15 tap_ms=30\n"), "{text}");
    let sizes: Vec<(u64, u64)> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "resize")
        .map(|r| (r["w"].as_u64().unwrap(), r["h"].as_u64().unwrap()))
        .collect();
    assert_eq!(sizes, [(1349, 809), (1349, 809), (760, 1271)]);
    assert_eq!(handle.size(window), Some((760, 1271)));
    let tapped: Vec<_> = ops(&handle)
        .into_iter()
        .filter(|o| o.2 == 546 && o.3 == 15)
        .collect();
    assert_eq!(
        tapped,
        [op("touch", "down", 546, 15), op("touch", "up", 546, 15)]
    );
}

#[test]
fn a_restore_that_does_not_land_keeps_the_size_changed_for_the_hand_back() {
    // The review of PR #74 (M10): `--min-probe`'s way back cleared
    // `changed` when it was asked, so a restore that failed left the
    // editor at Pro-Q's minimum for the rest of the run.
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    {
        let mut held = Held {
            backend: &mut sim,
            taken,
            contact: None,
            released: false,
            changed: false,
        };
        assert!(held.resize(MIN_PROBE).is_err(), "Pro-Q's minimum instead");
        handle.resize_refused(true);
        assert!(held.restore().is_err(), "it never lands");
        assert!(held.changed, "still changed");
        handle.resize_refused(false);
    }
    // Dropped: the take's size posted again, and handed back.
    let resizes = ops(&handle).into_iter().filter(|o| o.0 == "resize").count();
    assert_eq!(resizes, 3);
    assert_eq!(handle.size(window), Some((1349, 809)));
    assert_eq!(ops(&handle).last(), Some(&op("release", "", -1, -1)));
}

#[test]
fn the_probes_guard_places_its_editor_a_pixel_aside_first_then_where_it_goes() {
    // The review of PR #74, round 2: the probe's guard as the hub's. The
    // window moved on the PC to exactly where the first placement would go
    // a pixel to the right, so that one goes a pixel to the left: one the
    // window does not have before it lands.
    let (mut sim, handle, window) = one_window(42);
    let taken = sim.take(window).unwrap();
    handle.move_window(window, (1, 0));
    {
        let mut held = Held {
            backend: &mut sim,
            taken,
            contact: None,
            released: false,
            changed: false,
        };
        let placed = held.known().unwrap();
        assert_eq!((placed.client, placed.why), ((1349, 809), None));
        assert!(!held.changed, "the take's rectangle again");
        held.release();
    }
    let posted: Vec<(i64, i64)> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "resize")
        .map(|r| (r["left"].as_i64().unwrap(), r["top"].as_i64().unwrap()))
        .collect();
    assert_eq!(posted, [(-1, 0), (0, 0)], "nothing posted at the release");
}

#[test]
fn a_probe_whose_guard_placement_does_not_land_taps_nothing_and_puts_its_size_back() {
    // The review of PR #74, round 2: a placement not seen to land within
    // its second fails the probe, with no tap.
    let (mut sim, handle, _) = one_window(42);
    handle.resize_refused(true);
    let mut probe = args(42);
    probe.frames = 1;
    let mut out = Vec::new();
    assert_eq!(
        run(&mut sim, &probe, &mut out),
        Err(format!(
            "restore: {NOT_BACK}: its first placement did not land (the window is 1365x848 at 0,0, 1365x848 at 1,0 posted)"
        ))
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.ends_with("\nfield=ok\n"), "{text}");
    let ops = ops(&handle);
    assert!(
        ops.iter().all(|o| (o.2, o.3) != (546, 15)),
        "no tap: {ops:?}"
    );
    assert_eq!(
        ops[ops.len() - 3..],
        [
            op("resize", "", -1, -1),
            op("resize", "", -1, -1),
            op("release", "", -1, -1),
        ],
        "its placement, the take's rectangle again, then released"
    );
}
