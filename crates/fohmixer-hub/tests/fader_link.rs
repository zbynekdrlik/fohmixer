//! The fader link end to end (#130, D18): the markers' `+ID` and `+FADER`
//! make a leader and its partner, the frame's linked fader takes the
//! `+FADER` tracks of its link, and the hub tells every page each leader's
//! switch and keeps it across a restart.
#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Duration;

use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::layout::{
    Anchor, Binding, Control, FaderFollow, FaderLink, LayoutResponse, LinkPartner,
};
use fohmixer_proto::markers::{Leader, LinkReport};
use serde_json::{Value, json};
use support::{Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);
const REV: &str = "fader_link|band|REV";

/// The tag group FX, and Podklady All linked to band's PODKLADY.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [
            {"kind": "group", "id": "fx", "title": "FX", "tags": "FX"},
            {"kind": "group", "id": "bt", "controls": [
                {"kind": "param_fader", "label": "Podklady All", "link": {"instance": "band", "id": "PODKLADY"}}]}]}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// SimLive's band: Rev PA (Hand4 #) leads Rev ST (Hand3 #); Drums # and
/// Bass # are Podklady All's.
fn tag(host: &mut Host) {
    assert_eq!(
        host.tuner("track", 3, "set", r#""Rev PA" +G:FX +REVERB +ID:REV"#),
        1
    );
    assert_eq!(
        host.tuner("track", 2, "set", r#""Rev ST" +G:FX +REVERB +FADER:REV"#),
        1
    );
    assert_eq!(host.tuner("track", 10, "set", "+FADER:PODKLADY"), 1);
    assert_eq!(host.tuner("track", 11, "set", "+FADER:PODKLADY"), 1);
}

fn track(index: u32) -> Binding {
    Binding {
        instance: "band".into(),
        anchor: Anchor::TrackAt { index },
        path: None,
    }
}

/// Waits for the hub value of REV's switch to be `on`.
async fn switch_until(client: &mut support::Client, on: bool) {
    client
        .wait(SECS_5, |m| match m {
            ServerMsg::Hub { key, value } if key == REV && value == &json!(on) => Some(()),
            _ => None,
        })
        .await;
}

#[test]
fn a_leader_its_partner_and_the_linked_fader_come_from_the_tags_and_the_switch_survives_a_restart()
{
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        // The whole composition: both links with all their partners, and
        // the router's reverb faders of the same layout.
        let links = vec![
            LinkReport {
                instance: "band".into(),
                id: "PODKLADY".into(),
                leader: Leader::Frame,
                partners: 2,
            },
            LinkReport {
                instance: "band".into(),
                id: "REV".into(),
                leader: Leader::Strip,
                partners: 1,
            },
        ];
        let status = hub
            .status_until(SECS_5, |s| {
                s.layout.markers.links == links && s.reverb.faders == 2
            })
            .await;
        assert_eq!(status.layout.markers.problems, vec![]);
        let (_, body) = hub.get("/api/layout").await;
        let layout: LayoutResponse = serde_json::from_value(body).unwrap();
        let controls = layout.layout.pages[0].controls();
        let strip = |label: &str| {
            controls
                .iter()
                .find_map(|c| match c {
                    Control::Strip(s) if s.label.as_deref() == Some(label) => Some(s.clone()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            strip("Rev PA").fader_link,
            Some(FaderLink {
                id: "REV".into(),
                partners: vec![LinkPartner {
                    binding: track(2),
                    label: "Rev ST".into()
                }]
            })
        );
        assert_eq!(
            strip("Rev ST").fader_follows,
            vec![FaderFollow {
                id: "REV".into(),
                leader: "Rev PA".into(),
                fixed: false
            }]
        );
        let podklady = controls
            .iter()
            .find_map(|c| match c {
                Control::ParamFader {
                    targets,
                    link: Some(_),
                    ..
                } => Some(
                    targets
                        .iter()
                        .map(|t| t.binding.anchor.clone())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            podklady,
            vec![Anchor::TrackAt { index: 10 }, Anchor::TrackAt { index: 11 }]
        );
        let mut a = hub.client().await;
        switch_until(&mut a, true).await;
        a.send(&ClientMsg::SetHub {
            key: REV.into(),
            value: json!(false),
        })
        .await;
        switch_until(&mut a, false).await;
        assert_eq!(
            hub.status()
                .await
                .fader_links
                .iter()
                .map(|l| (l.id.clone(), l.on))
                .collect::<Vec<_>>(),
            vec![("REV".to_string(), false)]
        );
        a.close().await;
        hub.stop().await;
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved["fader_links"],
            json!([{"instance": "band", "id": "REV", "on": false}])
        );
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        switch_until(&mut a, false).await;
        a.close().await;
        hub.stop().await;
        for index in [3, 2, 10, 11] {
            host.tuner("track", index, "remove", "");
        }
        host.stop();
    });
}
