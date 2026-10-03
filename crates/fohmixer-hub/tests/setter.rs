//! The clients' writes (#43, protocol 2) against the real FohMixer script on
//! SimLive (`sim/host.py`): a `set` reaches Live and is acked, every hop is
//! an event-log record in order (`set` → `batch` → `applied` → `ack`), a
//! stalled Live gets one batch per result with only the newest value (spec
//! §6.2 test 5), and a newer write of another client supersedes a pending
//! one.
#![cfg(unix)]

mod support;

use std::time::Duration;

use fohmixer_proto::client::{AckItem, set_key};
use serde_json::{Value, json};
use support::{Host, TestHub, runtime, serial};

const VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
const SECS_5: Duration = Duration::from_secs(5);

fn key() -> String {
    set_key("band", VOLUME, "value")
}

/// The index of the first record after `from` that `pick` accepts.
fn index_after(records: &[Value], from: usize, pick: impl Fn(&Value) -> bool) -> Option<usize> {
    records
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, r)| pick(r))
        .map(|(i, _)| i)
}

/// The batches of `band` that carried `key`.
fn batches_of(records: &[Value], key: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| {
            r["ev"] == "batch"
                && r["instance"] == "band"
                && r["sent"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|i| i["key"] == key))
        })
        .cloned()
        .collect()
}

#[test]
fn a_write_reaches_live_and_every_hop_is_recorded_in_order() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_5).await;
        a.write("band", VOLUME, "value", json!(0.5), 1, true).await;
        let key = key();
        assert_eq!(
            a.ack_of(&key, 1, SECS_5).await,
            AckItem::applied(&key, 1, None)
        );
        assert_eq!(a.get("band", VOLUME, "value").await, json!(0.5));
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter().any(|x| x["ev"] == "ack" && x["seq"] == 1)
            })
            .await;
        let set = index_after(&records, 0, |r| r["ev"] == "set" && r["seq"] == 1).unwrap();
        assert_eq!(records[set]["key"], key.as_str());
        assert_eq!(records[set]["final"], json!(true));
        assert_eq!(records[set]["t"], 1_001.0);
        let batch = index_after(&records, set, |r| {
            r["ev"] == "batch" && r["sent"][0]["seq"] == 1
        })
        .expect("a batch after the set");
        let id = records[batch]["batch"].clone();
        let applied = index_after(&records, batch, |r| {
            r["ev"] == "applied" && r["batch"] == id
        })
        .expect("its result after the batch");
        assert_eq!(records[applied]["errors"], 0);
        assert!(records[applied]["rtt_ms"].as_f64().unwrap() >= 0.0);
        let ack = index_after(&records, applied, |r| r["ev"] == "ack" && r["seq"] == 1)
            .expect("the ack after the result");
        assert_eq!(records[ack]["batch"], id);
        assert_eq!(records[ack]["error"], Value::Null);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_stalled_live_gets_two_batches_for_sixty_writes_and_ends_at_the_last() {
    // Spec §6.2 test 5, end to end: while Live's main thread stalls, the
    // first write is in flight and the 59 after it wait as ONE want.
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_5).await;
        host.stall(1000);
        // The stall has begun once the hub calls the instance busy (300 ms
        // without a heartbeat): 700 ms are left for the 60 writes.
        a.instance_state("band", true, Some(true), SECS_5).await;
        for seq in 1..=60_u64 {
            a.write(
                "band",
                VOLUME,
                "value",
                json!(seq as f64 / 100.0),
                seq,
                seq == 60,
            )
            .await;
        }
        let key = key();
        assert_eq!(
            a.ack_of(&key, 60, SECS_5).await,
            AckItem::applied(&key, 60, None)
        );
        let live = a.get("band", VOLUME, "value").await.as_f64().unwrap();
        assert!(
            (live - 0.6).abs() < 1e-9,
            "Live ends at the last write: {live}"
        );
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter().any(|x| x["ev"] == "ack" && x["seq"] == 60)
            })
            .await;
        let batches = batches_of(&records, &key);
        assert_eq!(batches.len(), 2, "{batches:?}");
        assert_eq!(batches[0]["sent"][0]["seq"], 1);
        assert_eq!(batches[1]["sent"][0]["seq"], 60);
        let sets = records
            .iter()
            .filter(|r| r["ev"] == "set" && r["key"] == key.as_str())
            .count();
        assert_eq!(sets, 60, "every write is recorded");
        // The first batch waited out the stall.
        let first = records
            .iter()
            .find(|r| r["ev"] == "applied" && r["batch"] == batches[0]["batch"])
            .unwrap();
        assert!(first["rtt_ms"].as_f64().unwrap() >= 300.0, "{first}");
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_newer_write_of_another_client_supersedes_a_pending_one() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_5).await;
        host.stall(1000);
        a.instance_state("band", true, Some(true), SECS_5).await;
        let key = key();
        // A's first write is in flight (Live stalls), its second waits…
        a.write("band", VOLUME, "value", json!(0.3), 1, false).await;
        a.write("band", VOLUME, "value", json!(0.4), 2, true).await;
        // …until B's later write takes the slot: A hears it was superseded.
        tokio::time::sleep(Duration::from_millis(50)).await;
        b.write("band", VOLUME, "value", json!(0.7), 1, true).await;
        assert_eq!(
            a.ack_of(&key, 2, SECS_5).await,
            AckItem::superseded(&key, 2)
        );
        assert_eq!(
            a.ack_of(&key, 1, SECS_5).await,
            AckItem::applied(&key, 1, None)
        );
        assert_eq!(
            b.ack_of(&key, 1, SECS_5).await,
            AckItem::applied(&key, 1, None)
        );
        let live = a.get("band", VOLUME, "value").await.as_f64().unwrap();
        assert!((live - 0.7).abs() < 1e-9, "B's value wins: {live}");
        hub.stop().await;
        host.stop();
    });
}
