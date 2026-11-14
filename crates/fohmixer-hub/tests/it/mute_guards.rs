//! The mute guard's marks end to end (#203): the document at a page's
//! attach, a page's switch reaching a second page and `mute_guards.json`
//! (never the mute alarm's file), off, a fader not in the pool refused, a
//! hand edit, the records and `/api/status`. No Live host (the instances
//! are unreachable): these run on Windows too. Nothing reaches Live: the
//! hold is the page's.

use std::path::Path;
use std::time::Duration;

use crate::support::{Client, TestHub, runtime, serial};
use fohmixer_hub::config::InstanceCfg;
use fohmixer_hub::mute_alarms::MUTE_ALARMS_FILE;
use fohmixer_hub::mute_guards::{MUTE_GUARDS_FILE, parse};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::mute_guards::MuteGuardsDoc;
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::strip_marks::StripMarks;
use serde_json::{Value, json};

const SECS_3: Duration = Duration::from_secs(3);

/// Two instances nothing listens for.
fn instances() -> Vec<InstanceCfg> {
    vec![
        InstanceCfg {
            name: "band".into(),
            port: 1,
            link: None,
        },
        InstanceCfg {
            name: "master".into(),
            port: 2,
            link: None,
        },
    ]
}

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

/// A document with these band faders guarded.
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

/// A hub over the synthetic sectors fixture (Vox 1..4, Klavir, Bass,
/// Keys 1, PA Out), with `doc` as `mute_guards.json` when given, once it
/// serves the layout.
async fn start(dir: &Path, doc: Option<&MuteGuardsDoc>) -> TestHub {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sectors-layout.json");
    std::fs::copy(fixture, dir.join("layout.json")).unwrap();
    if let Some(doc) = doc {
        write_doc(dir, doc);
    }
    let hub = TestHub::start(instances(), dir).await;
    hub.status_until(SECS_3, |s| s.layout.rev == 1).await;
    hub
}

/// The first `mute_guards` message `check` takes.
async fn guards_where(
    client: &mut Client,
    check: impl Fn(&MuteGuardsDoc) -> bool,
) -> MuteGuardsDoc {
    client
        .wait(SECS_3, |m| match m {
            ServerMsg::MuteGuards { doc } if check(doc) => Some(doc.clone()),
            _ => None,
        })
        .await
}

/// The answer (`result` or `error`) to the request `id`.
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
    ClientMsg::SetMuteGuard {
        id: id.into(),
        strip: band(label),
        on,
    }
}

#[test]
fn a_switch_reaches_a_second_page_and_its_own_file_and_off_clears_it() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), None).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        assert_eq!(guards_where(&mut a, |_| true).await, MuteGuardsDoc::empty());
        assert_eq!(guards_where(&mut b, |_| true).await, MuteGuardsDoc::empty());
        a.send(&set("g1", "Bass", true)).await;
        assert_eq!(answer(&mut a, "g1").await, Ok(()));
        let guarded = doc(&["Bass"]);
        assert_eq!(guards_where(&mut b, |d| *d == guarded).await, guarded);
        assert_eq!(on_disk(dir.path()), guarded);
        assert!(
            !dir.path().join(MUTE_ALARMS_FILE).exists(),
            "the alarm's file is another"
        );
        let status = hub.status().await;
        assert_eq!(
            (status.mute_guards.strips, status.mute_alarms.strips),
            (1, 0)
        );
        b.send(&set("g2", "Bass", false)).await;
        assert_eq!(answer(&mut b, "g2").await, Ok(()));
        assert_eq!(
            guards_where(&mut a, |d| *d == MuteGuardsDoc::empty()).await,
            MuteGuardsDoc::empty()
        );
        assert_eq!(on_disk(dir.path()), MuteGuardsDoc::empty());
        hub.stop().await;
    });
}

#[test]
fn a_fader_not_in_the_pool_is_refused_and_a_hand_edit_is_served() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let first = doc(&["Vox 1", "Gone"]);
        let hub = start(dir.path(), Some(&first)).await;
        let mut page = hub.client().await;
        assert_eq!(guards_where(&mut page, |_| true).await, first);
        let status = hub.status().await;
        assert_eq!(
            (status.mute_guards.strips, status.mute_guards.unknown),
            (2, 1)
        );
        page.send(&set("g1", "Nobody", true)).await;
        assert_eq!(
            answer(&mut page, "g1").await,
            Err("not in the pool".to_string())
        );
        let bad = br#"{"mute_guards": {"band": "Vox 1"}}"#;
        std::fs::write(dir.path().join(MUTE_GUARDS_FILE), bad).unwrap();
        let why = parse(bad).unwrap_err();
        let status = hub
            .status_until(SECS_3, |s| {
                s.mute_guards.error.as_deref() == Some(why.as_str())
            })
            .await;
        assert_eq!(status.mute_guards.strips, 2, "the last good document stays");
        let edited = doc(&["Klavir"]);
        write_doc(dir.path(), &edited);
        assert_eq!(guards_where(&mut page, |d| *d == edited).await, edited);
        hub.stop().await;
    });
}

#[test]
fn the_records_name_what_and_who_and_nothing_reaches_live() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = start(dir.path(), None).await;
        let mut page = hub.client().await;
        page.send(&set("g1", "Klavir", true)).await;
        assert_eq!(answer(&mut page, "g1").await, Ok(()));
        write_doc(dir.path(), &doc(&["Vox 2"]));
        let guards = |records: &[Value]| -> Vec<Value> {
            records
                .iter()
                .filter(|r| r["ev"] == "mute_guards")
                .cloned()
                .collect()
        };
        let records = hub.events_until(SECS_3, |r| guards(r).len() >= 2).await;
        let got = guards(&records);
        assert_eq!(got[0]["what"], json!("set"));
        assert_eq!(got[0]["peer"], json!("127.0.0.1"));
        assert_eq!(
            got[0]["strip"],
            json!({"instance": "band", "label": "Klavir"})
        );
        assert_eq!(got[0]["on"], json!(true));
        assert_eq!(got[1]["what"], json!("file"));
        assert!(records.iter().all(|r| r["ev"] != "set"));
        assert!(records.iter().all(|r| r["ev"] != "mute_alarms"));
        hub.stop().await;
    });
}
