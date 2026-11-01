//! The recording watch end to end (#172): the hub reads which of SimLive's
//! band tracks can be armed, follows their `arm` and the record button with
//! no page subscribed, tells every page, and logs each change, so the event
//! log shows when and why a recording stopped.
#![cfg(unix)]

use std::time::Duration;

use crate::support::{Client, Host, TestHub, runtime, serial};
use fohmixer_proto::client::{RecState, RecWatch, ServerMsg};
use serde_json::{Value, json};

const SECS_5: Duration = Duration::from_secs(5);
/// Hand2 # in SimLive's fixture.
const TRACK: &str = "live_set tracks 1";
/// Vocals Repro grp#, a group.
const GROUP: u32 = 4;

/// The first `rec` message that passes `check`.
async fn rec_until(client: &mut Client, check: impl Fn(&RecState) -> bool) -> RecState {
    client
        .wait(SECS_5, |m| match m {
            ServerMsg::Rec(s) if check(s) => Some(s.clone()),
            _ => None,
        })
        .await
}

/// Starts or stops SimLive's transport.
async fn transport(client: &mut Client, verb: &str) {
    let slots = client
        .cmd(
            "band",
            vec![json!({"target": "live_set", "name": verb, "args": []})],
        )
        .await
        .unwrap();
    assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
}

/// The `rec` records so far.
fn records(hub: &TestHub) -> Vec<Value> {
    hub.events()
        .into_iter()
        .filter(|r| r["ev"] == "rec")
        .collect()
}

#[test]
fn the_watch_follows_a_recording_to_its_stop_and_logs_why() {
    let _guard = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start("band");
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut client = hub.client().await;
        // The armable tracks, a group's left out, every arm heard and off.
        let told = rec_until(&mut client, |s| !s.tracks.is_empty()).await;
        assert!(told.tracks.iter().all(|t| !t.arm), "{told:?}");
        assert!(
            told.tracks
                .iter()
                .any(|t| t.index == 1 && t.name == "Hand2 #")
        );
        assert!(told.tracks.iter().all(|t| t.index != GROUP), "{told:?}");
        assert_eq!(told.watch(), RecWatch::Off);

        // The write of a key as a page's: the watch hears it through Live.
        client
            .write("band", TRACK, "arm", json!(true), 1, true)
            .await;
        let armed = rec_until(&mut client, |s| s.armed() == 1).await;
        assert_eq!(armed.watch(), RecWatch::Ready);
        client
            .write("band", "live_set", "record_mode", json!(true), 2, true)
            .await;
        rec_until(&mut client, |s| s.record_mode == Some(true)).await;
        transport(&mut client, "start_playing").await;
        rec_until(&mut client, |s| s.watch() == RecWatch::Recording).await;

        // Each cause of a stop names itself in the log.
        client
            .write("band", "live_set", "record_mode", json!(false), 3, true)
            .await;
        rec_until(&mut client, |s| s.watch() == RecWatch::NotRec).await;
        client
            .write("band", "live_set", "record_mode", json!(true), 4, true)
            .await;
        rec_until(&mut client, |s| s.watch() == RecWatch::Recording).await;
        client
            .write("band", TRACK, "arm", json!(false), 5, true)
            .await;
        rec_until(&mut client, |s| s.watch() == RecWatch::NoArm).await;
        client
            .write("band", TRACK, "arm", json!(true), 6, true)
            .await;
        rec_until(&mut client, |s| s.watch() == RecWatch::Recording).await;
        transport(&mut client, "stop_playing").await;
        rec_until(&mut client, |s| s.watch() == RecWatch::Ready).await;

        let logged = hub
            .events_until(SECS_5, |all| {
                all.iter().any(|r| {
                    r["ev"] == "rec" && r["why"] == "transport" && r["prev"] == "recording"
                })
            })
            .into_iter()
            .filter(|r| r["ev"] == "rec");
        let steps: Vec<(String, String)> = logged
            .map(|r| {
                (
                    r["state"].as_str().unwrap_or_default().to_string(),
                    r["why"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        for step in [
            ("ready", "arm"),
            ("recording", "transport"),
            ("not_rec", "record_mode"),
            ("no_arm", "arm"),
            ("ready", "transport"),
        ] {
            assert!(
                steps.iter().any(|s| s.0 == step.0 && s.1 == step.1),
                "{step:?} in {steps:?}"
            );
        }
        let stop = records(&hub)
            .into_iter()
            .rfind(|r| r["why"] == "transport")
            .expect("the stop");
        assert_eq!(stop["is_playing"], json!(false));
        assert_eq!(stop["record_mode"], json!(true));
        assert_eq!(stop["armed"], json!(1));
        assert_eq!(stop["armed_tracks"], json!(["Hand2 #"]));

        // The hub's own: no page's subscription counts it.
        let status = hub.status().await;
        assert_eq!(status.instances[0].subscriptions, 0);

        // A late page hears the state at its attach.
        let mut late = hub.client().await;
        let at_attach = rec_until(&mut late, |s| s.armed() == 1).await;
        assert_eq!(at_attach.watch(), RecWatch::Ready);

        client.close().await;
        late.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_track_list_change_reads_the_armable_tracks_again() {
    let _guard = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start("band");
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut client = hub.client().await;
        rec_until(&mut client, |s| !s.tracks.is_empty()).await;
        client.set("band", TRACK, "arm", json!(true)).await;
        rec_until(&mut client, |s| s.armed() == 1).await;

        // Hand1 # goes: Hand2 #, armed, is index 0 now, and still followed.
        assert!(host.delete_track(0) >= 0);
        let moved = rec_until(&mut client, |s| {
            s.tracks.iter().any(|t| t.index == 0 && t.arm)
        })
        .await;
        assert_eq!(moved.armed(), 1);
        assert_eq!(moved.tracks[0].name, "Hand2 #");
        client.clear();
        client
            .set("band", "live_set tracks 0", "arm", json!(false))
            .await;
        rec_until(&mut client, |s| s.armed() == 0 && !s.tracks.is_empty()).await;

        client.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_lost_live_forgets_the_state_and_logs_it() {
    let _guard = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start("band");
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut client = hub.client().await;
        rec_until(&mut client, |s| !s.tracks.is_empty()).await;
        client.set("band", TRACK, "arm", json!(true)).await;
        rec_until(&mut client, |s| s.armed() == 1).await;
        client.clear();
        host.stop();
        let gone = rec_until(&mut client, |s| s.tracks.is_empty()).await;
        assert_eq!(gone, RecState::default());
        let logged = hub
            .events_until(SECS_5, |all| {
                all.iter()
                    .any(|r| r["ev"] == "rec" && r["why"] == "offline")
            })
            .into_iter()
            .rfind(|r| r["ev"] == "rec")
            .expect("the record");
        assert_eq!(logged["prev"], json!("ready"));
        assert_eq!(logged["armed"], json!(0));
        client.close().await;
        hub.stop().await;
    });
}
