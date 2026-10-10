//! `fohmixer-hub eq-probe` end to end (#71 PR E): bad arguments are exit 2
//! with the usage; `--sim` runs every step on the simulated backend and
//! prints its findings; off Windows the real backend is refused (exit 2), on
//! Windows a process with no window is a failed step (exit 1). Host-free: it
//! also runs in the `windows` job.

use std::process::{Command, Output};

fn probe(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .arg("eq-probe")
        .args(args)
        .output()
        .expect("run fohmixer-hub")
}

#[test]
fn bad_arguments_are_exit_2_with_the_usage() {
    for args in [&[][..], &["--pid"][..], &["--pid", "1", "--loud"][..]] {
        let out = probe(args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.contains("usage: fohmixer-hub eq-probe --pid"),
            "{stderr}"
        );
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn a_simulated_probe_runs_every_step() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("frames");
    let out = probe(&[
        "--sim",
        "--pid",
        "42",
        "--frames",
        "2",
        "--out",
        folder.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "windows=1");
    assert_eq!(lines[1], "picture=1349x809 child=false was_topmost=false");
    assert!(lines[2].starts_with("frames=2 sent="), "{stdout}");
    assert_eq!(lines[lines.len() - 1], "release=ok");
    assert!(
        stdout.contains("\nguard=ok at=546,15 tap_ms=30\n"),
        "{stdout}"
    );
    assert!(folder.join("after-guard.jpg").exists());
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn the_real_backend_runs_on_windows_only() {
    let out = probe(&["--pid", "0", "--frames", "1"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if cfg!(windows) {
        // A failed step: no window of that process (or, on a runner without
        // an interactive desktop, no touch injection at all).
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains("no visible window of process 0")
                || stderr.contains("InitializeTouchInjection"),
            "{stderr}"
        );
    } else {
        assert_eq!(out.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("run it on the PC"), "{stderr}");
    }
}
