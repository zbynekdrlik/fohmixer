use std::path::Path;
use std::sync::Arc;

use fohmixer_proto::client::{ColorsStatus, ServerMsg};
use fohmixer_proto::colors::{ColorKey, ColorsDoc};
use fohmixer_proto::sectors::StripRef;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::tests::{attach, offline_router, records, status};
use super::super::{Router, RouterMsg};
use super::{
    RESTORED_LINE, TAKEN_LINE, colors_fields, refused_line, set_line, unreadable_line,
    unwritable_line, write_problem,
};
use crate::colors::COLORS_FILE;
use crate::events::{is_warn, stamp};
use crate::outbox::Outbox;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn doc(pairs: &[(&str, ColorKey)]) -> ColorsDoc {
    pairs.iter().fold(ColorsDoc::empty(), |doc, (label, key)| {
        doc.with(&band(label), Some(*key))
    })
}

fn write_doc(dir: &Path, doc: &ColorsDoc) {
    std::fs::write(dir.join(COLORS_FILE), serde_json::to_string(doc).unwrap()).unwrap();
}

fn on_disk(dir: &Path) -> ColorsDoc {
    serde_json::from_slice(&std::fs::read(dir.join(COLORS_FILE)).unwrap()).unwrap()
}

/// A router over the offline band serving the synthetic sectors fixture
/// (`tests/fixtures/sectors-layout.json`: Vox 1..4, Klavir, Bass, Keys 1
/// and PA Out, band tracks by name), with `doc` as `colors.json` when
/// given.
fn served(
    dir: &Path,
    colors: Option<&ColorsDoc>,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(colors) = colors {
        write_doc(dir, colors);
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

/// The `colors` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<ColorsDoc> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Colors { doc } => Some(doc),
            _ => None,
        })
        .collect()
}

/// The `colors` records waiting, without their time.
fn color_records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "colors")
        .map(|mut r| {
            r.as_object_mut().unwrap().remove("ts");
            r
        })
        .collect()
}

fn set(client: u64, id: &str, label: &str, color: Option<ColorKey>) -> RouterMsg {
    RouterMsg::SetColor {
        client,
        id: id.into(),
        strip: band(label),
        color,
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
    let first = doc(&[("Vox 1", ColorKey::Pink)]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![first]);
    let empty = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = served(empty.path(), None);
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![ColorsDoc::empty()]);
}

#[tokio::test]
async fn a_choice_is_answered_recorded_and_told_to_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(set(1, "c1", "Bass", Some(ColorKey::Blue)));
    let after = doc(&[("Bass", ColorKey::Blue)]);
    assert_eq!(
        a.take().unwrap(),
        vec![result("c1"), ServerMsg::Colors { doc: after.clone() }]
    );
    assert_eq!(told(&b), vec![after.clone()]);
    assert_eq!(on_disk(dir.path()), after);
    assert_eq!(
        color_records(&rx),
        vec![
            json!({"ev": "colors", "what": "set", "client": 1, "peer": "10.0.0.1",
                   "strip": {"instance": "band", "label": "Bass"}, "color": "blue"})
        ]
    );
    // NONE: the colour goes, `null` in the record.
    router.handle(set(2, "c2", "Bass", None));
    assert_eq!(b.take().unwrap()[0], result("c2"));
    assert_eq!(told(&a), vec![ColorsDoc::empty()]);
    assert_eq!(on_disk(dir.path()), ColorsDoc::empty());
    assert_eq!(color_records(&rx)[0]["color"], Value::Null);
    // A choice that changes nothing: its result only.
    router.handle(set(1, "c3", "Bass", None));
    assert_eq!(a.take().unwrap(), vec![result("c3")]);
    assert_eq!(told(&b), vec![]);
    assert_eq!(color_records(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn a_colour_for_a_fader_not_in_the_pool_is_refused_to_the_asker_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = served(dir.path(), None);
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    router.handle(set(1, "c1", "Nobody", Some(ColorKey::Red)));
    assert_eq!(a.take().unwrap(), vec![error("c1", "not in the pool")]);
    assert_eq!(b.take().unwrap(), vec![]);
    assert_eq!(color_records(&rx), Vec::<Value>::new());
    assert!(!dir.path().join(COLORS_FILE).exists(), "nothing written");
    // With no layout served nothing is in the pool.
    let bare = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(bare.path());
    let page = attach(&mut router, 1);
    page.take();
    router.handle(set(1, "c2", "Bass", Some(ColorKey::Red)));
    assert_eq!(page.take().unwrap(), vec![error("c2", "not in the pool")]);
}

#[tokio::test]
async fn a_hand_edit_is_told_and_recorded_file_and_a_bad_one_shows_in_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&[("Vox 1", ColorKey::Pink)]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    router.handle(RouterMsg::ColorsPoll);
    assert_eq!(told(&a), vec![], "the file as served");
    let edited = doc(&[("Klavir", ColorKey::Amber)]);
    write_doc(dir.path(), &edited);
    router.handle(RouterMsg::ColorsPoll);
    assert_eq!(told(&a), vec![edited.clone()]);
    assert_eq!(
        color_records(&rx),
        vec![json!({"ev": "colors", "what": "file", "client": null, "peer": null})]
    );
    std::fs::write(dir.path().join(COLORS_FILE), "{").unwrap();
    router.handle(RouterMsg::ColorsPoll);
    assert_eq!(told(&a), vec![]);
    assert_eq!(color_records(&rx), Vec::<Value>::new());
    let now = status(&mut router).colors;
    assert_eq!(now.strips, 1, "the last good document stays");
    assert_eq!(now.error, Some(crate::colors::parse(b"{").unwrap_err()));
}

#[tokio::test]
async fn a_removed_file_is_written_back_and_recorded_once() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&[("Vox 1", ColorKey::Pink)]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    a.take();
    let _ = records(&rx);
    std::fs::remove_file(dir.path().join(COLORS_FILE)).unwrap();
    router.handle(RouterMsg::ColorsPoll);
    assert_eq!(on_disk(dir.path()), first);
    assert_eq!(told(&a), vec![], "the document did not change");
    assert_eq!(
        color_records(&rx),
        vec![
            json!({"ev": "colors", "what": "file", "client": null, "peer": null,
                   "why": "deleted"})
        ]
    );
    router.handle(RouterMsg::ColorsPoll);
    assert_eq!(color_records(&rx), Vec::<Value>::new(), "once");
}

#[tokio::test]
async fn the_status_counts_the_colours_and_those_no_pool_fader_carries() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&[("Vox 1", ColorKey::Pink), ("Renamed", ColorKey::Red)]);
    let (mut router, _own, _rx) = served(dir.path(), Some(&first));
    assert_eq!(
        status(&mut router).colors,
        ColorsStatus {
            strips: 2,
            unknown: 1,
            error: None,
        }
    );
    router.handle(set(1, "c1", "Bass", Some(ColorKey::Green)));
    let now = status(&mut router).colors;
    assert_eq!((now.strips, now.unknown), (3, 1));
}

#[tokio::test]
async fn a_write_that_fails_is_an_error_to_the_asker_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let first = doc(&[("Vox 1", ColorKey::Pink)]);
    let (mut router, _own, rx) = served(dir.path(), Some(&first));
    let a = attach(&mut router, 1);
    let b = attach(&mut router, 2);
    a.take();
    b.take();
    let _ = records(&rx);
    // The file's temporary name is taken by a folder: its write fails.
    std::fs::create_dir(dir.path().join("colors.tmp")).unwrap();
    std::fs::write(dir.path().join("colors.tmp").join("x"), "x").unwrap();
    router.handle(set(1, "c1", "Bass", Some(ColorKey::Blue)));
    let got = a.take().unwrap();
    assert!(
        matches!(&got[..], [ServerMsg::Error { id: Some(id), message }] if id == "c1" && !message.is_empty()),
        "{got:?}"
    );
    assert_eq!(b.take().unwrap(), vec![], "nothing told");
    assert_eq!(color_records(&rx), Vec::<Value>::new());
    assert_eq!(router.colors.store.doc(), &first);
    assert_eq!(on_disk(dir.path()), first);
}

#[test]
fn a_colors_record_is_warn_class() {
    assert!(is_warn(&stamp(
        "colors",
        1,
        colors_fields("set", Some(1), Some("10.0.0.1"), json!({"color": "red"}))
    )));
    assert!(is_warn(&stamp(
        "colors",
        1,
        colors_fields("file", None, None, json!({}))
    )));
}

#[test]
fn the_records_and_lines_read_as_written() {
    assert_eq!(
        colors_fields("set", Some(3), Some("10.0.0.3"), json!({"color": "teal"})),
        json!({"what": "set", "client": 3, "peer": "10.0.0.3", "color": "teal"})
    );
    assert_eq!(
        colors_fields("file", None, None, json!({})),
        json!({"what": "file", "client": null, "peer": null})
    );
    assert_eq!(
        set_line(&band("Vox 1"), Some(ColorKey::Pink)),
        "a strip's colour changed: band|Vox 1 is PINK"
    );
    assert_eq!(
        set_line(&band("Vox 1"), None),
        "a strip's colour changed: band|Vox 1 is NONE"
    );
    assert_eq!(
        TAKEN_LINE,
        "colors.json was edited by hand: its colours are served"
    );
    assert_eq!(
        RESTORED_LINE,
        "colors.json was removed; it is written back from the document served"
    );
    assert_eq!(
        refused_line("bad"),
        "colors.json is refused, the last good document stays served: bad"
    );
    assert_eq!(
        write_problem(&std::io::Error::other("disk full")),
        "cannot write the strips' colours (colors.json): disk full"
    );
    assert_eq!(
        unreadable_line("Is a directory"),
        "cannot read colors.json, the document stays served and is read again at the next poll: Is a directory"
    );
    assert_eq!(
        unwritable_line("No such file or directory"),
        "colors.json was removed and cannot be written back, the document stays served and is written again at the next poll: No such file or directory"
    );
}
