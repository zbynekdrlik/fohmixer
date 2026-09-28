//! `fohmixer-hub config check <file>` end to end (#17): the installer runs it
//! on the config it is about to write, before it stops the running hub.

use std::process::{Command, Output};

fn check(file: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["config", "check"])
        .arg(file)
        .output()
        .expect("run fohmixer-hub")
}

#[test]
fn a_good_config_passes_and_a_bad_one_is_exit_2_with_why() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fohmixer-hub.toml.check");
    std::fs::write(
        &file,
        "http_port = 8480\n[tls]\nname = \"foh.example.org\"\nport = 443\n[acme]\n",
    )
    .unwrap();
    let out = check(&file);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.ends_with(": OK\n"), "{stderr}");
    std::fs::write(
        &file,
        "http_port = 8480\n[tls]\nname = \"foh.example.org\"\n[acme]\ndirectory = \"http://ca.example.org/dir\"\n",
    )
    .unwrap();
    let out = check(&file);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("fohmixer-hub.toml.check"), "{stderr}");
    assert!(stderr.contains("an https URL"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_missing_file_or_argument_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let out = check(&dir.path().join("none.toml"));
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading config "));
    let out = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["config", "check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("config check <file>"));
}
