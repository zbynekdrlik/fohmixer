use serde_json::Value;

use super::*;
use crate::plugwin::sim::{REFUSED, SimHandle};
use crate::plugwin::{CAPTURE_MS, KEEPALIVE_MS, WindowId};

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
        })
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
            "guard=ok at=546,15 tap_ms=30",
            "release=ok",
            "close=sent",
        ]
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
    assert_eq!(sizes, [(760, 1271), (1349, 809)]);
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
    assert!(!text.contains("\nrestore="), "back already: {text}");
    assert!(text.contains("\nband=ok at=431,321\n"), "{text}");
    let resizes = ops(&handle).into_iter().filter(|o| o.0 == "resize").count();
    assert_eq!(resizes, 2);
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
    assert_eq!(sizes, [(1349, 809), (760, 1271)]);
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
