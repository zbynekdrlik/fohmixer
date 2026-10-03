//! The setter of one Live instance (#43, design note §3.1): the clients'
//! writes (`set`) on their way to Live, latest-wins and bounded.
//!
//! - Per key (`instance|target|prop`) it keeps only the newest want. A set
//!   whose `seq` is not above the last one taken from that client for that
//!   key is old and dropped. When the slot holds another client's want, the
//!   newer arrival at the hub (`t_hub`) wins and the replaced client is told
//!   with a `superseded` ack.
//! - At most one batch is in flight: when nothing is, every pending want
//!   goes to Live as ONE `set_prop` batch. Its result acks every item to the
//!   client that sent it (Live's error, or the value Live reported), and the
//!   next batch takes what piled up meanwhile, one want per key. During a
//!   Live stall nothing queues but the newest want per key (L5).
//! - A batch that fails as a whole (timeout, offline) acks `error` to its
//!   senders; newer wants stay pending. An instance disconnect forgets the
//!   batch in flight and the pending wants without acks: they belong to the
//!   old Live session (the pages resend, L4).
//!
//! Pure: the router task owns one per instance, hands it the clock (hub ms)
//! and carries out its batches with `LiveHandle::call`.

use std::collections::{BTreeMap, HashMap};

use fohmixer_proto::client::AckItem;
use serde_json::{Value, json};

use crate::live::subs::ClientId;

/// One client's latest write of one key.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub client: ClientId,
    /// The LOM path as the client sent it.
    pub target: String,
    pub prop: String,
    pub value: Value,
    /// The client's sequence number of this set.
    pub seq: u64,
    /// The page's clock when it sent the set (ms).
    pub t_page: f64,
    /// The hub's clock when the set arrived (ms).
    pub t_hub: f64,
    /// A release, a toggle or a tap.
    pub is_final: bool,
}

/// A batch of wants written to Live as one `set_prop` request.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// The instance's batch number (1, 2, …).
    pub id: u64,
    /// When it was written (hub ms).
    pub sent_at: f64,
    /// The wants by key, in key order (the request's command order).
    pub items: Vec<(String, Want)>,
}

impl Batch {
    /// The script's commands of this batch: one `set_prop` per want.
    pub fn commands(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(|(_, want)| {
                json!({
                    "target": want.target,
                    "name": "set_prop",
                    "args": {"prop": want.prop, "value": want.value},
                })
            })
            .collect()
    }
}

/// What a `set` did.
#[derive(Debug, Clone, PartialEq)]
pub struct SetOutcome {
    /// Its `seq` was not above the client's last one for this key: dropped.
    pub dropped_old: bool,
    /// The time since this client's previous set of this key reached the
    /// hub (ms), when there was one.
    pub gap_ms: Option<f64>,
    /// A client whose want lost the slot to a newer one, and its ack.
    pub superseded: Option<(ClientId, AckItem)>,
}

/// What a batch's result did.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub batch: u64,
    /// How many wants it carried.
    pub n: usize,
    /// From its write to its result (ms).
    pub rtt_ms: f64,
    /// One ack per want, to the client that sent it.
    pub acks: Vec<(ClientId, AckItem)>,
    /// How many of those acks carry an error.
    pub errors: usize,
    /// It is the first batch with an error after one without (the hub log
    /// warns once per run of failures; the event log has every one).
    pub first_failure: bool,
}

/// The setter of one instance.
#[derive(Debug, Default)]
pub struct Setter {
    pending: BTreeMap<String, Want>,
    in_flight: Option<Batch>,
    /// The last `seq` taken from each client for each key, and when it
    /// arrived (hub ms).
    last: HashMap<(ClientId, String), (u64, f64)>,
    /// The last batch number.
    batches: u64,
    /// The last batch's result carried an error.
    failing: bool,
}

/// What a batch's failed acks say, for the hub log: only for the first
/// failed batch of a run (`None` when every want was applied, or the batch
/// before failed too).
pub fn batch_problem(applied: &Applied) -> Option<String> {
    if !applied.first_failure {
        return None;
    }
    let first = applied
        .acks
        .iter()
        .find_map(|(_, ack)| ack.error.as_deref())?;
    Some(format!(
        "{} of {} writes of batch {} failed: {first}",
        applied.errors, applied.n, applied.batch
    ))
}

/// The ack of one want after Live's result `slot` for it (`None`: the result
/// had no slot for it).
pub fn ack_for(key: &str, seq: u64, slot: Option<&Value>) -> AckItem {
    let Some(slot) = slot else {
        return AckItem::failed(key, seq, "no result");
    };
    if slot.get("ok") == Some(&Value::Bool(true)) {
        let value = slot.get("data").filter(|d| !d.is_null()).cloned();
        AckItem::applied(key, seq, value)
    } else {
        let error = slot
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("failed");
        AckItem::failed(key, seq, error)
    }
}

impl Setter {
    /// A set of `key` (`want` carries its client, `seq` and times).
    pub fn on_set(&mut self, key: &str, want: Want) -> SetOutcome {
        let id = (want.client, key.to_string());
        let previous = self.last.get(&id).copied();
        if previous.is_some_and(|(seq, _)| want.seq <= seq) {
            return SetOutcome {
                dropped_old: true,
                gap_ms: None,
                superseded: None,
            };
        }
        let gap_ms = previous.map(|(_, at)| want.t_hub - at);
        self.last.insert(id, (want.seq, want.t_hub));
        let superseded = match self.pending.get(key) {
            Some(held) if held.client != want.client && held.t_hub > want.t_hub => {
                // The slot's want arrived later: this one lost before it was
                // written.
                return SetOutcome {
                    dropped_old: false,
                    gap_ms,
                    superseded: Some((want.client, AckItem::superseded(key, want.seq))),
                };
            }
            Some(held) if held.client != want.client => {
                Some((held.client, AckItem::superseded(key, held.seq)))
            }
            _ => None,
        };
        self.pending.insert(key.to_string(), want);
        SetOutcome {
            dropped_old: false,
            gap_ms,
            superseded,
        }
    }

    /// The batch to write now, at `now` (hub ms): every pending want, when
    /// nothing is in flight. It is in flight until its result.
    pub fn next_batch(&mut self, now: f64) -> Option<Batch> {
        if self.in_flight.is_some() || self.pending.is_empty() {
            return None;
        }
        self.batches += 1;
        let batch = Batch {
            id: self.batches,
            sent_at: now,
            items: std::mem::take(&mut self.pending).into_iter().collect(),
        };
        self.in_flight = Some(batch.clone());
        Some(batch)
    }

    /// The result of batch `id` at `now`: Live's result slots, or why there
    /// are none. `None` for a batch no longer in flight (the instance
    /// disconnected since).
    pub fn on_result(
        &mut self,
        id: u64,
        outcome: &Result<Vec<Value>, String>,
        now: f64,
    ) -> Option<Applied> {
        let batch = self.in_flight.take_if(|b| b.id == id)?;
        let acks: Vec<(ClientId, AckItem)> = batch
            .items
            .iter()
            .enumerate()
            .map(|(i, (key, want))| {
                let ack = match outcome {
                    Ok(slots) => ack_for(key, want.seq, slots.get(i)),
                    Err(why) => AckItem::failed(key, want.seq, why),
                };
                (want.client, ack)
            })
            .collect();
        let errors = acks.iter().filter(|(_, a)| a.error.is_some()).count();
        let failed = errors > 0;
        let first_failure = failed && !self.failing;
        self.failing = failed;
        Some(Applied {
            batch: batch.id,
            n: batch.items.len(),
            rtt_ms: now - batch.sent_at,
            acks,
            errors,
            first_failure,
        })
    }

    /// The instance disconnected: the batch in flight and the pending wants
    /// are forgotten, without acks.
    pub fn on_disconnect(&mut self) {
        self.in_flight = None;
        self.pending.clear();
    }

    /// A client left: its sequence numbers are forgotten (its pending wants
    /// stay: they are the engineer's latest intent).
    pub fn drop_client(&mut self, client: ClientId) {
        self.last.retain(|(c, _), _| *c != client);
    }

    /// The pending wants, by key.
    pub fn pending(&self) -> &BTreeMap<String, Want> {
        &self.pending
    }

    /// The batch in flight.
    pub fn in_flight(&self) -> Option<&Batch> {
        self.in_flight.as_ref()
    }

    /// How many (client, key) sequence numbers are kept.
    pub fn tracked(&self) -> usize {
        self.last.len()
    }
}

#[cfg(test)]
#[path = "setter/tests.rs"]
mod tests;
