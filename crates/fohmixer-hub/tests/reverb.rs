//! REVERB end to end (#130, D18), part 1: the `+REVERB` markers make the
//! reverb faders; the hub serves them as reverb strips and the reverb
//! faders are told to every page.
#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Duration;

use fohmixer_proto::client::{ReverbState, ServerMsg};
use fohmixer_proto::layout::{Control, LayoutResponse};
use serde_json::json;
use support::{Client, Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);

/// One page whose group shows the tag group FX, and the REVERB button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "reverb", "label": "REVERB"}],
        "rows": [{"sections": [{"kind": "group", "id": "fx", "title": "FX", "tags": "FX"}]}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// The two reverb faders on SimLive's band: Hand3 # and Hand4 #.
fn tag(host: &mut Host) {
    assert_eq!(
        host.tuner("track", 2, "set", r#""Rev ST" +G:FX +REVERB"#),
        1
    );
    assert_eq!(
        host.tuner("track", 3, "set", r#""Rev PA" +G:FX +REVERB"#),
        1
    );
}

/// The first `reverb` message that passes `check`.
async fn reverb_until(client: &mut Client, check: impl Fn(&ReverbState) -> bool) -> ReverbState {
    client
        .wait(SECS_5, |m| match m {
            ServerMsg::Reverb(s) if check(s) => Some(s.clone()),
            _ => None,
        })
        .await
}

#[test]
fn the_reverb_faders_are_told_and_served() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        let mut a = hub.client().await;
        let state = reverb_until(&mut a, |s| s.faders.len() == 2).await;
        assert_eq!(
            state
                .faders
                .iter()
                .map(|f| f.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Rev ST", "Rev PA"]
        );
        assert_eq!(
            state.faders[0].key,
            "band|live_set tracks 2 mixer_device volume|value"
        );
        assert!(!state.aut && state.low.is_none() && !state.waiting);
        let (_, body) = hub.get("/api/layout").await;
        let layout: LayoutResponse = serde_json::from_value(body).unwrap();
        let strips: Vec<_> = layout.layout.pages[0]
            .controls()
            .into_iter()
            .filter_map(|c| match c {
                Control::Strip(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(strips.len(), 2);
        assert!(strips.iter().all(|s| s.reverb));
        let status = hub.status().await;
        assert_eq!(status.reverb.faders, 2);
        assert!(!status.reverb.aut);
        a.close().await;
        hub.stop().await;
        host.tuner("track", 2, "remove", "");
        host.tuner("track", 3, "remove", "");
        host.stop();
    });
}
