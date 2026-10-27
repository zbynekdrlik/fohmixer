//! `fohmixer-hub cloudflare set-token` end to end (#17): the token is read
//! from stdin, stored sealed in the data folder's `secrets/`, and never
//! printed.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_token(dir: &std::path::Path, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["cloudflare", "set-token"])
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

/// A low-entropy token-shaped value (a realistic one would read as a leak
/// to the secret scan).
fn token() -> String {
    "z".repeat(40)
}

#[test]
fn set_token_stores_the_token_sealed_and_never_prints_it() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_token(dir.path(), &format!("{}\n", token()));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr: {stderr}");
    assert!(
        stderr.contains("stored the Cloudflare API token"),
        "{stderr}"
    );
    assert!(
        !stderr.contains(&token()) && out.stdout.is_empty(),
        "never printed"
    );
    let secrets = dir.path().join("secrets");
    let loaded = fohmixer_hub::cf_token::load(&secrets).unwrap().unwrap();
    assert_eq!(loaded.expose(), token());
}

#[test]
fn a_bad_token_is_rejected_with_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_token(dir.path(), "not a token\n");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("a Cloudflare API token is one line"));
    assert!(!dir.path().join("secrets").exists());
}

#[test]
fn an_unusable_data_folder_is_an_error_with_exit_1() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let out = run_token(&file, &format!("{}\n", token()));
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("I/O error"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
