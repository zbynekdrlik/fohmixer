//! `fohmixer-hub config check <file>` (#17) and `layout check <layout>
//! <config>` (#21) end to end: the installer runs them on the config it is
//! about to write and the layout the hub will serve, before it stops the
//! running hub.

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

fn layout_check(layout: &std::path::Path, config: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["layout", "check"])
        .arg(layout)
        .arg(config)
        .output()
        .expect("run fohmixer-hub")
}

#[test]
fn a_layout_the_hub_would_serve_passes_and_another_is_exit_2_with_why() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // The default instances: band and master.
    let config = dir.path().join("fohmixer-hub.toml.check");
    std::fs::write(&config, "http_port = 8480\n").unwrap();
    let out = layout_check(&fixtures.join("layout-ok.json"), &config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.ends_with(": OK\n"), "{stderr}");
    // It does not validate.
    let out = layout_check(&fixtures.join("layout-bad.json"), &config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("layout-bad.json"), "{stderr}");
    assert!(
        stderr.contains("pages[0].rows[0].sections[0].color"),
        "{stderr}"
    );
    // A schema 1 layout (the one before the redesign) does not parse.
    let old = dir.path().join("old.json");
    std::fs::write(
        &old,
        r#"{"schema": 1, "canvas": {"w": 2360, "h": 1640}, "pages": []}"#,
    )
    .unwrap();
    let out = layout_check(&old, &config);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("does not parse"), "{stderr}");
    // It binds an instance the config does not have.
    let band_only = dir.path().join("band-only.toml");
    std::fs::write(
        &band_only,
        "http_port = 8480\n[[instances]]\nname = \"band\"\nport = 39101\n",
    )
    .unwrap();
    let out = layout_check(&fixtures.join("layout-ok.json"), &band_only);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains(r#"unknown instance "master""#), "{stderr}");
    // A config the hub refuses, or a missing file, is exit 2 too.
    let out = layout_check(
        &fixtures.join("layout-ok.json"),
        &dir.path().join("none.toml"),
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading config "));
    let out = layout_check(&dir.path().join("none.json"), &config);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading layout "));
    let out = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .args(["layout", "check"])
        .arg(fixtures.join("layout-ok.json"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("layout check <layout> <config>"));
}

fn markers(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .arg("markers")
        .args(args)
        .output()
        .expect("run fohmixer-hub")
}

#[test]
fn markers_plan_prints_each_tracks_marker_and_frame_writes_a_new_file() {
    // #68 PR C: the migration of a frame's strips bound by name.
    let dir = tempfile::tempdir().unwrap();
    let layout = dir.path().join("layout.json");
    std::fs::write(
        &layout,
        serde_json::to_vec(&serde_json::json!({
            "schema": 2,
            "default_page": "main",
            "pages": [{"id": "main", "title": "M", "rows": [{"sections": [
                {"kind": "group", "id": "g", "title": "Vocals", "controls": [
                    {"kind": "strip", "strip_kind": "standard", "mute_guard": true,
                     "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Vox 1 #"}}}]}]}]}]
        }))
        .unwrap(),
    )
    .unwrap();
    let out = markers(&[std::ffi::OsStr::new("plan"), layout.as_os_str()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "band\ttrack\tVox 1 #\t\"Vox\" +G:VOCALS:1 +MG\n"
    );
    assert!(stderr.contains("1 tracks planned"), "{stderr}");
    let converted = dir.path().join("converted.json");
    let out = markers(&[
        std::ffi::OsStr::new("frame"),
        layout.as_os_str(),
        converted.as_os_str(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("1 tracks come from markers"), "{stderr}");
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&converted).unwrap()).unwrap();
    assert_eq!(
        written["pages"][0]["rows"][0]["sections"][0]["tags"],
        "VOCALS"
    );
    // Never over a file, and a layout that cannot be read is exit 2.
    let out = markers(&[
        std::ffi::OsStr::new("frame"),
        layout.as_os_str(),
        converted.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("creating "));
    let out = markers(&[
        std::ffi::OsStr::new("plan"),
        dir.path().join("none.json").as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("reading layout "));
    assert!(out.stdout.is_empty());
}

#[test]
fn markers_plan_says_what_stops_the_migration() {
    let dir = tempfile::tempdir().unwrap();
    let layout = dir.path().join("layout.json");
    std::fs::write(
        &layout,
        serde_json::to_vec(&serde_json::json!({
            "schema": 2,
            "default_page": "main",
            "pages": [{"id": "main", "title": "M", "rows": [{"sections": [
                {"kind": "group", "id": "g", "title": "G", "controls": [
                    {"kind": "strip", "strip_kind": "standard", "wide": true,
                     "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Bass #"}}}]}]}]}]
        }))
        .unwrap(),
    )
    .unwrap();
    let out = markers(&[std::ffi::OsStr::new("plan"), layout.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "band\ttrack\tBass #\t\"Bass\" +G:G:1\n"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr
            .contains(r#"problem: "Bass #" (band): its width, label, path or kind would be lost"#),
        "{stderr}"
    );
}
