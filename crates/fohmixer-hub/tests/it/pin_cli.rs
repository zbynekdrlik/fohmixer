//! `fohmixer-hub pin set-engineer` end to end (from iemmixer's
//! `tests/pin_cli.rs` @ 22372bc): the PIN is read from stdin and only an
//! argon2id hash is stored in the data folder's `secrets/`.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_pin(dir: &std::path::Path, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["pin", "set-engineer"])
        .env("FOHMIXER_DATA", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fohmixer-hub");
    // The child may exit before reading stdin; a broken pipe here is then
    // expected and the exit code is what the test checks.
    let _ = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes());
    child.wait_with_output().expect("wait for fohmixer-hub")
}

fn stored(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("secrets").join("pin_hashes.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["engineer"].as_str().map(str::to_string)
}

#[test]
fn set_engineer_stores_only_a_hash() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_pin(dir.path(), "2468\n");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("stored the engineer PIN hash"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hash = stored(dir.path()).expect("a hash");
    assert!(hash.starts_with("$argon2id$"), "{hash}");
    let text = std::fs::read_to_string(dir.path().join("secrets").join("pin_hashes.json")).unwrap();
    assert!(!text.contains("\"2468\""));
}

#[test]
fn an_invalid_pin_is_rejected_with_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_pin(dir.path(), "12a4\n");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("the PIN must be exactly 4 digits"));
    assert!(stored(dir.path()).is_none());
}

#[test]
fn a_lost_pepper_next_to_a_hash_is_an_error_with_exit_1() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run_pin(dir.path(), "2468\n").status.success());
    let secrets = dir.path().join("secrets");
    for entry in std::fs::read_dir(&secrets).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("pepper"))
        {
            std::fs::remove_file(path).unwrap();
        }
    }
    let out = run_pin(dir.path(), "1357\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("I/O error"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
