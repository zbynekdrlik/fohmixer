//! The fader sectors end to end (#148): the document at a page's attach,
//! a tick, FULL, ARRANGE's save and the undo, a hand edit of
//! `sectors.json`, the records and `/api/status`. No Live host (the
//! instances are unreachable, so no seed runs): these run on Windows too.

use std::path::Path;
use std::time::Duration;

use crate::support::{Client, TestHub, runtime, serial};
use fohmixer_hub::config::InstanceCfg;
use fohmixer_hub::sectors::{PREV_FILE, SECTORS_FILE, parse};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::sectors::{SectorId, SectorLayout, SectorOrder, SectorsDoc, StripRef};
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

/// A document at `rev` whose `lu-1` holds these band faders.
fn lu(rev: u64, labels: &[&str]) -> SectorsDoc {
    SectorsDoc {
        rev,
        sectors: [(
            SectorId::Lu,
            vec![SectorLayout {
                id: "lu-1".into(),
                title: "VOX".into(),
                strips: labels.iter().map(|l| band(l)).collect(),
            }],
        )]
        .into_iter()
        .collect(),
    }
}

fn write_doc(dir: &Path, doc: &SectorsDoc) {
    std::fs::write(dir.join(SECTORS_FILE), serde_json::to_string(doc).unwrap()).unwrap();
}

fn on_disk(dir: &Path) -> SectorsDoc {
    serde_json::from_slice(&std::fs::read(dir.join(SECTORS_FILE)).unwrap()).unwrap()
}

/// A hub over the synthetic sectors fixture (`tests/fixtures/
/// sectors-layout.json`), with `doc` as `sectors.json` when given, once it
/// serves the layout.
async fn start(dir: &Path, doc: Option<&SectorsDoc>) -> TestHub {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(doc) = doc {
        write_doc(dir, doc);
    }
    let hub = TestHub::start(instances(), dir).await;
    hub.status_until(SECS_3, |s| s.layout.rev == 1).await;
    hub
}

/// The first `sectors` message `check` takes.
async fn sectors_where(
    client: &mut Client,
    check: impl Fn(&SectorsDoc, bool) -> bool,
) -> (SectorsDoc, bool) {
    client
        .wait(SECS_3, |m| match m {
            ServerMsg::Sectors { doc, undo } if check(doc, *undo) => Some((doc.clone(), *undo)),
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

fn tick(id: &str, label: &str, on: bool) -> ClientMsg {
    ClientMsg::SectorTick {
        id: id.into(),
        layout: "lu-1".into(),
        strip: band(label),
        on,
    }
}

fn order(id: &str, labels: &[&str], rev: u64) -> ClientMsg {
    ClientMsg::SectorOrders {
        id: id.into(),
        orders: vec![SectorOrder {
            layout: "lu-1".into(),
            strips: labels.iter().map(|l| band(l)).collect(),
        }],
        rev,
    }
}

#[test]
fn a_page_gets_the_document_at_attach_and_after_a_hand_edit() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), Some(&lu(3, &["Vox 1"]))).await;
        let mut page = hub.client().await;
        let (doc, undo) = sectors_where(&mut page, |_, _| true).await;
        assert_eq!(doc, lu(3, &["Vox 1"]));
        assert!(!undo);
        // A hand edit at an older revision: served one past the served.
        write_doc(dir.path(), &lu(1, &["Klavir", "Bass"]));
        let (doc, undo) = sectors_where(&mut page, |d, _| d.rev == 4).await;
        assert_eq!(doc, lu(4, &["Klavir", "Bass"]));
        assert!(undo, "the version before it is the undo");
        assert_eq!(
            on_disk(dir.path()),
            lu(1, &["Klavir", "Bass"]),
            "not written back"
        );
        // A page attached now gets it too.
        let mut later = hub.client().await;
        let (doc, _) = sectors_where(&mut later, |_, _| true).await;
        assert_eq!(doc.rev, 4);
        hub.stop().await;
    });
}

#[test]
fn a_tick_reaches_a_second_page_and_the_file() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), Some(&lu(1, &["Vox 1"]))).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        sectors_where(&mut a, |d, _| d.rev == 1).await;
        sectors_where(&mut b, |d, _| d.rev == 1).await;
        a.send(&tick("t1", "Bass", true)).await;
        assert_eq!(answer(&mut a, "t1").await, Ok(()));
        let (doc, undo) = sectors_where(&mut b, |d, _| d.rev == 2).await;
        assert_eq!(doc, lu(2, &["Vox 1", "Bass"]));
        assert!(undo);
        assert_eq!(on_disk(dir.path()), lu(2, &["Vox 1", "Bass"]));
        assert_eq!(hub.status().await.sectors.strips, 2, "the status counts it");
        hub.stop().await;
    });
}

#[test]
fn a_full_layout_is_refused_full() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let full = lu(1, &["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Klavir", "Keys 1"]);
        let hub = start(dir.path(), Some(&full)).await;
        let mut page = hub.client().await;
        page.send(&tick("t1", "PA Out", true)).await;
        assert_eq!(answer(&mut page, "t1").await, Err("FULL".to_string()));
        page.send(&tick("t2", "Nobody", true)).await;
        assert_eq!(
            answer(&mut page, "t2").await,
            Err("not in the pool".to_string())
        );
        assert_eq!(on_disk(dir.path()), full);
        hub.stop().await;
    });
}

#[test]
fn orders_and_undo_end_to_end() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), Some(&lu(1, &["Vox 1", "Vox 2", "Vox 3"]))).await;
        let mut page = hub.client().await;
        page.send(&order("o1", &["Vox 3", "Vox 1", "Vox 2"], 1))
            .await;
        assert_eq!(answer(&mut page, "o1").await, Ok(()));
        let (doc, _) = sectors_where(&mut page, |d, _| d.rev == 2).await;
        assert_eq!(doc, lu(2, &["Vox 3", "Vox 1", "Vox 2"]));
        page.send(&order("o2", &["Vox 1", "Vox 2", "Vox 3"], 1))
            .await;
        assert_eq!(answer(&mut page, "o2").await, Err("stale".to_string()));
        page.send(&ClientMsg::SectorUndo { id: "u1".into() }).await;
        assert_eq!(answer(&mut page, "u1").await, Ok(()));
        let (doc, undo) = sectors_where(&mut page, |d, _| d.rev == 3).await;
        assert_eq!(doc, lu(3, &["Vox 1", "Vox 2", "Vox 3"]));
        assert!(undo);
        page.send(&ClientMsg::SectorUndo { id: "u2".into() }).await;
        assert_eq!(answer(&mut page, "u2").await, Ok(()));
        let (doc, _) = sectors_where(&mut page, |d, _| d.rev == 4).await;
        assert_eq!(doc, lu(4, &["Vox 3", "Vox 1", "Vox 2"]), "redone");
        let prev: SectorsDoc =
            serde_json::from_slice(&std::fs::read(dir.path().join(PREV_FILE)).unwrap()).unwrap();
        assert_eq!(prev, lu(3, &["Vox 1", "Vox 2", "Vox 3"]));
        hub.stop().await;
    });
}

#[test]
fn a_bad_hand_edit_keeps_the_document_and_the_status_says_why() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), Some(&lu(2, &["Vox 1"]))).await;
        let mut page = hub.client().await;
        sectors_where(&mut page, |d, _| d.rev == 2).await;
        let bad = br#"{"rev": 1, "sectors": {"lx": []}}"#;
        std::fs::write(dir.path().join(SECTORS_FILE), bad).unwrap();
        let why = parse(bad).unwrap_err();
        // The poll may read the file half written: wait for this content's
        // own error.
        let status = hub
            .status_until(SECS_3, |s| s.sectors.error.as_deref() == Some(why.as_str()))
            .await;
        assert_eq!(status.sectors.rev, 2, "the last good document stays");
        assert_eq!(status.sectors.layouts, 1);
        // A good edit clears it.
        write_doc(dir.path(), &lu(1, &["Bass"]));
        let status = hub
            .status_until(SECS_3, |s| s.sectors.error.is_none() && s.sectors.rev == 3)
            .await;
        assert_eq!(status.sectors.strips, 1);
        hub.stop().await;
    });
}

#[test]
fn the_records_name_what_and_who() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), Some(&lu(1, &["Vox 1"]))).await;
        let mut page = hub.client().await;
        page.send(&tick("t1", "Bass", true)).await;
        assert_eq!(answer(&mut page, "t1").await, Ok(()));
        page.send(&order("o1", &["Bass", "Vox 1"], 2)).await;
        assert_eq!(answer(&mut page, "o1").await, Ok(()));
        page.send(&ClientMsg::SectorUndo { id: "u1".into() }).await;
        assert_eq!(answer(&mut page, "u1").await, Ok(()));
        write_doc(dir.path(), &lu(1, &["Klavir"]));
        let sectors = |records: &[Value]| -> Vec<Value> {
            records
                .iter()
                .filter(|r| r["ev"] == "sectors")
                .cloned()
                .collect()
        };
        let records = hub.events_until(SECS_3, |r| sectors(r).len() >= 4).await;
        let got = sectors(&records);
        let whats: Vec<&Value> = got.iter().map(|r| &r["what"]).collect();
        assert_eq!(
            whats,
            [
                &json!("tick"),
                &json!("order"),
                &json!("undo"),
                &json!("file")
            ]
        );
        assert!(got[0]["client"].is_u64());
        assert_eq!(got[0]["peer"], json!("127.0.0.1"));
        assert_eq!(got[0]["layout"], json!("lu-1"));
        assert_eq!(
            got[0]["strip"],
            json!({"instance": "band", "label": "Bass"})
        );
        assert_eq!(got[0]["on"], json!(true));
        assert_eq!(got[0]["rev"], json!(2));
        assert_eq!(got[1]["layouts"], json!(["lu-1"]));
        assert_eq!(got[1]["client"], got[0]["client"]);
        assert_eq!(got[2]["rev"], json!(4));
        assert_eq!(got[3]["client"], Value::Null);
        assert_eq!(got[3]["peer"], Value::Null);
        assert_eq!(got[3]["rev"], json!(5));
        hub.stop().await;
    });
}

#[test]
fn no_seed_runs_while_no_instance_was_read() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), None).await;
        let mut page = hub.client().await;
        let (doc, undo) = sectors_where(&mut page, |_, _| true).await;
        assert_eq!(doc, SectorsDoc::empty());
        assert!(!undo);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!dir.path().join(SECTORS_FILE).exists());
        let status = hub.status().await;
        assert_eq!(status.sectors.rev, 0);
        assert_eq!(status.sectors.layouts, 0);
        hub.stop().await;
    });
}
