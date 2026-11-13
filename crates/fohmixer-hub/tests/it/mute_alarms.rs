//! The mute alarm's marks end to end (#188): the document at a page's
//! attach, a page's switch reaching a second page and the file, off, a
//! fader not in the pool refused, a hand edit of `mute_alarms.json`, the
//! records and `/api/status`. No Live host (the instances are
//! unreachable): these run on Windows too. Nothing reaches Live: a mark is
//! the hub's alone.

use std::path::Path;
use std::time::Duration;

use crate::support::{Client, TestHub, runtime, serial};
use fohmixer_hub::config::InstanceCfg;
use fohmixer_hub::mute_alarms::{MUTE_ALARMS_FILE, parse};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
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

/// A document with these band faders marked.
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

/// A hub over the synthetic sectors fixture (`tests/fixtures/
/// sectors-layout.json`: Vox 1..4, Klavir, Bass, Keys 1, PA Out), with
/// `doc` as `mute_alarms.json` when given, once it serves the layout.
async fn start(dir: &Path, doc: Option<&MuteAlarmsDoc>) -> TestHub {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(doc) = doc {
        write_doc(dir, doc);
    }
    let hub = TestHub::start(instances(), dir).await;
    hub.status_until(SECS_3, |s| s.layout.rev == 1).await;
    hub
}

/// The first `mute_alarms` message `check` takes.
async fn alarms_where(
    client: &mut Client,
    check: impl Fn(&MuteAlarmsDoc) -> bool,
) -> MuteAlarmsDoc {
    client
        .wait(SECS_3, |m| match m {
            ServerMsg::MuteAlarms { doc } if check(doc) => Some(doc.clone()),
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

fn set(id: &str, label: &str, on: bool) -> ClientMsg {
    ClientMsg::SetMuteAlarm {
        id: id.into(),
        strip: band(label),
        on,
    }
}

#[test]
fn a_page_gets_the_document_at_attach_and_after_a_hand_edit() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&["Vox 1"]);
        let hub = start(dir.path(), Some(&first)).await;
        let mut page = hub.client().await;
        assert_eq!(alarms_where(&mut page, |_| true).await, first);
        let edited = doc(&["Bass"]);
        write_doc(dir.path(), &edited);
        assert_eq!(alarms_where(&mut page, |d| *d == edited).await, edited);
        assert_eq!(on_disk(dir.path()), edited, "not written back");
        let mut later = hub.client().await;
        assert_eq!(alarms_where(&mut later, |_| true).await, edited);
        hub.stop().await;
    });
}

#[test]
fn a_switch_reaches_a_second_page_and_the_file_and_off_clears_it() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        // No mute_alarms.json: nothing marked, nothing written yet.
        let hub = start(dir.path(), None).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        assert_eq!(alarms_where(&mut a, |_| true).await, MuteAlarmsDoc::empty());
        assert_eq!(alarms_where(&mut b, |_| true).await, MuteAlarmsDoc::empty());
        assert!(!dir.path().join(MUTE_ALARMS_FILE).exists());
        a.send(&set("a1", "Bass", true)).await;
        assert_eq!(answer(&mut a, "a1").await, Ok(()));
        let marked = doc(&["Bass"]);
        assert_eq!(alarms_where(&mut b, |d| *d == marked).await, marked);
        assert_eq!(on_disk(dir.path()), marked);
        assert_eq!(
            hub.status().await.mute_alarms.strips,
            1,
            "the status counts it"
        );
        // Off takes it away again.
        b.send(&set("a2", "Bass", false)).await;
        assert_eq!(answer(&mut b, "a2").await, Ok(()));
        assert_eq!(
            alarms_where(&mut a, |d| *d == MuteAlarmsDoc::empty()).await,
            MuteAlarmsDoc::empty()
        );
        assert_eq!(on_disk(dir.path()), MuteAlarmsDoc::empty());
        hub.stop().await;
    });
}

#[test]
fn a_fader_not_in_the_pool_is_refused_and_nothing_changes() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&["Vox 1"]);
        let hub = start(dir.path(), Some(&first)).await;
        let mut page = hub.client().await;
        page.send(&set("a1", "Nobody", true)).await;
        assert_eq!(
            answer(&mut page, "a1").await,
            Err("not in the pool".to_string())
        );
        // The same mark again changes nothing: answered, no record.
        page.send(&set("a2", "Vox 1", true)).await;
        assert_eq!(answer(&mut page, "a2").await, Ok(()));
        assert_eq!(on_disk(dir.path()), first);
        assert!(
            hub.events().iter().all(|r| r["ev"] != "mute_alarms"),
            "no record for a refusal or a switch that changes nothing"
        );
        hub.stop().await;
    });
}

#[test]
fn a_bad_hand_edit_keeps_the_document_and_the_status_says_why() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&["Vox 1", "Gone"]);
        let hub = start(dir.path(), Some(&first)).await;
        let status = hub.status().await;
        assert_eq!(
            (status.mute_alarms.strips, status.mute_alarms.unknown),
            (2, 1)
        );
        let bad = br#"{"mute_alarms": {"band": "Vox 1"}}"#;
        std::fs::write(dir.path().join(MUTE_ALARMS_FILE), bad).unwrap();
        let why = parse(bad).unwrap_err();
        // The poll may read the file half written: wait for this content's
        // own error.
        let status = hub
            .status_until(SECS_3, |s| {
                s.mute_alarms.error.as_deref() == Some(why.as_str())
            })
            .await;
        assert_eq!(status.mute_alarms.strips, 2, "the last good document stays");
        write_doc(dir.path(), &doc(&["Bass"]));
        let status = hub
            .status_until(SECS_3, |s| {
                s.mute_alarms.error.is_none() && s.mute_alarms.strips == 1
            })
            .await;
        assert_eq!(status.mute_alarms.unknown, 0);
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
        page.send(&set("a1", "Klavir", true)).await;
        assert_eq!(answer(&mut page, "a1").await, Ok(()));
        page.send(&set("a2", "Klavir", false)).await;
        assert_eq!(answer(&mut page, "a2").await, Ok(()));
        write_doc(dir.path(), &doc(&["Vox 2"]));
        let alarms = |records: &[Value]| -> Vec<Value> {
            records
                .iter()
                .filter(|r| r["ev"] == "mute_alarms")
                .cloned()
                .collect()
        };
        let records = hub.events_until(SECS_3, |r| alarms(r).len() >= 3).await;
        let got = alarms(&records);
        let whats: Vec<&Value> = got.iter().map(|r| &r["what"]).collect();
        assert_eq!(whats, [&json!("set"), &json!("set"), &json!("file")]);
        assert!(got[0]["client"].is_u64());
        assert_eq!(got[0]["peer"], json!("127.0.0.1"));
        assert_eq!(
            got[0]["strip"],
            json!({"instance": "band", "label": "Klavir"})
        );
        assert_eq!(got[0]["on"], json!(true));
        assert_eq!(got[1]["on"], json!(false));
        assert_eq!(got[2]["client"], Value::Null);
        // Nothing reached Live: no write of any kind was recorded.
        assert!(records.iter().all(|r| r["ev"] != "set"));
        hub.stop().await;
    });
}
