//! The client's liveness probe (the UI's watchdog, #8; its clock and round
//! trip since #43): `ping` is answered `pong` through the client's outbox,
//! as often as it is sent, with its number and time echoed and the hub's
//! clock. The socket's open and close, every ping, a page's `trace` and a
//! write to an unknown instance land in the event log. No Live host: this
//! runs on Windows too.

mod support;

use std::time::Duration;

use fohmixer_proto::client::{AckItem, ClientMsg, ServerMsg};
use serde_json::{Value, json};
use support::{TestHub, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);

fn of<'a>(records: &'a [Value], ev: &str) -> Vec<&'a Value> {
    records.iter().filter(|r| r["ev"] == ev).collect()
}

#[test]
fn every_ping_is_answered_pong_and_recorded_with_the_page_clock() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let mut client = hub.client().await;
        let page_start = 5_000.25;
        for n in 0..3_u32 {
            let t = page_start + f64::from(n) * 100.0;
            let rtt = (n > 0).then_some(20.0);
            client.send(&ClientMsg::Ping { n, t, rtt }).await;
            let (got_t, h) = client
                .wait(SECS_3, |m| match m {
                    ServerMsg::Pong { n: got, t, h } if *got == n => Some((*t, *h)),
                    _ => None,
                })
                .await;
            assert_eq!(got_t, t, "the page's time is echoed");
            let now = fohmixer_hub::live::wall_ms().unwrap();
            assert!(h <= now && now - h < 3_000.0, "the hub's clock: {h} {now}");
        }
        let records = hub.events_until(SECS_3, |r| of(r, "ping").len() == 3).await;
        let pings = of(&records, "ping");
        assert_eq!(pings[0]["n"], 0);
        assert_eq!(pings[0]["t"], page_start);
        assert_eq!(pings[0]["rtt"], Value::Null);
        assert_eq!(pings[0]["offset_ms"], Value::Null, "no round trip yet");
        assert_eq!(pings[0]["peer"], "127.0.0.1");
        // From the second ping on, the page clock's offset (Cristian): ping 1
        // carries ping 0's round trip, so it is ping 0's arrival less ping
        // 0's page time and half that round trip.
        let hub_ms = pings[0]["hub_ms"].as_f64().unwrap();
        let offset = pings[1]["offset_ms"].as_f64().unwrap();
        assert!(
            (offset - (hub_ms - (page_start + 10.0))).abs() < 1e-6,
            "{offset} {hub_ms}"
        );
        assert_eq!(pings[2]["rtt"], 20.0);
        // The socket's open is recorded with who opened it.
        let open = of(&records, "sock");
        assert_eq!(open[0]["what"], "open");
        assert_eq!(open[0]["source"], "lan");
        let client_id = open[0]["client"].clone();
        assert_eq!(pings[0]["client"], client_id);
        client.close().await;
        let records = hub
            .events_until(SECS_3, |r| {
                of(r, "sock").iter().any(|s| s["what"] == "close")
            })
            .await;
        let close = of(&records, "sock")
            .into_iter()
            .find(|s| s["what"] == "close")
            .unwrap()
            .clone();
        assert_eq!(close["client"], client_id);
        assert_eq!(close["reason"], "the client closed it");
        hub.stop().await;
    });
}

#[test]
fn a_trace_and_a_write_to_an_unknown_instance_land_in_the_event_log() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let mut client = hub.client().await;
        let dropout = json!({"ev": "dropout", "t": 9_000.5, "ms": 420.0,
                             "socket_lost": false, "rtts": [11.0, 12.5]});
        client
            .send(&ClientMsg::Trace {
                events: vec![dropout.clone()],
            })
            .await;
        let target = "live_set tracks 0 mixer_device volume";
        client
            .write("drums", target, "value", json!(0.5), 7, true)
            .await;
        let key = "drums|live_set tracks 0 mixer_device volume|value";
        assert_eq!(
            client.ack_of(key, 7, SECS_3).await,
            AckItem::failed(key, 7, "unknown instance \"drums\"")
        );
        // Once a ping with a round trip came, a write carries the page
        // clock's offset and its one-way delay.
        for (n, rtt) in [(0, None), (1, Some(4.0))] {
            client.send(&ClientMsg::Ping { n, t: 1_000.0, rtt }).await;
            client
                .wait(SECS_3, |m| {
                    matches!(m, ServerMsg::Pong { n: got, .. } if *got == n).then_some(())
                })
                .await;
        }
        client
            .write("drums", target, "value", json!(0.6), 8, false)
            .await;
        client.ack_of(key, 8, SECS_3).await;
        let records = hub
            .events_until(SECS_3, |r| {
                !of(r, "trace").is_empty() && of(r, "ack").len() == 2
            })
            .await;
        assert_eq!(of(&records, "trace")[0]["events"], json!([dropout]));
        let sets = of(&records, "set");
        assert_eq!(sets[0]["seq"], 7);
        assert_eq!(sets[0]["unknown"], json!(true));
        assert_eq!(sets[0]["t"], 1_007.0);
        assert_eq!(sets[0]["offset_ms"], Value::Null, "no ping yet");
        assert_eq!(sets[0]["delay_ms"], Value::Null);
        assert!(sets[0]["hub_ms"].as_f64().unwrap() > 0.0);
        assert_eq!(
            of(&records, "ack")[0]["error"],
            "unknown instance \"drums\""
        );
        let offset = of(&records, "ping")[1]["offset_ms"].as_f64().unwrap();
        assert_eq!(sets[1]["offset_ms"], offset);
        let hub_ms = sets[1]["hub_ms"].as_f64().unwrap();
        let delay = sets[1]["delay_ms"].as_f64().unwrap();
        assert!(
            (delay - (hub_ms - (1_008.0 + offset))).abs() < 1e-6,
            "{delay} {hub_ms} {offset}"
        );
        hub.stop().await;
    });
}
