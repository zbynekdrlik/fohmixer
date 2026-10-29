//! The strips' colours end to end (#164): the document at a page's attach,
//! a page's choice reaching a second page and the file, NONE, a fader not
//! in the pool refused, a hand edit of `colors.json`, the records and
//! `/api/status`. No Live host (the instances are unreachable): these run
//! on Windows too. Nothing reaches Live: a colour is the hub's alone.

use std::path::Path;
use std::time::Duration;

use crate::support::{Client, TestHub, runtime, serial};
use fohmixer_hub::colors::{COLORS_FILE, parse};
use fohmixer_hub::config::InstanceCfg;
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::colors::{ColorKey, ColorsDoc};
use fohmixer_proto::sectors::StripRef;
use serde_json::{Value, json};

const SECS_3: Duration = Duration::from_secs(3);

/// Two instances nothing listens for.
fn instances() -> Vec<InstanceCfg> {
    vec![
        InstanceCfg {
            name: "band".into(),
            port: 1,
        },
        InstanceCfg {
            name: "master".into(),
            port: 2,
        },
    ]
}

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

/// A document with these band faders in these colours.
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

/// A hub over the synthetic sectors fixture (`tests/fixtures/
/// sectors-layout.json`: Vox 1..4, Klavir, Bass, Keys 1, PA Out), with
/// `doc` as `colors.json` when given, once it serves the layout.
async fn start(dir: &Path, doc: Option<&ColorsDoc>) -> TestHub {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(doc) = doc {
        write_doc(dir, doc);
    }
    let hub = TestHub::start(instances(), dir).await;
    hub.status_until(SECS_3, |s| s.layout.rev == 1).await;
    hub
}

/// The first `colors` message `check` takes.
async fn colors_where(client: &mut Client, check: impl Fn(&ColorsDoc) -> bool) -> ColorsDoc {
    client
        .wait(SECS_3, |m| match m {
            ServerMsg::Colors { doc } if check(doc) => Some(doc.clone()),
            _ => None,
        })
        .await
}

/// The answer (`result` or `error`) to the request `id`: `Ok` or the
/// error's words.
async fn answer(client: &mut Client, id: &str) -> Result<(), String> {
    client
        .wait(SECS_3, |m| match m {
            ServerMsg::Result { id: got, .. } if got == id => Some(Ok(())),
            ServerMsg::Error {
                id: Some(got),
                message,
            } if got == id => Some(Err(message.clone())),
            _ => None,
        })
        .await
}

fn set(id: &str, label: &str, color: Option<ColorKey>) -> ClientMsg {
    ClientMsg::SetColor {
        id: id.into(),
        strip: band(label),
        color,
    }
}

#[test]
fn a_page_gets_the_document_at_attach_and_after_a_hand_edit() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&[("Vox 1", ColorKey::Pink)]);
        let hub = start(dir.path(), Some(&first)).await;
        let mut page = hub.client().await;
        assert_eq!(colors_where(&mut page, |_| true).await, first);
        let edited = doc(&[("Bass", ColorKey::Blue)]);
        write_doc(dir.path(), &edited);
        assert_eq!(colors_where(&mut page, |d| *d == edited).await, edited);
        assert_eq!(on_disk(dir.path()), edited, "not written back");
        let mut later = hub.client().await;
        assert_eq!(colors_where(&mut later, |_| true).await, edited);
        hub.stop().await;
    });
}

#[test]
fn a_choice_reaches_a_second_page_and_the_file_and_none_resets_it() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        // No colors.json: every strip NONE, nothing written yet.
        let hub = start(dir.path(), None).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        assert_eq!(colors_where(&mut a, |_| true).await, ColorsDoc::empty());
        assert_eq!(colors_where(&mut b, |_| true).await, ColorsDoc::empty());
        assert!(!dir.path().join(COLORS_FILE).exists());
        a.send(&set("c1", "Bass", Some(ColorKey::Teal))).await;
        assert_eq!(answer(&mut a, "c1").await, Ok(()));
        let teal = doc(&[("Bass", ColorKey::Teal)]);
        assert_eq!(colors_where(&mut b, |d| *d == teal).await, teal);
        assert_eq!(on_disk(dir.path()), teal);
        assert_eq!(hub.status().await.colors.strips, 1, "the status counts it");
        // NONE takes it away again.
        b.send(&set("c2", "Bass", None)).await;
        assert_eq!(answer(&mut b, "c2").await, Ok(()));
        assert_eq!(
            colors_where(&mut a, |d| *d == ColorsDoc::empty()).await,
            ColorsDoc::empty()
        );
        assert_eq!(on_disk(dir.path()), ColorsDoc::empty());
        hub.stop().await;
    });
}

#[test]
fn a_fader_not_in_the_pool_is_refused_and_nothing_changes() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&[("Vox 1", ColorKey::Pink)]);
        let hub = start(dir.path(), Some(&first)).await;
        let mut page = hub.client().await;
        page.send(&set("c1", "Nobody", Some(ColorKey::Red))).await;
        assert_eq!(
            answer(&mut page, "c1").await,
            Err("not in the pool".to_string())
        );
        // The same colour again changes nothing: answered, no record.
        page.send(&set("c2", "Vox 1", Some(ColorKey::Pink))).await;
        assert_eq!(answer(&mut page, "c2").await, Ok(()));
        assert_eq!(on_disk(dir.path()), first);
        assert!(
            hub.events().iter().all(|r| r["ev"] != "colors"),
            "no record for a refusal or a choice that changes nothing"
        );
        hub.stop().await;
    });
}

#[test]
fn a_bad_hand_edit_keeps_the_document_and_the_status_says_why() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&[("Vox 1", ColorKey::Pink), ("Gone", ColorKey::Red)]);
        let hub = start(dir.path(), Some(&first)).await;
        let status = hub.status().await;
        assert_eq!((status.colors.strips, status.colors.unknown), (2, 1));
        let bad = br#"{"colors": {"band": {"Vox 1": "gold"}}}"#;
        std::fs::write(dir.path().join(COLORS_FILE), bad).unwrap();
        let why = parse(bad).unwrap_err();
        // The poll may read the file half written: wait for this content's
        // own error.
        let status = hub
            .status_until(SECS_3, |s| s.colors.error.as_deref() == Some(why.as_str()))
            .await;
        assert_eq!(status.colors.strips, 2, "the last good document stays");
        write_doc(dir.path(), &doc(&[("Bass", ColorKey::Blue)]));
        let status = hub
            .status_until(SECS_3, |s| s.colors.error.is_none() && s.colors.strips == 1)
            .await;
        assert_eq!(status.colors.unknown, 0);
        hub.stop().await;
    });
}

#[test]
fn the_records_name_what_and_who() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), None).await;
        let mut page = hub.client().await;
        page.send(&set("c1", "Klavir", Some(ColorKey::Amber))).await;
        assert_eq!(answer(&mut page, "c1").await, Ok(()));
        page.send(&set("c2", "Klavir", None)).await;
        assert_eq!(answer(&mut page, "c2").await, Ok(()));
        write_doc(dir.path(), &doc(&[("Vox 2", ColorKey::Lime)]));
        let colors = |records: &[Value]| -> Vec<Value> {
            records
                .iter()
                .filter(|r| r["ev"] == "colors")
                .cloned()
                .collect()
        };
        let records = hub.events_until(SECS_3, |r| colors(r).len() >= 3).await;
        let got = colors(&records);
        let whats: Vec<&Value> = got.iter().map(|r| &r["what"]).collect();
        assert_eq!(whats, [&json!("set"), &json!("set"), &json!("file")]);
        assert!(got[0]["client"].is_u64());
        assert_eq!(got[0]["peer"], json!("127.0.0.1"));
        assert_eq!(
            got[0]["strip"],
            json!({"instance": "band", "label": "Klavir"})
        );
        assert_eq!(got[0]["color"], json!("amber"));
        assert_eq!(got[1]["color"], Value::Null);
        assert_eq!(got[2]["client"], Value::Null);
        // Nothing reached Live: no write of any kind was recorded.
        assert!(records.iter().all(|r| r["ev"] != "set"));
        hub.stop().await;
    });
}
