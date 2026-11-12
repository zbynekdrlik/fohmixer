//! The markers remembered across the hub's restart (#178).

use std::path::Path;
use std::time::{Duration, SystemTime};

use fohmixer_proto::markers::{Found, TrackKind};
use serde_json::json;

use super::*;

fn store(dir: &Path) -> LayoutStore {
    store_in(
        dir,
        Path::new("layout.json"),
        vec!["band".into(), "master".into()],
    )
}

/// Writes the layout file with a modified time no earlier write had (a
/// coarse clock would hide a rewrite from the poll).
fn write(dir: &Path, text: &[u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static WRITES: AtomicU64 = AtomicU64::new(1);
    let path = dir.join("layout.json");
    std::fs::write(&path, text).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    let offset = Duration::from_secs(WRITES.fetch_add(1, Ordering::Relaxed));
    file.set_modified(SystemTime::now() + offset).unwrap();
}

/// A marker of `instance` on track `index` (#178's tests).
fn marker(instance: &str, index: u32, name: &str) -> Found {
    Found {
        instance: instance.into(),
        kind: TrackKind::Track,
        index,
        name: name.into(),
        tuners: 1,
    }
}

/// The labels of the served layout's strips, in order.
fn labels(store: &LayoutStore) -> Vec<String> {
    let (_, served) = store.current();
    served
        .map(|l| {
            l.controls()
                .into_iter()
                .filter_map(|c| match c {
                    fohmixer_proto::layout::Control::Strip(s) => s.label.clone(),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn tags_frame() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": 2,
        "default_page": "main",
        "pages": [{"id": "main", "title": "M", "rows": [{"sections": [
            {"kind": "group", "id": "g", "tags": "A"}]}]}]
    }))
    .unwrap()
}

#[test]
fn the_markers_last_composed_are_served_after_a_restart_before_any_read() {
    // #178: the hub started while both Lives were down served an
    // empty frame (no strip, every +CTRL control red).
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &tags_frame());
    let first = store(dir.path());
    assert_eq!(first.poll(), Some(1));
    let found = vec![
        marker("band", 1, r#""Vox" +G:A"#),
        marker("master", 2, r#""Key" +G:A"#),
    ];
    assert_eq!(first.set_markers(found.clone()), Some(2));
    assert_eq!(labels(&first), vec!["Vox", "Key"]);
    drop(first);
    // The hub starts again: the first poll composes the remembered
    // markers, nothing read from Live.
    let again = store(dir.path());
    assert_eq!(again.markers(), found);
    assert_eq!(again.markers_status().found, 2);
    assert_eq!(again.poll(), Some(1));
    assert_eq!(labels(&again), vec!["Vox", "Key"]);
    // A change is remembered too, the empty set included.
    assert_eq!(again.set_markers(vec![found[1].clone()]), Some(2));
    assert_eq!(store(dir.path()).markers(), vec![found[1].clone()]);
    assert_eq!(again.set_markers(Vec::new()), Some(3));
    assert_eq!(store(dir.path()).markers(), Vec::<Found>::new());
    // A store without a file of its own remembers nothing.
    let plain = LayoutStore::new(
        dir.path().join("layout.json"),
        dir.path().join(BACKUP_DIR),
        vec!["band".into()],
    );
    assert_eq!(plain.markers(), Vec::<Found>::new());
    let _ = plain.set_markers(found);
    assert_eq!(store(dir.path()).markers(), Vec::<Found>::new());
}

#[test]
fn the_same_markers_leave_the_remembered_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let first = store(dir.path());
    let found = vec![marker("band", 1, r#""Vox" +G:A"#)];
    let _ = first.set_markers(found.clone());
    let file = dir.path().join(MARKERS_FILE);
    std::fs::write(&file, "[]").unwrap();
    // Equal markers: no write (the file's hand-set content stays).
    let _ = first.set_markers(found);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
}

#[test]
fn a_missing_or_bad_remembered_file_is_no_markers() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(store(dir.path()).markers(), Vec::<Found>::new());
    std::fs::write(dir.path().join(MARKERS_FILE), "not json").unwrap();
    assert_eq!(store(dir.path()).markers(), Vec::<Found>::new());
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let unreadable = LayoutStore::new(
        dir.path().join("layout.json"),
        dir.path().join(BACKUP_DIR),
        vec!["band".into()],
    )
    .remembering(dir.path().join("sub"));
    assert_eq!(unreadable.markers(), Vec::<Found>::new());
    // A file that cannot be written is logged; the markers still serve.
    write(dir.path(), &tags_frame());
    assert_eq!(unreadable.poll(), Some(1));
    assert_eq!(
        unreadable.set_markers(vec![marker("band", 1, r#""Vox" +G:A"#)]),
        Some(2)
    );
    assert_eq!(labels(&unreadable), vec!["Vox"]);
}

#[test]
fn a_live_not_read_since_the_start_keeps_its_remembered_markers() {
    let before = vec![
        marker("band", 1, "b1"),
        marker("band", 3, "b3"),
        marker("master", 0, "m0"),
    ];
    let unread = |names: &[&str]| -> std::collections::BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    };
    // The band read first, the master still down: the master's kept.
    assert_eq!(
        kept_markers(vec![marker("band", 2, "b2")], &before, &unread(&["master"])),
        vec![marker("band", 2, "b2"), marker("master", 0, "m0")]
    );
    // Both read: only what the keeper found (an empty set too).
    assert_eq!(
        kept_markers(vec![marker("band", 2, "b2")], &before, &unread(&[])),
        vec![marker("band", 2, "b2")]
    );
    assert_eq!(
        kept_markers(Vec::new(), &before, &unread(&[])),
        Vec::<Found>::new()
    );
    // Neither read: everything remembered, in order.
    assert_eq!(
        kept_markers(Vec::new(), &before, &unread(&["band", "master"])),
        before
    );
    // A Live the keeper found something of keeps only what it found.
    assert_eq!(
        kept_markers(
            vec![marker("master", 4, "m4"), marker("band", 0, "b0")],
            &before,
            &unread(&["band", "master"])
        ),
        vec![marker("band", 0, "b0"), marker("master", 4, "m4")]
    );
}

#[test]
fn a_live_no_longer_configured_keeps_no_remembered_markers() {
    let dir = tempfile::tempdir().unwrap();
    let first = store(dir.path());
    let _ = first.set_markers(vec![marker("band", 1, "b"), marker("master", 2, "m")]);
    let band_only = store_in(dir.path(), Path::new("layout.json"), vec!["band".into()]);
    assert_eq!(band_only.markers(), vec![marker("band", 1, "b")]);
}

#[test]
fn only_a_missing_file_is_no_read_problem() {
    use std::io::{Error, ErrorKind};
    assert!(read_problem(Error::from(ErrorKind::NotFound)).is_none());
    let denied = read_problem(Error::from(ErrorKind::PermissionDenied));
    assert_eq!(denied.map(|e| e.kind()), Some(ErrorKind::PermissionDenied));
}
