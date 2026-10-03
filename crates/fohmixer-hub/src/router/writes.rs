//! The router's half of the clients' writes (#43, design note §3.1): it
//! hands each `set` to its instance's setter, writes the setter's batches
//! with `LiveHandle::call`, takes their results back as
//! [`RouterMsg::Applied`], acks each write to its client, and records every
//! hop in the event log (`set`, `batch`, `applied`, `ack`).
//!
//! A `set` record carries the page's send time (`t`), the hub's arrival
//! (`hub_ms`), the page clock's offset and the one-way delay they give, and
//! the gap since that client's previous set of the key: a stall on the way
//! shows as a gap and a delay spike on the moves after it. `applied` and a
//! batch's `ack`s carry the batch number, its sequence numbers and Live's
//! round trip.

use fohmixer_proto::client::{AckItem, set_key};
use serde_json::{Value, json};

use super::{Router, RouterMsg};
use crate::clock::one_way_delay;
use crate::live::subs::ClientId;
use crate::setter::{SetOutcome, Want, batch_problem};

/// A client's write as the router takes it.
pub(super) struct SetMsg {
    pub client: ClientId,
    pub instance: String,
    pub target: String,
    pub prop: String,
    pub value: Value,
    pub seq: u64,
    pub t: f64,
    pub is_final: bool,
    /// When it reached the hub (hub UTC ms).
    pub hub_ms: f64,
    /// The page clock's offset (hub − page, ms) as of the socket's last
    /// ping.
    pub offset_ms: Option<f64>,
}

/// The event-log fields of a `set` (`outcome`: what the setter did with
/// it; none for an unknown instance).
pub(super) fn set_fields(
    set: &SetMsg,
    key: &str,
    peer: Option<&str>,
    outcome: Option<&SetOutcome>,
) -> Value {
    json!({
        "client": set.client,
        "peer": peer,
        "instance": set.instance,
        "key": key,
        "seq": set.seq,
        "value": set.value,
        "final": set.is_final,
        "t": set.t,
        "hub_ms": set.hub_ms,
        "offset_ms": set.offset_ms,
        "delay_ms": one_way_delay(set.hub_ms, set.t, set.offset_ms),
        "gap_ms": outcome.and_then(|o| o.gap_ms),
        "dropped_old": outcome.is_some_and(|o| o.dropped_old),
        "unknown": outcome.is_none(),
    })
}

/// The error of a write to an instance the hub does not have.
pub fn unknown_instance(instance: &str) -> String {
    format!("unknown instance {instance:?}")
}

/// The event-log fields of an ack; `batch`: the batch it answers and Live's
/// round trip of it (none for an ack without a batch).
pub fn ack_fields(
    client: ClientId,
    peer: Option<&str>,
    instance: &str,
    ack: &AckItem,
    batch: Option<(u64, f64)>,
) -> Value {
    json!({
        "client": client,
        "peer": peer,
        "instance": instance,
        "key": ack.key,
        "seq": ack.seq,
        "value": ack.value,
        "error": ack.error,
        "superseded": ack.superseded,
        "batch": batch.map(|(id, _)| id),
        "rtt_ms": batch.map(|(_, rtt)| rtt),
    })
}

impl Router {
    /// The setters' clock: ms since the router started (monotonic).
    pub(super) fn now_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn peer(&self, client: ClientId) -> Option<&str> {
        self.peers.get(&client).map(String::as_str)
    }

    /// A client's `set`: its record, its setter, and a batch when none is
    /// in flight.
    pub(super) fn on_set(&mut self, set: SetMsg) {
        let key = set_key(&set.instance, &set.target, &set.prop);
        let now = self.now_ms();
        let want = Want {
            client: set.client,
            target: set.target.clone(),
            prop: set.prop.clone(),
            value: set.value.clone(),
            seq: set.seq,
            t_page: set.t,
            t_hub: now,
            is_final: set.is_final,
        };
        let Some(setter) = self.setters.get_mut(&set.instance) else {
            let ack = AckItem::failed(&key, set.seq, &unknown_instance(&set.instance));
            let fields = set_fields(&set, &key, self.peer(set.client), None);
            self.io.events.record("set", fields);
            self.send_ack(&set.instance, set.client, ack, None);
            return;
        };
        let outcome = setter.on_set(&key, want);
        let fields = set_fields(&set, &key, self.peer(set.client), Some(&outcome));
        self.io.events.record("set", fields);
        if let Some((client, ack)) = outcome.superseded {
            self.send_ack(&set.instance, client, ack, None);
        }
        self.write_next(&set.instance);
    }

    /// Writes an instance's next batch, when it has one and none is in
    /// flight; its result comes back as [`RouterMsg::Applied`].
    pub(super) fn write_next(&mut self, instance: &str) {
        let now = self.now_ms();
        let (Some(live), Some(setter)) = (self.live.get(instance), self.setters.get_mut(instance))
        else {
            return;
        };
        let Some(batch) = setter.next_batch(now) else {
            return;
        };
        let sent: Vec<Value> = batch
            .items
            .iter()
            .map(|(key, want)| {
                json!({"key": key, "client": want.client, "seq": want.seq, "value": want.value})
            })
            .collect();
        self.io.events.record(
            "batch",
            json!({"instance": instance, "batch": batch.id, "n": batch.items.len(), "sent": sent}),
        );
        let result = live.call(batch.commands());
        let tx = self.io.tx.clone();
        let instance = instance.to_string();
        let id = batch.id;
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::Applied {
                instance,
                batch: id,
                outcome,
            });
        });
    }

    /// A batch's result: its record, an ack per write, then the next batch.
    pub(super) fn on_applied(
        &mut self,
        instance: &str,
        batch: u64,
        outcome: &Result<Vec<Value>, String>,
    ) {
        let now = self.now_ms();
        let Some(applied) = self
            .setters
            .get_mut(instance)
            .and_then(|s| s.on_result(batch, outcome, now))
        else {
            tracing::debug!(instance, batch, "the result of a batch no longer in flight");
            return;
        };
        let sent: Vec<Value> = applied
            .acks
            .iter()
            .map(|(client, ack)| json!({"key": ack.key, "client": client, "seq": ack.seq}))
            .collect();
        self.io.events.record(
            "applied",
            json!({"instance": instance, "batch": applied.batch, "n": applied.n,
                   "rtt_ms": applied.rtt_ms, "errors": applied.errors, "sent": sent}),
        );
        if let Some(problem) = batch_problem(&applied) {
            tracing::warn!(instance, rtt_ms = applied.rtt_ms, "{problem}");
        }
        let round = Some((applied.batch, applied.rtt_ms));
        for (client, ack) in applied.acks {
            self.send_ack(instance, client, ack, round);
        }
        self.write_next(instance);
    }

    /// Records an ack and hands it to its client (gone: recorded only).
    pub(super) fn send_ack(
        &self,
        instance: &str,
        client: ClientId,
        ack: AckItem,
        batch: Option<(u64, f64)>,
    ) {
        self.io.events.record(
            "ack",
            ack_fields(client, self.peer(client), instance, &ack, batch),
        );
        if let Some(outbox) = self.clients.get(&client) {
            outbox.ack(ack);
        }
    }
}
