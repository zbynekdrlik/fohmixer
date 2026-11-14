//! The mute guard's marks (#203) through the shared glue: its own file,
//! message, records and status, apart from the mute alarm's.

use std::path::Path;
use std::sync::Arc;

use fohmixer_proto::client::{MuteGuardsStatus, ServerMsg};
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
use fohmixer_proto::mute_guards::MuteGuardsDoc;
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::strip_marks::StripMarks;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::tests::{attach, records, status};
use super::super::{Router, RouterMsg};
use super::{
    refused_line, restored_line, set_line, taken_line, unreadable_line, unwritable_line,
    write_problem,
};
use crate::events::{is_warn, stamp};
use crate::mute_guards::MUTE_GUARDS_FILE;
use crate::outbox::Outbox;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn doc(labels: &[&str]) -> MuteGuardsDoc {
    labels.iter().fold(MuteGuardsDoc::empty(), |doc, label| {
        doc.with(&band(label), true)
    })
}

fn write_doc(dir: &Path, doc: &MuteGuardsDoc) {
    std::fs::write(
        dir.join(MUTE_GUARDS_FILE),
        serde_json::to_string(doc).unwrap(),
    )
    .unwrap();
}

fn on_disk(dir: &Path) -> MuteGuardsDoc {
    serde_json::from_slice(&std::fs::read(dir.join(MUTE_GUARDS_FILE)).unwrap()).unwrap()
}

/// A router serving the synthetic sectors fixture (Vox 1..4, Klavir, Bass,
/// Keys 1 and PA Out), with `guards` as `mute_guards.json` when given.
fn served(
    dir: &Path,
    guards: Option<&MuteGuardsDoc>,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(guards) = guards {
        write_doc(dir, guards);
    }
    let store = Arc::new(crate::layout::store_in(
        dir,
        Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, own, rx) = super::super::tests::offline_router(dir);
    (router.with_layout(store), own, rx)
}

/// The `mute_guards` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<MuteGuardsDoc> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::MuteGuards { doc } => Some(doc),
            _ => None,
        })
        .collect()
}

/// The `mute_guards` records waiting, without their time.
fn guard_records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "mute_guards")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect()
}

fn set(client: u64, id: &str, label: &str, on: bool) -> RouterMsg {
    RouterMsg::SetMuteGuard {
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

#[tokio::test]
async fn a_page_gets_the_guards_after_the_alarms_at_attach() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    let page = attach(&mut router, 1);
    let got = page.take().unwrap();
    let alarms = got
        .iter()
        .position(|m| matches!(m, ServerMsg::MuteAlarms { .. }))
        .expect("the alarms");
    assert_eq!(
        got.get(alarms + 1),
        Some(&ServerMsg::MuteGuards { doc: first }),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_switch_is_answered_recorded_written_and_told_to_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(set(1, "g1", "Bass", true));
    let after = doc(&["Bass"]);
    assert_eq!(
        a.take().unwrap(),
        vec![result("g1"), ServerMsg::MuteGuards { doc: after.clone() }]
    );
    assert_eq!(told(&b), vec![after.clone()]);
    assert_eq!(on_disk(dir.path()), after);
    assert_eq!(
        guard_records(&rx),
        vec![
            json!({"ev": "mute_guards", "what": "set", "client": 1, "peer": "10.0.0.1",
                   "strip": {"instance": "band", "label": "Bass"}, "on": true})
        ]
    );
    // The alarm's document is another one.
    assert!(
        !dir.path()
            .join(crate::mute_alarms::MUTE_ALARMS_FILE)
            .exists()
    );
    assert_eq!(router.mute_alarms.store.doc(), &MuteAlarmsDoc::empty());
    router.handle(set(2, "g2", "Bass", false));
    assert_eq!(b.take().unwrap()[0], result("g2"));
    assert_eq!(told(&a), vec![MuteGuardsDoc::empty()]);
    assert_eq!(on_disk(dir.path()), MuteGuardsDoc::empty());
    assert_eq!(guard_records(&rx)[0]["on"], json!(false));
}

#[tokio::test]
async fn a_guard_for_a_fader_not_in_the_pool_or_another_live_set_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(set(1, "g1", "Nobody", true));
    assert_eq!(
        a.take().unwrap(),
        vec![ServerMsg::Error {
            id: Some("g1".into()),
            message: "not in the pool".into(),
        }]
    );
    let mbc = super::super::tests::attach_as(&mut router, 2, fohmixer_proto::roles::Role::Mbc);
    mbc.take();
    router.handle(set(2, "g2", "Bass", true));
    assert_eq!(
        mbc.take().unwrap(),
        vec![ServerMsg::Error {
            id: Some("g2".into()),
            message: "role".into(),
        }]
    );
    assert_eq!(guard_records(&rx), Vec::<Value>::new());
    assert!(
        !dir.path().join(MUTE_GUARDS_FILE).exists(),
        "nothing written"
    );
}

#[tokio::test]
async fn a_hand_edit_and_a_removed_file_go_through_the_guards_poll() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1"]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    let edited = doc(&["Klavir"]);
    write_doc(dir.path(), &edited);
    // The alarm's poll does not read the guards' file.
    router.handle(RouterMsg::MuteAlarmsPoll);
    assert_eq!(told(&a), vec![]);
    router.handle(RouterMsg::MuteGuardsPoll);
    assert_eq!(told(&a), vec![edited.clone()]);
    assert_eq!(
        guard_records(&rx),
        vec![json!({"ev": "mute_guards", "what": "file", "client": null, "peer": null})]
    );
    std::fs::remove_file(dir.path().join(MUTE_GUARDS_FILE)).unwrap();
    router.handle(RouterMsg::MuteGuardsPoll);
    assert_eq!(on_disk(dir.path()), edited);
    assert_eq!(
        guard_records(&rx),
        vec![
            json!({"ev": "mute_guards", "what": "file", "client": null, "peer": null,
                   "why": "deleted"})
        ]
    );
}

#[tokio::test]
async fn the_status_counts_the_guards_apart_from_the_alarms() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&["Vox 1", "Renamed"]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    attach(&mut router, 1);
    let now = status(&mut router);
    assert_eq!(
        now.mute_guards,
        MuteGuardsStatus {
            strips: 2,
            unknown: 1,
            error: None,
        }
    );
    assert_eq!(now.mute_alarms.strips, 0);
    std::fs::write(dir.path().join(MUTE_GUARDS_FILE), "{").unwrap();
    router.handle(RouterMsg::MuteGuardsPoll);
    let now = status(&mut router).mute_guards;
    assert_eq!(now.strips, 2, "the last good document stays");
    assert_eq!(
        now.error,
        Some(crate::mute_guards::parse(b"{").unwrap_err())
    );
}

#[test]
fn a_mute_guards_record_is_warn_class() {
    assert!(is_warn(&stamp(
        "mute_guards",
        1,
        super::mark_fields("set", Some(1), None, json!({"on": true}))
    )));
}

#[test]
fn the_guards_lines_name_the_guard_and_its_file() {
    assert_eq!(
        set_line::<MuteGuardsDoc>(&band("Vox 1"), true),
        "a strip's mute guard changed: band|Vox 1 is on"
    );
    assert_eq!(
        taken_line::<MuteGuardsDoc>(),
        "mute_guards.json was edited by hand: its marks are served"
    );
    assert_eq!(
        restored_line::<MuteGuardsDoc>(),
        "mute_guards.json was removed; it is written back from the document served"
    );
    assert_eq!(
        refused_line::<MuteGuardsDoc>("bad"),
        "mute_guards.json is refused, the last good document stays served: bad"
    );
    assert_eq!(
        write_problem::<MuteGuardsDoc>(&std::io::Error::other("disk full")),
        "cannot write the mute guard's marks (mute_guards.json): disk full"
    );
    assert_eq!(
        unreadable_line::<MuteGuardsDoc>("x"),
        "cannot read mute_guards.json, the document stays served and is read again at the next poll: x"
    );
    assert_eq!(
        unwritable_line::<MuteGuardsDoc>("y"),
        "mute_guards.json was removed and cannot be written back, the document stays served and is written again at the next poll: y"
    );
}
