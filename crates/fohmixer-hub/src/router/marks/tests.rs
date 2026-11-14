use std::path::Path;
use std::sync::Arc;

use fohmixer_proto::client::{MuteAlarmsStatus, ServerMsg};
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::strip_marks::StripMarks;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::tests::{attach, offline_router, records, status};
use super::super::{Router, RouterMsg};
use super::{
    mark_fields, refused_line, restored_line, set_line, taken_line, unreadable_line,
    unwritable_line, write_problem,
};
use crate::events::{is_warn, stamp};
use crate::mute_alarms::MUTE_ALARMS_FILE;
use crate::outbox::Outbox;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn doc(labels: &[&str]) -> MuteAlarmsDoc {
    labels.iter().fold(MuteAlarmsDoc::empty(), |doc, label| {
        doc.with(&band(label), true)
    })
}

fn write_doc(dir: &Path, doc: &MuteAlarmsDoc) {
    std::fs::write(
        dir.join(MUTE_ALARMS_FILE),
        serde_json::to_string(doc).unwrap(),
    )
    .unwrap();
}

fn on_disk(dir: &Path) -> MuteAlarmsDoc {
    serde_json::from_slice(&std::fs::read(dir.join(MUTE_ALARMS_FILE)).unwrap()).unwrap()
}

/// A router over the offline band serving the synthetic sectors fixture
/// (`tests/fixtures/sectors-layout.json`: Vox 1..4, Klavir, Bass, Keys 1
/// and PA Out, band tracks by name), with `doc` as `mute_alarms.json`
/// when given.
fn served(
    dir: &Path,
    alarms: Option<&MuteAlarmsDoc>,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(alarms) = alarms {
        write_doc(dir, alarms);
    }
    let store = Arc::new(crate::layout::store_in(
        dir,
        Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, own, rx) = offline_router(dir);
    (router.with_layout(store), own, rx)
}

/// The `mute_alarms` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<MuteAlarmsDoc> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::MuteAlarms { doc } => Some(doc),
            _ => None,
        })
        .collect()
}

/// The `mute_alarms` records waiting, without their time.
fn alarm_records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "mute_alarms")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect()
}

fn set(client: u64, id: &str, label: &str, on: bool) -> RouterMsg {
    RouterMsg::SetMuteAlarm {
        client,
        id: id.into(),
        strip: band(label),
        on,
    }
}

fn result(id: &str) -> ServerMsg {
    ServerMsg::Result {
        id: id.into(),
        data: Vec::new(),
    }
}

fn error(id: &str, message: &str) -> ServerMsg {
    ServerMsg::Error {
        id: Some(id.into()),
        message: message.into(),
    }
}

#[tokio::test]
async fn a_page_gets_the_document_at_attach() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![first]);
    let empty = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(empty.path(), None);
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![MuteAlarmsDoc::empty()]);
}

#[tokio::test]
async fn a_switch_is_answered_recorded_and_told_to_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(set(1, "a1", "Bass", true));
    let after = doc(&["Bass"]);
    assert_eq!(
        a.take().unwrap(),
        vec![result("a1"), ServerMsg::MuteAlarms { doc: after.clone() }]
    );
    assert_eq!(told(&b), vec![after.clone()]);
    assert_eq!(on_disk(dir.path()), after);
    assert_eq!(
        alarm_records(&rx),
        vec![
            json!({"ev": "mute_alarms", "what": "set", "client": 1, "peer": "10.0.0.1",
                   "strip": {"instance": "band", "label": "Bass"}, "on": true})
        ]
    );
    // Off: the mark goes, `false` in the record.
    router.handle(set(2, "a2", "Bass", false));
    assert_eq!(b.take().unwrap()[0], result("a2"));
    assert_eq!(told(&a), vec![MuteAlarmsDoc::empty()]);
    assert_eq!(on_disk(dir.path()), MuteAlarmsDoc::empty());
    assert_eq!(alarm_records(&rx)[0]["on"], json!(false));
    // A switch that changes nothing: its result only.
    router.handle(set(1, "a3", "Bass", false));
    assert_eq!(a.take().unwrap(), vec![result("a3")]);
    assert_eq!(told(&b), vec![]);
    assert_eq!(alarm_records(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn a_mark_for_a_fader_not_in_the_pool_is_refused_to_the_asker_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(set(1, "a1", "Nobody", true));
    assert_eq!(a.take().unwrap(), vec![error("a1", "not in the pool")]);
    assert_eq!(b.take().unwrap(), vec![]);
    assert_eq!(alarm_records(&rx), Vec::<Value>::new());
    assert!(
        !dir.path().join(MUTE_ALARMS_FILE).exists(),
        "nothing written"
    );
    // With no layout served nothing is in the pool.
    let bare = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(bare.path());
    let page = attach(&mut router, 1);
    page.take();
    router.handle(set(1, "a2", "Bass", true));
    assert_eq!(page.take().unwrap(), vec![error("a2", "not in the pool")]);
}

#[tokio::test]
async fn a_hand_edit_is_told_and_recorded_file_and_a_bad_one_shows_in_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(told(&a), vec![], "the file as served");
    let edited = doc(&["Klavir"]);
    write_doc(dir.path(), &edited);
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(told(&a), vec![edited.clone()]);
    assert_eq!(
        alarm_records(&rx),
        vec![json!({"ev": "mute_alarms", "what": "file", "client": null, "peer": null})]
    );
    std::fs::write(dir.path().join(MUTE_ALARMS_FILE), "{").unwrap();
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(told(&a), vec![]);
    assert_eq!(alarm_records(&rx), Vec::<Value>::new());
    let now = status(&mut router).mute_alarms;
    assert_eq!(now.strips, 1, "the last good document stays");
    assert_eq!(
        now.error,
        Some(crate::mute_alarms::parse(b"{").unwrap_err())
    );
}

#[tokio::test]
async fn a_removed_file_is_written_back_and_recorded_once() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    std::fs::remove_file(dir.path().join(MUTE_ALARMS_FILE)).unwrap();
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(on_disk(dir.path()), first);
    assert_eq!(told(&a), vec![], "the document did not change");
    assert_eq!(
        alarm_records(&rx),
        vec![
            json!({"ev": "mute_alarms", "what": "file", "client": null, "peer": null,
                   "why": "deleted"})
        ]
    );
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(alarm_records(&rx), Vec::<Value>::new(), "once");
}

#[tokio::test]
async fn the_status_counts_the_marks_and_those_no_pool_fader_carries() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1", "Renamed"]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    // A page attached (#184: the router acts for attached clients by role).
    attach(&mut router, 1);
    assert_eq!(
        status(&mut router).mute_alarms,
        MuteAlarmsStatus {
            strips: 2,
            unknown: 1,
            error: None,
        }
    );
    router.handle(set(1, "a1", "Bass", true));
    let now = status(&mut router).mute_alarms;
    assert_eq!((now.strips, now.unknown), (3, 1));
}

#[tokio::test]
async fn a_write_that_fails_is_an_error_to_the_asker_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    // The file's temporary name is taken by a folder: its write fails.
    std::fs::create_dir(dir.path().join("mute_alarms.tmp")).unwrap();
    std::fs::write(dir.path().join("mute_alarms.tmp").join("x"), "x").unwrap();
    router.handle(set(1, "a1", "Bass", true));
    let got = a.take().unwrap();
    assert!(
        matches!(&got[..], [ServerMsg::Error { id: Some(id), message }] if id == "a1" && !message.is_empty()),
        "{got:?}"
    );
    assert_eq!(b.take().unwrap(), vec![], "nothing told");
    assert_eq!(alarm_records(&rx), Vec::<Value>::new());
    assert_eq!(router.mute_alarms.store.doc(), &first);
    assert_eq!(on_disk(dir.path()), first);
}

#[tokio::test]
async fn the_colours_and_the_marks_are_separate_documents() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    a.take();
    router.handle(set(1, "a1", "Bass", true));
    assert!(!dir.path().join(crate::colors::COLORS_FILE).exists());
    assert_eq!(
        router.colors.store.doc(),
        &fohmixer_proto::colors::ColorsDoc::empty()
    );
}

#[test]
fn a_mute_alarms_record_is_warn_class() {
    assert!(is_warn(&stamp(
        "mute_alarms",
        1,
        mark_fields("set", Some(1), Some("10.0.0.1"), json!({"on": true}))
    )));
    assert!(is_warn(&stamp(
        "mute_alarms",
        1,
        mark_fields("file", None, None, json!({}))
    )));
}

#[test]
fn the_records_and_lines_read_as_written() {
    assert_eq!(
        mark_fields("set", Some(3), Some("10.0.0.3"), json!({"on": true})),
        json!({"what": "set", "client": 3, "peer": "10.0.0.3", "on": true})
    );
    assert_eq!(
        mark_fields("file", None, None, json!({})),
        json!({"what": "file", "client": null, "peer": null})
    );
    assert_eq!(
        set_line::<MuteAlarmsDoc>(&band("Vox 1"), true),
        "a strip's mute alarm changed: band|Vox 1 is on"
    );
    assert_eq!(
        set_line::<MuteAlarmsDoc>(&band("Vox 1"), false),
        "a strip's mute alarm changed: band|Vox 1 is off"
    );
    assert_eq!(
        taken_line::<MuteAlarmsDoc>(),
        "mute_alarms.json was edited by hand: its marks are served"
    );
    assert_eq!(
        restored_line::<MuteAlarmsDoc>(),
        "mute_alarms.json was removed; it is written back from the document served"
    );
    assert_eq!(
        refused_line::<MuteAlarmsDoc>("bad"),
        "mute_alarms.json is refused, the last good document stays served: bad"
    );
    assert_eq!(
        write_problem::<MuteAlarmsDoc>(&std::io::Error::other("disk full")),
        "cannot write the mute alarm's marks (mute_alarms.json): disk full"
    );
    assert_eq!(
        unreadable_line::<MuteAlarmsDoc>("Is a directory"),
        "cannot read mute_alarms.json, the document stays served and is read again at the next poll: Is a directory"
    );
    assert_eq!(
        unwritable_line::<MuteAlarmsDoc>("No such file or directory"),
        "mute_alarms.json was removed and cannot be written back, the document stays served and is written again at the next poll: No such file or directory"
    );
}

#[tokio::test]
async fn an_mbc_mark_for_a_band_strip_is_refused_with_role() {
    // #184: an MBC login reaches only the Live set mbc's faders.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let mbc = super::super::tests::attach_as(&mut router, 1, fohmixer_proto::roles::Role::Mbc);
    mbc.take();
    let _ = records(&rx);
    router.handle(set(1, "a1", "Bass", true));
    assert_eq!(mbc.take().unwrap(), vec![error("a1", "role")]);
    assert_eq!(alarm_records(&rx), Vec::<Value>::new());
    assert!(
        !dir.path().join(MUTE_ALARMS_FILE).exists(),
        "nothing written"
    );
    let engineer = attach(&mut router, 2);
    engineer.take();
    router.handle(set(2, "a2", "Bass", true));
    assert_eq!(engineer.take().unwrap()[0], result("a2"));
}
