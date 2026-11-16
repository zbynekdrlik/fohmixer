//! Claps from the surface end to end (#220) on SimLive: a set of our own
//! whose `Claps #` track carries the `+CLAPS` Tuner and two arrangement
//! clips (one active, one deactivated); the device is the test's own UDP
//! socket on the hub's `[claps] device_port`, which sends the hub its hello.
//! A page taps while Live plays: the device gets the tapped beats and its
//! switch once a bar passed with no tap; a jump into the active clip
//! silences it; a 1 s hold's stop switches it off; another time signature
//! is told to the pages.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fohmixer_hub::config::Config;
use fohmixer_proto::claps::{ClapsPhase, ClapsState, Hello, Word, encode_hello, parse};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use serde_json::{Value, json};
use tokio::net::UdpSocket;

use crate::support::{Client, Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);

/// The test's own set: the Claps track (index 1) with its Tuner and clips.
fn site(dir: &Path) -> PathBuf {
    let tuner = json!({"template": "plain", "name": "\"CLAPS\" +G:VOCALS +CLAPS",
        "class_name": "Tuner"});
    let set = json!({"name": "Claps Site",
        "tracks": [
            {"name": "Click #"},
            {"name": "Claps #", "devices": [tuner], "arrangement_clips": [
                {"name": "", "start_time": 64.0, "end_time": 96.0},
                {"name": "", "start_time": 128.0, "end_time": 160.0, "muted": true}]}]});
    let path = dir.join("claps-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

fn frame() -> Value {
    json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"}
        ]}]}]
    })
}

/// A UDP port free now.
fn free_udp() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Starts or stops SimLive's transport.
async fn transport(c: &mut Client, verb: &str) {
    let slots = c
        .cmd(
            "band",
            vec![json!({"target": "live_set", "name": verb, "args": []})],
        )
        .await
        .unwrap();
    assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
}

/// Reads the device's words until `check` takes one (5 s at most).
async fn word_until(device: &UdpSocket, what: &str, check: impl Fn(&Word) -> bool) -> Word {
    let deadline = Instant::now() + SECS_5;
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let got = tokio::time::timeout(left, device.recv(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("the device never got {what}"))
            .unwrap();
        let word = parse(&buf[..got]).expect("a word that reads");
        if check(&word) {
            return word;
        }
    }
}

/// The page's next claps state `check` takes.
async fn state_until(c: &mut Client, check: impl Fn(&ClapsState) -> bool) -> ClapsState {
    c.wait(SECS_5, |m| match m {
        ServerMsg::Claps(state) if check(state) => Some(state.clone()),
        _ => None,
    })
    .await
}

#[test]
fn a_page_taps_live_claps_the_beats_a_clip_silences_it_and_a_stop_ends_it() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("layout.json"),
            serde_json::to_vec(&frame()).unwrap(),
        )
        .unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let device = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let hub_port = free_udp();
        let mut config = Config::defaults(dir.path());
        config.instances = vec![host.cfg()];
        config.layout_poll_ms = 100;
        config.lufs.port = 0;
        config.listen.port = 0;
        config.claps.port = hub_port;
        config.claps.device_port = device.local_addr().unwrap().port();
        config.claps.notes = vec![52, 57];
        let hub = TestHub::start_config(config).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_5).await;

        // The +CLAPS track found, its clips read, the device told.
        let status = hub
            .status_until(SECS_5, |s| {
                s.claps.track.as_deref() == Some("band CLAPS") && s.claps.clips == 2
            })
            .await;
        assert_eq!(status.claps.active_clips, 1);
        assert_eq!(status.claps.signature.as_deref(), Some("4/4"));
        word_until(&device, "the pitches", |w| *w == Word::Notes(vec![52, 57])).await;
        word_until(&device, "nothing on", |w| *w == Word::On(false)).await;
        // Its hello.
        let hello = Hello {
            version: "v9.9.9".into(),
            running: false,
        };
        device
            .send_to(&encode_hello(&hello), ("127.0.0.1", hub_port))
            .await
            .unwrap();
        hub.status_until(SECS_5, |s| {
            s.claps.device.as_ref().map(|d| d.version.as_str()) == Some("v9.9.9")
        })
        .await;

        // Live plays; two taps half a second apart (120 bpm: a beat).
        transport(&mut a, "start_playing").await;
        state_until(&mut a, |s| s.beat.is_some()).await;
        a.send(&ClientMsg::ClapTap { t: 0.0 }).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        a.send(&ClientMsg::ClapTap { t: 0.0 }).await;
        state_until(&mut a, |s| s.state == ClapsPhase::Tapping).await;
        let taps = hub
            .events_until(SECS_5, |evs| {
                evs.iter()
                    .filter(|e| e["ev"] == "claps" && e["step"] == "tap")
                    .count()
                    == 2
            })
            .await;
        let beats: BTreeSet<usize> = taps
            .iter()
            .filter(|e| e["ev"] == "claps" && e["step"] == "tap")
            .map(|e| e["beat"].as_u64().unwrap() as usize - 1)
            .collect();
        assert!(!beats.is_empty());
        // A bar (2 s) with no tap: the pattern and the switch.
        let want: [bool; 4] = std::array::from_fn(|i| beats.contains(&i));
        word_until(&device, "the tapped pattern", |w| *w == Word::Pattern(want)).await;
        word_until(&device, "on", |w| *w == Word::On(true)).await;
        let on = state_until(&mut a, |s| {
            matches!(s.state, ClapsPhase::NextBar | ClapsPhase::On)
        })
        .await;
        assert_eq!(on.pattern, want);

        // A jump into the active clip: the device is silent, the page says
        // CLIP; past it, not.
        assert!(host.song_time(66.0) >= 66.0);
        word_until(&device, "silent", |w| *w == Word::Silent(true)).await;
        state_until(&mut a, |s| s.state == ClapsPhase::Clip).await;
        // The deactivated clip claps nothing: the device plays there.
        assert!(host.song_time(130.0) >= 130.0);
        word_until(&device, "not silent", |w| *w == Word::Silent(false)).await;
        state_until(&mut a, |s| s.state == ClapsPhase::On).await;

        // The stop (TAP held 1 s): off.
        a.send(&ClientMsg::ClapStop).await;
        word_until(&device, "off", |w| *w == Word::On(false)).await;
        state_until(&mut a, |s| s.state == ClapsPhase::Off).await;

        // Another time signature: the strip says 4/4 only, a tap is refused.
        assert_eq!(host.signature(3, 4), 3);
        state_until(&mut a, |s| s.state == ClapsPhase::Signature).await;
        a.send(&ClientMsg::ClapTap { t: 0.0 }).await;
        hub.events_until(SECS_5, |evs| {
            evs.iter()
                .any(|e| e["ev"] == "claps" && e["step"] == "refused" && e["why"] == "not 4/4")
        })
        .await;
        transport(&mut a, "stop_playing").await;
        hub.stop().await;
        host.stop();
    });
}
