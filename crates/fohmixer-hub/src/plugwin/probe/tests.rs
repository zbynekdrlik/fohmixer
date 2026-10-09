use serde_json::Value;

use super::*;
use crate::plugwin::sim::{REFUSED, SimHandle};
use crate::plugwin::{CAPTURE_MS, WindowId};

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
        })
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
    assert_eq!(RESENDS, 3);
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
    // Without --out nothing is saved; the release did not run.
    assert!(!handle.records().iter().any(|r| r["op"] == "release"));
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
