use std::path::Path;
use std::sync::Arc;

use fohmixer_proto::client::{SectorsStatus, ServerMsg};
use fohmixer_proto::sectors::{SectorId, SectorLayout, SectorOrder, SectorsDoc, StripRef};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::reverb::tests::{answer, next_read, read_answered, settle, slot};
use super::super::tests::{attach, offline_router, records, status};
use super::super::{Router, RouterMsg};
use super::{refused_line, sectors_fields, sectors_line, seed_due, unreadable_line, write_problem};
use crate::events::{is_warn, stamp};
use crate::outbox::Outbox;
use crate::sectors::{PREV_FILE, SECTORS_FILE};

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn layout(id: &str, title: &str, labels: &[&str]) -> SectorLayout {
    SectorLayout {
        id: id.into(),
        title: title.into(),
        strips: labels.iter().map(|l| band(l)).collect(),
    }
}

fn doc(rev: u64, sectors: Vec<(SectorId, Vec<SectorLayout>)>) -> SectorsDoc {
    SectorsDoc {
        rev,
        sectors: sectors.into_iter().collect(),
    }
}

fn write_doc(dir: &Path, doc: &SectorsDoc) {
    std::fs::write(dir.join(SECTORS_FILE), serde_json::to_string(doc).unwrap()).unwrap();
}

/// A router over the offline band serving the synthetic sectors fixture
/// (`tests/fixtures/sectors-layout.json`: Vox 1..4, Klavir, Bass, Keys 1
/// and the wide PA Out, all band tracks by name), with `doc` as
/// `sectors.json` when given.
fn served(
    dir: &Path,
    sectors: Option<&SectorsDoc>,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(sectors) = sectors {
        write_doc(dir, sectors);
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

/// The `sectors` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<(SectorsDoc, bool)> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Sectors { doc, undo } => Some((doc, undo)),
            _ => None,
        })
        .collect()
}

/// The answers (`result`, `error`) waiting in `outbox`, besides the rest.
fn answers(outbox: &Outbox) -> Vec<ServerMsg> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| matches!(m, ServerMsg::Result { .. } | ServerMsg::Error { .. }))
        .collect()
}

/// The `sectors` records waiting, without their time.
fn sector_records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "sectors")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect()
}

fn tick(client: u64, id: &str, layout: &str, label: &str, on: bool) -> RouterMsg {
    RouterMsg::SectorTick {
        client,
        id: id.into(),
        layout: layout.into(),
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

fn on_disk(dir: &Path, name: &str) -> SectorsDoc {
    serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap()
}

#[tokio::test]
async fn a_page_gets_the_document_at_attach() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        3,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![(first, false)]);
    // An empty data folder: the empty document, sectors off.
    let empty = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(empty.path(), None);
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![(SectorsDoc::empty(), false)]);
}

#[tokio::test]
async fn a_tick_is_answered_recorded_and_told_to_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        1,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(tick(1, "t1", "lu-1", "Bass", true));
    let after = doc(
        2,
        vec![(
            SectorId::Lu,
            vec![layout("lu-1", "VOX", &["Vox 1", "Bass"])],
        )],
    );
    assert_eq!(
        a.take().unwrap(),
        vec![
            result("t1"),
            ServerMsg::Sectors {
                doc: after.clone(),
                undo: true
            }
        ]
    );
    assert_eq!(told(&b), vec![(after.clone(), true)]);
    assert_eq!(on_disk(dir.path(), SECTORS_FILE), after);
    assert_eq!(on_disk(dir.path(), PREV_FILE), first);
    assert_eq!(
        sector_records(&rx),
        vec![
            json!({"ev": "sectors", "what": "tick", "client": 1, "peer": "10.0.0.1",
                    "rev": 2, "layout": "lu-1",
                    "strip": {"instance": "band", "label": "Bass"}, "on": true})
        ]
    );
    // Off again: the strip goes.
    router.handle(tick(2, "t2", "lu-1", "Vox 1", false));
    assert_eq!(answers(&b), vec![result("t2")]);
    assert_eq!(
        told(&a),
        vec![(
            doc(
                3,
                vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Bass"])])]
            ),
            true
        )]
    );
    assert_eq!(sector_records(&rx).len(), 1, "the tick off");
    // A tick that changes nothing: its result only.
    router.handle(tick(1, "t3", "lu-1", "Bass", true));
    assert_eq!(a.take().unwrap(), vec![result("t3")]);
    assert_eq!(told(&b), vec![]);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
    assert_eq!(router.sectors.store.doc().rev, 3);
}

#[tokio::test]
async fn each_refusal_answers_the_asker_alone_and_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let full = &["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Klavir", "Keys 1"];
    let first = doc(1, vec![(SectorId::Lu, vec![layout("lu-1", "VOX", full)])]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(tick(1, "t1", "x", "Bass", true));
    router.handle(tick(1, "t2", "lu-1", "Nobody", true));
    router.handle(tick(1, "t3", "lu-1", "Bass", true));
    assert_eq!(
        a.take().unwrap(),
        vec![
            error("t1", "unknown layout"),
            error("t2", "not in the pool"),
            error("t3", "FULL"),
        ]
    );
    assert_eq!(b.take().unwrap(), vec![]);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
    assert_eq!(router.sectors.store.doc(), &first);
    assert!(!dir.path().join(PREV_FILE).exists(), "nothing written");
}

#[tokio::test]
async fn two_ticks_into_the_last_unit_take_one_and_refuse_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let five = &["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Klavir"];
    let first = doc(1, vec![(SectorId::Lu, vec![layout("lu-1", "VOX", five)])]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    router.handle(tick(1, "a1", "lu-1", "Bass", true));
    router.handle(tick(2, "b1", "lu-1", "Keys 1", true));
    assert_eq!(answers(&a), vec![result("a1")]);
    assert_eq!(answers(&b), vec![error("b1", "FULL")]);
    let strips = &router.sectors.store.doc().layouts(SectorId::Lu)[0].strips;
    assert_eq!(strips.len(), 6);
    assert_eq!(strips.last(), Some(&band("Bass")));
}

#[tokio::test]
async fn an_order_with_an_old_rev_is_refused_stale() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        4,
        vec![(
            SectorId::Lu,
            vec![layout("lu-1", "VOX", &["Vox 1", "Vox 2"])],
        )],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(RouterMsg::SectorOrders {
        client: 1,
        id: "o1".into(),
        orders: vec![SectorOrder {
            layout: "lu-1".into(),
            strips: vec![band("Vox 2"), band("Vox 1")],
        }],
        rev: 3,
    });
    assert_eq!(a.take().unwrap(), vec![error("o1", "stale")]);
    assert_eq!(router.sectors.store.doc(), &first);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn an_order_of_no_layout_is_refused_empty_and_keeps_the_undo() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        4,
        vec![(
            SectorId::Lu,
            vec![layout("lu-1", "VOX", &["Vox 1", "Vox 2"])],
        )],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    router.handle(tick(1, "t1", "lu-1", "Bass", true));
    a.take();
    let _ = records(&rx);
    let taken = router.sectors.store.doc().clone();
    router.handle(RouterMsg::SectorOrders {
        client: 1,
        id: "o1".into(),
        orders: Vec::new(),
        rev: taken.rev,
    });
    assert_eq!(a.take().unwrap(), vec![error("o1", "EMPTY")]);
    assert_eq!(router.sectors.store.doc(), &taken);
    assert_eq!(on_disk(dir.path(), PREV_FILE), first, "the undo step kept");
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn orders_of_two_layouts_are_one_change_and_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        1,
        vec![
            (
                SectorId::Lu,
                vec![layout("lu-1", "VOX", &["Vox 1", "Vox 2"])],
            ),
            (
                SectorId::Ru,
                vec![layout("ru-1", "BAND", &["Klavir", "Bass"])],
            ),
        ],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(RouterMsg::SectorOrders {
        client: 1,
        id: "o1".into(),
        orders: vec![
            SectorOrder {
                layout: "lu-1".into(),
                strips: vec![band("Vox 2"), band("Vox 1")],
            },
            SectorOrder {
                layout: "ru-1".into(),
                strips: vec![band("Bass"), band("Klavir")],
            },
        ],
        rev: 1,
    });
    let arranged = doc(
        2,
        vec![
            (
                SectorId::Lu,
                vec![layout("lu-1", "VOX", &["Vox 2", "Vox 1"])],
            ),
            (
                SectorId::Ru,
                vec![layout("ru-1", "BAND", &["Bass", "Klavir"])],
            ),
        ],
    );
    assert_eq!(
        a.take().unwrap(),
        vec![
            result("o1"),
            ServerMsg::Sectors {
                doc: arranged.clone(),
                undo: true
            }
        ]
    );
    assert_eq!(told(&b), vec![(arranged.clone(), true)]);
    router.handle(RouterMsg::SectorUndo {
        client: 2,
        id: "u1".into(),
    });
    let mut back = first.clone();
    back.rev = 3;
    assert_eq!(answers(&b), vec![result("u1")]);
    assert_eq!(told(&a), vec![(back.clone(), true)]);
    assert_eq!(on_disk(dir.path(), SECTORS_FILE), back);
    assert_eq!(
        sector_records(&rx),
        vec![
            json!({"ev": "sectors", "what": "order", "client": 1, "peer": "10.0.0.1",
                   "rev": 2, "layouts": ["lu-1", "ru-1"]}),
            json!({"ev": "sectors", "what": "undo", "client": 2, "peer": "10.0.0.2",
                   "rev": 3}),
        ]
    );
}

#[tokio::test]
async fn undo_without_prev_is_refused_nothing_to_undo() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(1, vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &[])])]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(RouterMsg::SectorUndo {
        client: 1,
        id: "u1".into(),
    });
    assert_eq!(a.take().unwrap(), vec![error("u1", "nothing to undo")]);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn a_hand_edit_is_told_to_every_page_and_recorded_file() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        4,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(RouterMsg::SectorsPoll);
    assert_eq!(told(&a), vec![], "the file as served");
    let edited = doc(
        1,
        vec![(SectorId::Ld, vec![layout("ld-1", "KEYS", &["Klavir"])])],
    );
    write_doc(dir.path(), &edited);
    router.handle(RouterMsg::SectorsPoll);
    let mut served_doc = edited.clone();
    served_doc.rev = 5;
    assert_eq!(told(&a), vec![(served_doc, true)]);
    assert_eq!(
        sector_records(&rx),
        vec![
            json!({"ev": "sectors", "what": "file", "client": null, "peer": null,
                    "rev": 5})
        ]
    );
    // A bad edit: nothing told, nothing recorded, the status says why.
    std::fs::write(dir.path().join(SECTORS_FILE), "{").unwrap();
    router.handle(RouterMsg::SectorsPoll);
    assert_eq!(told(&a), vec![]);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
    let now = status(&mut router).sectors;
    assert_eq!(now.rev, 5);
    assert_eq!(
        now.error,
        Some(crate::sectors::parse(b"{").unwrap_err()),
        "the last good document stays"
    );
}

#[tokio::test]
async fn a_removed_file_is_written_back_and_recorded_never_seeded() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        4,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    router.handle(tick(1, "t1", "lu-1", "Bass", true));
    a.take();
    let _ = records(&rx);
    let taken = router.sectors.store.doc().clone();
    std::fs::remove_file(dir.path().join(SECTORS_FILE)).unwrap();
    router.handle(RouterMsg::SectorsPoll);
    assert_eq!(on_disk(dir.path(), SECTORS_FILE), taken);
    assert_eq!(on_disk(dir.path(), PREV_FILE), first, "the undo kept");
    assert_eq!(router.sectors.store.doc(), &taken);
    assert!(!router.sectors.store.missing());
    assert_eq!(told(&a), vec![], "the document did not change");
    assert_eq!(
        sector_records(&rx),
        vec![
            json!({"ev": "sectors", "what": "file", "client": null, "peer": null,
                   "rev": 5, "why": "deleted"})
        ]
    );
    router.handle(RouterMsg::SectorsPoll);
    assert_eq!(sector_records(&rx), Vec::<Value>::new(), "once");
}

/// A frame of one page `p`: the tag group FX, then an untitled group of
/// one band strip by name, Klavir.
fn tagged_frame() -> Value {
    json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [
            {"kind": "group", "id": "fx", "title": "FX", "tags": "FX"},
            {"kind": "group", "id": "own", "controls": [
                {"kind": "strip", "strip_kind": "standard",
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Klavir"}}}]}]}]}]})
}

#[tokio::test]
async fn the_seed_waits_for_every_instances_marker_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("layout.json"), tagged_frame().to_string()).unwrap();
    let store = Arc::new(crate::layout::store_in(
        dir.path(),
        Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, mut own, rx) = offline_router(dir.path());
    let mut router = router.with_layout(store);
    let page = attach(&mut router, 1);
    page.take();
    let file = dir.path().join(SECTORS_FILE);
    // The frame's own layout is served: its pool holds Klavir, yet no
    // instance was read.
    router.handle(RouterMsg::SectorsPoll);
    assert!(!file.exists(), "before any read");
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "return_tracks") => slot("live_1.returns", json!([])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let (seq, _) = next_read(&mut own).await;
    assert!(!file.exists(), "the read runs");
    read_answered(
        &mut router,
        &mut own,
        seq,
        2,
        &[(0, r#""Rev ST" +G:FX"#), (1, r#""Rev PA" +G:FX"#)],
    )
    .await;
    assert!(
        !file.exists(),
        "read, but its composition waits for the quiet time"
    );
    assert!(router.sectors.store.missing());
    let _ = records(&rx);
    settle(&mut router, &mut own).await;
    assert!(file.exists());
    let seeded = doc(
        1,
        vec![
            (
                SectorId::Lu,
                vec![layout("lu-1", "FX", &["Rev ST", "Rev PA"])],
            ),
            (SectorId::Ld, vec![layout("ld-1", "MAIN", &[])]),
            (SectorId::Ru, vec![layout("ru-1", "MAIN", &["Klavir"])]),
            (SectorId::Rd, vec![layout("rd-1", "MAIN", &[])]),
        ],
    );
    assert_eq!(router.sectors.store.doc(), &seeded);
    assert_eq!(on_disk(dir.path(), SECTORS_FILE), seeded);
    assert_eq!(told(&page), vec![(seeded, false)]);
    assert_eq!(
        sector_records(&rx),
        vec![
            json!({"ev": "sectors", "what": "seed", "client": null, "peer": null,
                    "rev": 1})
        ]
    );
    // Seeded once: a later layout seeds nothing.
    router.handle(RouterMsg::SectorsPoll);
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
}

#[test]
fn seed_due_needs_a_missing_file_a_pool_and_every_read() {
    assert!(seed_due(true, 1, true));
    assert!(!seed_due(false, 1, true), "a file is there");
    assert!(!seed_due(true, 0, true), "no fader served");
    assert!(!seed_due(true, 1, false), "an instance not read yet");
    assert!(!seed_due(false, 0, false));
}

#[test]
fn a_sectors_record_is_warn_class() {
    assert!(is_warn(&stamp(
        "sectors",
        1,
        sectors_fields("tick", Some(1), Some("10.0.0.1"), json!({"rev": 2}))
    )));
    assert!(is_warn(&stamp(
        "sectors",
        1,
        sectors_fields("seed", None, None, json!({}))
    )));
}

#[test]
fn the_records_and_lines_read_as_written() {
    assert_eq!(
        sectors_fields(
            "order",
            Some(3),
            Some("10.0.0.3"),
            json!({"rev": 7, "layouts": ["lu-1"]})
        ),
        json!({"what": "order", "client": 3, "peer": "10.0.0.3", "rev": 7, "layouts": ["lu-1"]})
    );
    assert_eq!(
        sectors_fields("file", None, None, json!({"rev": 2})),
        json!({"what": "file", "client": null, "peer": null, "rev": 2})
    );
    assert_eq!(
        sectors_line("tick", 4),
        "the fader sectors changed (tick): revision 4"
    );
    assert_eq!(
        refused_line("bad"),
        "sectors.json is refused, the last good document stays served: bad"
    );
    assert_eq!(
        write_problem(&std::io::Error::other("disk full")),
        "cannot write the fader sectors (sectors.json): disk full"
    );
}

#[test]
fn the_unreadable_line_reads_as_written() {
    assert_eq!(
        unreadable_line("Is a directory"),
        "cannot read sectors.json, the document stays served and is read again at the next poll: Is a directory"
    );
}

#[tokio::test]
async fn the_status_counts_layouts_strips_and_unknown_refs() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        6,
        vec![(
            SectorId::Lu,
            vec![
                layout("lu-1", "VOX", &["Vox 1", "Renamed"]),
                layout("lu-2", "ALL", &["Vox 1", "Bass"]),
            ],
        )],
    );
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    assert_eq!(
        status(&mut router).sectors,
        SectorsStatus {
            rev: 6,
            layouts: 2,
            strips: 3,
            unknown: 1,
            undo: false,
            error: None,
        }
    );
    router.handle(tick(1, "t1", "lu-2", "Klavir", true));
    let now = status(&mut router).sectors;
    assert_eq!((now.rev, now.strips, now.undo), (7, 4, true));
}

#[tokio::test]
async fn with_no_layout_served_every_tick_on_is_not_in_the_pool() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        1,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    write_doc(dir.path(), &first);
    let (mut router, _own, _rx) = offline_router(dir.path());
    let a = attach(&mut router, 1);
    a.take();
    router.handle(tick(1, "t1", "lu-1", "Bass", true));
    router.handle(tick(1, "t2", "lu-1", "Vox 1", false));
    assert_eq!(
        answers(&a),
        vec![error("t1", "not in the pool"), result("t2")]
    );
    assert_eq!(status(&mut router).sectors.unknown, 0);
}

#[tokio::test]
async fn a_write_that_fails_is_an_error_to_the_asker_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(
        1,
        vec![(SectorId::Lu, vec![layout("lu-1", "VOX", &["Vox 1"])])],
    );
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    router.handle(tick(1, "t1", "lu-1", "Bass", true));
    a.take();
    b.take();
    let _ = records(&rx);
    let taken = router.sectors.store.doc().clone();
    // The file's temporary name is taken by a folder: its write fails.
    std::fs::create_dir(dir.path().join("sectors.tmp")).unwrap();
    std::fs::write(dir.path().join("sectors.tmp").join("x"), "x").unwrap();
    router.handle(tick(1, "t2", "lu-1", "Klavir", true));
    router.handle(RouterMsg::SectorUndo {
        client: 1,
        id: "u1".into(),
    });
    let got = a.take().unwrap();
    assert_eq!(got.len(), 2, "{got:?}");
    for (msg, want) in got.iter().zip(["t2", "u1"]) {
        assert!(
            matches!(msg, ServerMsg::Error { id: Some(id), message } if id == want && !message.is_empty()),
            "{msg:?}"
        );
    }
    assert_eq!(b.take().unwrap(), vec![], "nothing told");
    assert_eq!(sector_records(&rx), Vec::<Value>::new());
    assert_eq!(router.sectors.store.doc(), &taken);
    assert_eq!(on_disk(dir.path(), SECTORS_FILE), taken);
}
