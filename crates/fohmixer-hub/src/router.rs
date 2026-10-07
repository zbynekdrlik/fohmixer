//! The hub's router: one task that owns the subscription table, the client
//! outboxes, the setters and the STAGE AUT rule (S3 design note §3, §4, §6).
//!
//! Everything reaches it as a message, in order: the instances' events (a
//! subscription's result stays ordered with the value pushes of the same
//! connection), the clients' requests, layout changes. It never waits: it
//! writes into outboxes and queues requests to the instances' tasks. It also
//! runs the layout's check for unresolved names (spec §2.5 D4) when a layout
//! is accepted and when an instance connects.
//!
//! The clients' writes (#43, `set`) go through one setter per instance
//! (`setter.rs`, driven by `writes.rs`): one batch in flight, the latest
//! want per key, acks per client. Every hop is an event-log record
//! (`events.rs`); Live's health goes to every client as `link`.
//!
//! The Stream Deck (#52, `deck.rs`): the router owns its state too; the
//! Companion task's events come as messages, and `router/deck.rs` carries
//! the decisions out.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use fohmixer_proto::client::{HUB_STAGE_AUT, ServerMsg, StageAutStatus, Unresolved, ValueItem};
use fohmixer_proto::layout::Binding;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::events::EventLog;
use crate::live::client::{LiveError, LiveEvent, LiveHandle};
use crate::live::names::NameCheck;
use crate::live::subs::{Cached, ClientId, Outgoing, Subs};
use crate::outbox::Outbox;
use crate::rules::{HubState, StageAut};
use crate::setter::Setter;

#[path = "router/writes.rs"]
mod writes;

#[path = "router/deck.rs"]
mod deck;
mod unfold;

/// The router's own subscriber: the STAGE AUT rule (clients start at 1).
pub const STAGE_CLIENT: ClientId = 0;
/// The router's subscriber that keeps the strips' groups unfolded (#58;
/// client numbers count up from 1 and never reach it).
pub const UNFOLD_CLIENT: ClientId = ClientId::MAX;

/// A message to the router.
pub enum RouterMsg {
    /// An event of an instance's task.
    Live {
        instance: String,
        event: LiveEvent,
    },
    /// A client connected: its outbox and its peer's address.
    Attach {
        client: ClientId,
        outbox: Arc<Outbox>,
        peer: String,
    },
    Sub {
        client: ClientId,
        instance: String,
        target: String,
        prop: String,
        display: bool,
    },
    Unsub {
        client: ClientId,
        sub: String,
    },
    SetHub {
        client: ClientId,
        key: String,
        value: Value,
    },
    /// A client's write (#43, `set`), with when it reached the hub (hub UTC
    /// ms) and the page clock's offset as of its socket's last ping.
    Set {
        client: ClientId,
        instance: String,
        target: String,
        prop: String,
        value: Value,
        seq: u64,
        t: f64,
        is_final: bool,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// The result of an instance's batch of writes (`Err`: why it has
    /// none: offline, a timeout, refused).
    Applied {
        instance: String,
        batch: u64,
        outcome: Result<Vec<Value>, String>,
    },
    /// The answer to a read of the unfold keeper (#58).
    UnfoldRead {
        instance: String,
        seq: u64,
        outcome: Result<Vec<Value>, String>,
    },
    /// A failed read of the unfold keeper is due again (#58).
    UnfoldRetry {
        instance: String,
    },
    /// A client's connection ended.
    Detach {
        client: ClientId,
    },
    /// A new layout is served: its revision, its STAGE AUT binding and its
    /// check targets per instance (for the unresolved-names check: the
    /// bindings, `live::names::layout_targets`) and its strips' tracks.
    Layout {
        rev: u64,
        stage: Option<Binding>,
        targets: BTreeMap<String, Vec<String>>,
        /// The strips' tracks (`Layout::strip_tracks`, #58).
        strips: Vec<(String, String)>,
    },
    /// The router's part of `/api/status`.
    Status {
        reply: oneshot::Sender<RouterStatus>,
    },
    /// A page opened or closed the Stream Deck tab (#52).
    DeckView {
        client: ClientId,
        on: bool,
    },
    /// A page's Stream Deck press (#52), with its arrival (hub UTC ms) and
    /// the page clock's offset as of its socket's last ping.
    DeckPress {
        client: ClientId,
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        hold_ms: Option<f64>,
        why: Option<String>,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// Something came from a client (#52: a holding page silent for 2 s is
    /// released).
    Heard {
        client: ClientId,
    },
    /// An event of the Companion task (#52).
    Deck {
        event: crate::companion::CompanionEvent,
    },
    /// The Stream Deck's clock (#52, every `crate::deck::TICK` with
    /// `[companion]`).
    Tick,
    /// The hub stops: the Stream Deck's held keys released and its task
    /// told to stop (#52), then every client closed.
    Stop,
}

/// The router's part of `/api/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterStatus {
    pub subscriptions: BTreeMap<String, usize>,
    /// The groups the unfold keeper holds, per instance (#58).
    pub unfolded: BTreeMap<String, Vec<String>>,
    pub listeners: BTreeMap<String, usize>,
    pub stage_aut: StageAutStatus,
    pub clients: usize,
    pub unresolved: Vec<Unresolved>,
}

#[derive(Debug, Clone, Default)]
struct InstanceState {
    online: bool,
    busy: bool,
    set_name: String,
}

/// Where STAGE AUT acts: the stage-mic track's mute and the transport
/// subscription that drives it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StageTarget {
    instance: String,
    target: String,
    sub: String,
}

/// What the router needs besides its instances: its own sender (a batch's
/// result comes back as a message) and the event log.
pub struct RouterIo {
    pub tx: mpsc::UnboundedSender<RouterMsg>,
    pub events: EventLog,
}

/// The router task's state.
pub struct Router {
    subs: Subs,
    names: NameCheck,
    clients: HashMap<ClientId, Arc<Outbox>>,
    /// Each client's peer address (event-log records).
    peers: HashMap<ClientId, String>,
    live: BTreeMap<String, LiveHandle>,
    states: BTreeMap<String, InstanceState>,
    setters: BTreeMap<String, Setter>,
    stage: StageAut,
    stage_target: Option<StageTarget>,
    layout_rev: u64,
    data_dir: PathBuf,
    io: RouterIo,
    /// The setters' clock (monotonic, ms since the router started).
    started: std::time::Instant,
    /// The Stream Deck (#52); none without `[companion]`.
    deck: Option<deck::DeckIo>,
    /// Keeps the strips' groups unfolded (#58).
    unfold: unfold::Keeper,
}

impl Router {
    /// A router over these instances, STAGE AUT persisted as `stage_aut`.
    pub fn new(
        live: BTreeMap<String, LiveHandle>,
        stage_aut: bool,
        data_dir: PathBuf,
        io: RouterIo,
    ) -> Self {
        Self {
            subs: Subs::new(live.keys().cloned()),
            names: NameCheck::default(),
            clients: HashMap::new(),
            peers: HashMap::new(),
            states: live
                .keys()
                .map(|k| (k.clone(), InstanceState::default()))
                .collect(),
            setters: live
                .keys()
                .map(|k| (k.clone(), Setter::default()))
                .collect(),
            live,
            stage: StageAut::new(stage_aut),
            stage_target: None,
            layout_rev: 0,
            data_dir,
            io,
            started: std::time::Instant::now(),
            deck: None,
            unfold: unfold::Keeper::default(),
        }
    }

    /// Serves messages until [`RouterMsg::Stop`], then releases the Stream
    /// Deck's held keys and tells its task to stop (#52), and closes every
    /// client (it holds a sender of its own for its batches' results, so the
    /// channel never ends by itself: the hub's stop sends `Stop`).
    pub async fn run(mut self, mut rx: mpsc::UnboundedReceiver<RouterMsg>) {
        while let Some(msg) = rx.recv().await {
            let go_on = self.handle(msg);
            self.flush();
            if !go_on {
                break;
            }
        }
        self.deck_stop();
        for outbox in self.clients.values() {
            outbox.close();
        }
        tracing::info!("router stopped");
    }

    fn handle(&mut self, msg: RouterMsg) -> bool {
        match msg {
            RouterMsg::Live { instance, event } => self.live_event(&instance, event),
            RouterMsg::Attach {
                client,
                outbox,
                peer,
            } => {
                for name in self.states.keys() {
                    self.send_instance(&outbox, name);
                }
                outbox.hub(HUB_STAGE_AUT, json!(self.stage.is_on()));
                if self.layout_rev > 0 {
                    outbox.layout(self.layout_rev);
                }
                self.deck_attach(client, &outbox);
                self.clients.insert(client, outbox);
                self.peers.insert(client, peer);
            }
            RouterMsg::Sub {
                client,
                instance,
                target,
                prop,
                display,
            } => {
                let reply = match self
                    .subs
                    .subscribe(client, &instance, &target, &prop, display)
                {
                    Ok(reply) => subbed(&reply.key, reply.cached.as_ref()),
                    Err((key, error)) => subbed(&key, Some(&Cached::Error(error))),
                };
                if let Some(outbox) = self.clients.get(&client) {
                    outbox.reply(reply);
                }
            }
            RouterMsg::Unsub { client, sub } => {
                self.subs.unsubscribe(client, &sub);
                if let Some(outbox) = self.clients.get(&client) {
                    outbox.forget(&sub);
                }
            }
            RouterMsg::SetHub { client, key, value } => self.set_hub(client, &key, &value),
            RouterMsg::Set {
                client,
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
                hub_ms,
                offset_ms,
            } => self.on_set(writes::SetMsg {
                client,
                instance,
                target,
                prop,
                value,
                seq,
                t,
                is_final,
                hub_ms,
                offset_ms,
            }),
            RouterMsg::UnfoldRead {
                instance,
                seq,
                outcome,
            } => {
                if let Err(error) = &outcome {
                    tracing::warn!(instance = %instance, error = %error, "the hub could not read the groups of the strips' tracks");
                }
                let actions = self.unfold.read_done(&instance, seq, &outcome);
                self.unfold_apply(actions);
            }
            RouterMsg::UnfoldRetry { instance } => {
                let actions = self.unfold.retry(&instance);
                self.unfold_apply(actions);
            }
            RouterMsg::Applied {
                instance,
                batch,
                outcome,
            } => self.on_applied(&instance, batch, &outcome),
            RouterMsg::Detach { client } => {
                self.deck_detach(client);
                self.subs.drop_client(client);
                self.clients.remove(&client);
                self.peers.remove(&client);
                for setter in self.setters.values_mut() {
                    setter.drop_client(client);
                }
            }
            RouterMsg::Layout {
                rev,
                stage,
                targets,
                strips,
            } => {
                self.layout_rev = rev;
                for outbox in self.clients.values() {
                    outbox.layout(rev);
                }
                self.set_stage_binding(stage.as_ref());
                let actions = self.unfold.set_strips(strips);
                self.unfold_apply(actions);
                self.names.set_targets(targets);
                let online: Vec<String> = self
                    .states
                    .iter()
                    .filter(|(_, state)| state.online)
                    .map(|(name, _)| name.clone())
                    .collect();
                for name in online {
                    self.names.start(&name);
                }
            }
            RouterMsg::Status { reply } => {
                let names = self.live.keys();
                let _ = reply.send(RouterStatus {
                    subscriptions: names
                        .clone()
                        .map(|n| (n.clone(), self.subs.subscriptions_besides(n, UNFOLD_CLIENT)))
                        .collect(),
                    unfolded: unfold::by_instance(self.unfold.held()),
                    listeners: names.map(|n| (n.clone(), self.subs.listeners(n))).collect(),
                    stage_aut: StageAutStatus {
                        on: self.stage.is_on(),
                        writes: self.stage.writes(),
                    },
                    clients: self.clients.len(),
                    unresolved: self.names.unresolved(),
                });
            }
            RouterMsg::DeckView { client, on } => self.deck_view(client, on),
            RouterMsg::DeckPress {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            } => self.deck_press(deck::PressMsg {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            }),
            RouterMsg::Heard { client } => self.deck_heard(client),
            RouterMsg::Deck { event } => self.deck_event(event),
            RouterMsg::Tick => self.deck_tick(),
            RouterMsg::Stop => return false,
        }
        true
    }

    fn live_event(&mut self, instance: &str, event: LiveEvent) {
        match event {
            LiveEvent::Connected(info) => {
                if let Some(state) = self.states.get_mut(instance) {
                    *state = InstanceState {
                        online: true,
                        busy: false,
                        set_name: info.set_name,
                    };
                }
                self.subs.connected(instance);
                self.names.start(instance);
                self.broadcast_instance(instance);
            }
            LiveEvent::Disconnected => {
                if let Some(state) = self.states.get_mut(instance) {
                    *state = InstanceState::default();
                }
                if let Some(setter) = self.setters.get_mut(instance) {
                    setter.on_disconnect();
                }
                self.subs.disconnected(instance);
                self.names.stop(instance);
                if self
                    .stage_target
                    .as_ref()
                    .is_some_and(|t| t.instance == instance)
                {
                    self.stage.on_disconnect();
                }
                self.broadcast_instance(instance);
            }
            LiveEvent::Busy {
                busy,
                changed,
                tick_age_ms,
                reason,
            } => {
                if changed {
                    if let Some(state) = self.states.get_mut(instance) {
                        state.busy = busy;
                    }
                    self.broadcast_instance(instance);
                    self.io.events.record(
                        "link",
                        json!({"instance": instance, "busy": busy,
                               "tick_age_ms": tick_age_ms, "reason": reason}),
                    );
                }
                for outbox in self.clients.values() {
                    outbox.link(ServerMsg::Link {
                        instance: instance.to_string(),
                        tick_age_ms,
                        busy,
                    });
                }
            }
            LiveEvent::Result { uuid, data } => {
                if !self.subs.on_result(instance, &uuid, &data) {
                    self.names.on_result(instance, &uuid, &data);
                }
            }
            LiveEvent::Values(items) => self.subs.on_values(instance, &items),
        }
    }

    /// An instance's state for one client.
    fn send_instance(&self, outbox: &Outbox, name: &str) {
        let state = self.states.get(name).cloned().unwrap_or_default();
        outbox.instance(
            name,
            state.online,
            ServerMsg::Instance {
                name: name.to_string(),
                online: state.online,
                busy: state.busy,
                set_name: state.set_name,
            },
        );
    }

    fn broadcast_instance(&self, name: &str) {
        for outbox in self.clients.values() {
            self.send_instance(outbox, name);
        }
    }

    fn set_hub(&mut self, client: ClientId, key: &str, value: &Value) {
        let (true, Some(on)) = (key == HUB_STAGE_AUT, value.as_bool()) else {
            if let Some(outbox) = self.clients.get(&client) {
                outbox.reply(ServerMsg::Error {
                    id: None,
                    message: format!("no hub value {key:?} taking {value}"),
                });
            }
            return;
        };
        let changed = on != self.stage.is_on();
        let write = self.stage.set_flag(on);
        if changed {
            tracing::info!(on, "STAGE AUT turned {}", if on { "on" } else { "off" });
            if let Err(e) = (HubState { stage_aut: on }).save(&self.data_dir) {
                tracing::error!(error = %e, "cannot save the STAGE AUT flag");
            }
        }
        for outbox in self.clients.values() {
            outbox.hub(HUB_STAGE_AUT, json!(on));
        }
        if let Some(mute) = write {
            self.write_stage(mute);
        }
    }

    /// Follows a new layout's STAGE AUT binding: leaves the old transport
    /// subscription and starts afresh on the new one.
    fn set_stage_binding(&mut self, binding: Option<&Binding>) {
        let wanted = binding.and_then(|b| b.target().ok().map(|t| (b.instance.clone(), t)));
        let current = self
            .stage_target
            .as_ref()
            .map(|t| (t.instance.clone(), t.target.clone()));
        if wanted == current {
            return;
        }
        if let Some(old) = self.stage_target.take() {
            self.subs.unsubscribe(STAGE_CLIENT, &old.sub);
        }
        self.stage.on_disconnect();
        let Some((instance, target)) = wanted else {
            tracing::info!("the layout has no STAGE AUT binding");
            return;
        };
        match self
            .subs
            .subscribe(STAGE_CLIENT, &instance, "live_set", "is_playing", false)
        {
            Ok(reply) => {
                tracing::info!(instance = %instance, target = %target, "STAGE AUT follows the transport");
                self.stage_target = Some(StageTarget {
                    instance,
                    target,
                    sub: reply.key,
                });
                if let Some(Cached::Value { value, .. }) = reply.cached {
                    self.stage_value(&value);
                }
            }
            Err((_, error)) => {
                tracing::warn!(instance = %instance, error = %error, "STAGE AUT cannot follow the transport");
            }
        }
    }

    /// A transport value for the rule.
    fn stage_value(&mut self, value: &Value) {
        if let Some(playing) = value.as_bool()
            && let Some(mute) = self.stage.on_playing(playing)
        {
            self.write_stage(mute);
        }
    }

    /// Writes the stage mics' mute (logged; a failure is logged, never
    /// retried — spec I6).
    fn write_stage(&self, mute: bool) {
        let Some(target) = &self.stage_target else {
            return;
        };
        let Some(live) = self.live.get(&target.instance) else {
            return;
        };
        tracing::info!(instance = %target.instance, target = %target.target, mute, "STAGE AUT writes the stage mics' mute");
        let result = live.call(vec![json!({
            "target": target.target,
            "name": "set_prop",
            "args": {"prop": "mute", "value": mute},
        })]);
        let what = target.target.clone();
        tokio::spawn(async move {
            if let Some(problem) = write_failure(&result.await) {
                tracing::warn!(target = %what, problem = %problem, "STAGE AUT write failed");
            }
        });
    }

    /// Carries out the unfold keeper's actions (#58): its subscriptions
    /// first, then the values they already held. Those ask only for reads
    /// and writes (`Keeper::value`), so two steps do it, with no loop a
    /// mutant could make endless.
    fn unfold_apply(&mut self, actions: Vec<unfold::Action>) {
        let cached = self.unfold_do(actions);
        let mut next = Vec::new();
        for (key, state) in cached {
            next.extend(self.unfold.value(&key, cached_value(&state)));
        }
        self.unfold_do(next);
    }

    /// Carries out actions; the subscriptions' cached states, by key.
    fn unfold_do(&mut self, actions: Vec<unfold::Action>) -> Vec<(String, Cached)> {
        let mut cached: Vec<(String, Cached)> = Vec::new();
        for action in actions {
            match action {
                unfold::Action::Sub(watch) => {
                    let (instance, target, prop) = watch.target();
                    let instance = instance.to_string();
                    match self
                        .subs
                        .subscribe(UNFOLD_CLIENT, &instance, &target, prop, false)
                    {
                        Ok(reply) => {
                            self.unfold.subscribed(watch, reply.key.clone());
                            if let Some(state) = reply.cached {
                                cached.push((reply.key, state));
                            }
                        }
                        Err((_, error)) => {
                            tracing::warn!(instance = %instance, target = %target, error = %error, "the hub cannot keep the strips' groups unfolded there");
                        }
                    }
                }
                unfold::Action::Unsub { key } => {
                    self.subs.unsubscribe(UNFOLD_CLIENT, &key);
                }
                unfold::Action::Read {
                    instance,
                    seq,
                    commands,
                } => self.unfold_read(instance, seq, commands),
                unfold::Action::Unfold {
                    instance,
                    target,
                    group,
                } => self.unfold_group(&instance, target, &group),
                unfold::Action::Retry { instance } => self.unfold_retry(instance),
            }
        }
        cached
    }

    /// Reads the groups the strips' tracks sit in; the answer comes back as
    /// [`RouterMsg::UnfoldRead`].
    fn unfold_read(&self, instance: String, seq: u64, commands: Vec<Value>) {
        let Some(live) = self.live.get(&instance) else {
            return;
        };
        let result = live.call(commands);
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::UnfoldRead {
                instance,
                seq,
                outcome,
            });
        });
    }

    /// Asks the keeper to read `instance` again after `RETRY_MS`.
    fn unfold_retry(&self, instance: String) {
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(unfold::RETRY_MS)).await;
            let _ = tx.send(RouterMsg::UnfoldRetry { instance });
        });
    }

    /// Unfolds a group a strip's track sits in, through the path it was
    /// read by (logged; a failure is logged, and the next read tries again).
    fn unfold_group(&self, instance: &str, target: String, group: &str) {
        let Some(live) = self.live.get(instance) else {
            return;
        };
        tracing::info!(instance = %instance, group = %group, "the hub unfolds a group a strip's track sits in (Live meters no track inside a folded group)");
        let result = live.call(vec![json!({
            "target": target,
            "name": "set_prop",
            "args": {"prop": unfold::FOLD, "value": false},
        })]);
        tokio::spawn(async move {
            if let Some(problem) = write_failure(&result.await) {
                tracing::warn!(target = %target, problem = %problem, "the unfold failed");
            }
        });
    }

    /// Sends the table's and the name check's requests.
    fn send_requests(&mut self) {
        let requests: Vec<Outgoing> = self
            .subs
            .drain_outgoing()
            .into_iter()
            .chain(self.names.drain_outgoing())
            .collect();
        for out in requests {
            if let Some(live) = self.live.get(&out.instance) {
                live.send(out.uuid, out.commands);
            }
        }
    }

    /// Sends the requests and hands out the table's values (the unfold
    /// keeper's to the keeper: they ask only for reads and writes, which go
    /// out on their own).
    fn flush(&mut self) {
        self.send_requests();
        let mut unfold = Vec::new();
        for (client, item) in self.subs.take_deliveries() {
            if client == STAGE_CLIENT {
                if let Some(value) = &item.value {
                    self.stage_value(value);
                }
            } else if client == UNFOLD_CLIENT {
                if let Some(error) = &item.error {
                    tracing::warn!(key = %item.sub, error = %error, "a list the hub's unfold keeper listens to failed");
                }
                unfold.extend(self.unfold.value(&item.sub, item.value.as_ref()));
            } else if let Some(outbox) = self.clients.get(&client) {
                outbox.value(item);
            }
        }
        self.unfold_apply(unfold);
    }
}

/// A cached state's value for the unfold keeper (none: an error).
fn cached_value(state: &Cached) -> Option<&Value> {
    match state {
        Cached::Value { value, .. } => Some(value),
        Cached::Error(_) => None,
    }
}

/// Why a STAGE AUT write did not happen, if it did not.
fn write_failure(outcome: &Result<Vec<Value>, LiveError>) -> Option<String> {
    match outcome {
        Ok(slots) if slots.first().and_then(|s| s.get("ok")) == Some(&json!(true)) => None,
        Ok(slots) => Some(Value::from(slots.clone()).to_string()),
        Err(e) => Some(e.to_string()),
    }
}

/// The `subbed` reply for a key in a state.
fn subbed(key: &str, cached: Option<&Cached>) -> ServerMsg {
    let item = cached.map_or_else(
        || ValueItem {
            sub: key.to_string(),
            value: None,
            display: None,
            error: None,
        },
        |c| c.item(key),
    );
    ServerMsg::Subbed {
        sub: item.sub,
        value: item.value,
        display: item.display,
        error: item.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::AckItem;

    fn router(dir: &std::path::Path) -> Router {
        let (tx, _) = mpsc::unbounded_channel();
        Router::new(
            BTreeMap::new(),
            false,
            dir.to_path_buf(),
            RouterIo {
                tx,
                events: EventLog::off(),
            },
        )
    }

    /// A router over `band` on a port nothing listens on (the instance stays
    /// offline: a write is answered "instance offline" at once), its own
    /// messages and the event-log records.
    fn offline_router(
        dir: &std::path::Path,
    ) -> (
        Router,
        mpsc::UnboundedReceiver<RouterMsg>,
        std::sync::mpsc::Receiver<Value>,
    ) {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let (live, _task) = LiveHandle::spawn(
            &crate::config::InstanceCfg {
                name: "band".into(),
                port,
            },
            Arc::new(|_: &str, _: LiveEvent| {}),
        );
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, records) = EventLog::channel(1024);
        let router = Router::new(
            BTreeMap::from([("band".to_string(), live)]),
            false,
            dir.to_path_buf(),
            RouterIo { tx, events },
        );
        (router, rx, records)
    }

    fn attach(router: &mut Router, client: ClientId) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
        });
        outbox
    }

    fn set_msg(client: ClientId, instance: &str, seq: u64, value: f64) -> RouterMsg {
        RouterMsg::Set {
            client,
            instance: instance.into(),
            target: "live_set  tracks 0 mixer_device volume".into(),
            prop: "value".into(),
            value: json!(value),
            seq,
            t: 1_000.5,
            is_final: true,
            hub_ms: 1_250.5,
            offset_ms: Some(200.0),
        }
    }

    /// Every record waiting, by `ev`.
    fn records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
        rx.try_iter().collect()
    }

    fn evs(records: &[Value]) -> Vec<&str> {
        records.iter().map(|r| r["ev"].as_str().unwrap()).collect()
    }

    const VOLUME: &str = "band|live_set tracks 0 mixer_device volume|value";

    #[test]
    fn a_write_to_an_unknown_instance_is_acked_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (events, rx) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let mut router = Router::new(
            BTreeMap::new(),
            false,
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        );
        let outbox = attach(&mut router, 3);
        outbox.take();
        assert!(router.handle(set_msg(3, "drums", 9, 0.5)));
        let key = "drums|live_set tracks 0 mixer_device volume|value";
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(key, 9, "unknown instance \"drums\"")]
            }]
        );
        let written = records(&rx);
        assert_eq!(evs(&written), vec!["set", "ack"]);
        assert_eq!(written[0]["unknown"], json!(true));
        assert_eq!(written[0]["peer"], "10.0.0.3");
        assert_eq!(written[0]["seq"], 9);
        assert_eq!(written[0]["t"], 1_000.5);
        assert_eq!(written[0]["final"], json!(true));
        // Sent at page 1000.5, here at hub 1250.5, the page 200 ms behind:
        // 50 ms on the way.
        assert_eq!(written[0]["hub_ms"], 1_250.5);
        assert_eq!(written[0]["offset_ms"], 200.0);
        assert_eq!(written[0]["delay_ms"], 50.0);
        assert_eq!(written[0]["gap_ms"], Value::Null);
        assert_eq!(written[0]["dropped_old"], json!(false));
        assert_eq!(written[1]["error"], "unknown instance \"drums\"");
        assert_eq!(written[1]["client"], 3);
        assert_eq!(written[1]["batch"], Value::Null, "no batch answered it");
        assert_eq!(writes::unknown_instance("x"), "unknown instance \"x\"");
    }

    #[tokio::test]
    async fn a_write_to_an_offline_instance_goes_round_and_is_acked_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, records_rx) = offline_router(dir.path());
        let outbox = attach(&mut router, 1);
        outbox.take();
        router.handle(set_msg(1, "band", 1, 0.5));
        // Batch 1 is in flight: a newer set waits.
        router.handle(set_msg(1, "band", 2, 0.6));
        router.handle(set_msg(1, "band", 2, 0.7));
        let applied = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the batch's result comes back")
            .unwrap();
        let RouterMsg::Applied {
            ref instance,
            batch,
            ref outcome,
        } = applied
        else {
            panic!("an applied message")
        };
        assert_eq!((instance.as_str(), batch), ("band", 1));
        assert_eq!(outcome, &Err("instance offline".to_string()));
        // The result is taken 30 ms after the write: Live's round trip on
        // the router's own clock.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        router.handle(applied);
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(VOLUME, 1, "instance offline")]
            }]
        );
        // The newer want went out as batch 2.
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(next, RouterMsg::Applied { batch: 2, .. }));
        router.handle(next);
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::failed(VOLUME, 2, "instance offline")]
            }]
        );
        let written = records(&records_rx);
        assert_eq!(
            evs(&written),
            vec![
                "set", "batch", "set", "set", "applied", "ack", "batch", "applied", "ack"
            ]
        );
        assert_eq!(written[0]["key"], VOLUME);
        assert_eq!(written[0]["gap_ms"], Value::Null);
        assert_eq!(written[0]["dropped_old"], json!(false));
        assert_eq!(written[0]["unknown"], json!(false));
        assert_eq!(written[0]["delay_ms"], 50.0);
        assert_eq!(written[1]["batch"], 1);
        assert_eq!(written[1]["n"], 1);
        assert_eq!(
            written[1]["sent"],
            json!([{"key": VOLUME, "client": 1, "seq": 1, "value": 0.5}])
        );
        let gap = written[2]["gap_ms"].as_f64().unwrap();
        assert!((0.0..1_000.0).contains(&gap), "{gap}");
        assert_eq!(written[3]["dropped_old"], json!(true), "seq 2 twice");
        assert_eq!(written[4]["errors"], 1);
        assert_eq!(written[4]["n"], 1);
        assert_eq!(written[4]["batch"], 1);
        assert_eq!(
            written[4]["sent"],
            json!([{"key": VOLUME, "client": 1, "seq": 1}])
        );
        let rtt = written[4]["rtt_ms"].as_f64().unwrap();
        assert!((30.0..5_000.0).contains(&rtt), "{rtt}");
        assert_eq!(written[5]["seq"], 1);
        assert_eq!(written[5]["error"], "instance offline");
        assert_eq!(written[5]["peer"], "10.0.0.1");
        assert_eq!(written[5]["batch"], 1);
        assert_eq!(written[5]["rtt_ms"], rtt);
        assert_eq!(written[6]["sent"][0]["value"], 0.6);
        // A result for a batch no longer in flight changes nothing.
        router.handle(RouterMsg::Applied {
            instance: "band".into(),
            batch: 7,
            outcome: Ok(vec![]),
        });
        assert_eq!(outbox.take().unwrap(), vec![]);
        assert!(records(&records_rx).is_empty());
    }

    #[tokio::test]
    async fn a_disconnect_forgets_the_writes_and_a_detach_the_clients_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, _records) = offline_router(dir.path());
        attach(&mut router, 1);
        router.handle(set_msg(1, "band", 4, 0.5));
        router.handle(set_msg(1, "band", 5, 0.6));
        assert!(router.setters["band"].in_flight().is_some());
        assert_eq!(router.setters["band"].pending().len(), 1);
        router.handle(RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Disconnected,
        });
        assert!(router.setters["band"].in_flight().is_none());
        assert!(router.setters["band"].pending().is_empty());
        // The old batch's answer acks nothing.
        let late = rx.recv().await.unwrap();
        router.handle(late);
        assert_eq!(router.setters["band"].tracked(), 1);
        router.handle(RouterMsg::Detach { client: 1 });
        assert_eq!(router.setters["band"].tracked(), 0);
        assert!(!router.peers.contains_key(&1));
    }

    #[test]
    fn live_health_goes_to_every_client_and_a_change_to_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let (events, rx) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        let mut router = Router::new(
            BTreeMap::new(),
            false,
            dir.path().to_path_buf(),
            RouterIo { tx, events },
        );
        router
            .states
            .insert("band".into(), InstanceState::default());
        let a = attach(&mut router, 1);
        let b = attach(&mut router, 2);
        a.take();
        b.take();
        let busy = |changed: bool, tick_age_ms: f64| RouterMsg::Live {
            instance: "band".into(),
            event: LiveEvent::Busy {
                busy: true,
                changed,
                tick_age_ms,
                reason: changed.then(|| "no heartbeat for 301 ms".to_string()),
            },
        };
        router.handle(busy(true, 301.0));
        let link = |tick_age_ms: f64| ServerMsg::Link {
            instance: "band".into(),
            tick_age_ms,
            busy: true,
        };
        let state = ServerMsg::Instance {
            name: "band".into(),
            online: false,
            busy: true,
            set_name: String::new(),
        };
        for outbox in [&a, &b] {
            assert_eq!(outbox.take().unwrap(), vec![state.clone(), link(301.0)]);
        }
        assert!(router.states["band"].busy);
        let written = records(&rx);
        assert_eq!(evs(&written), vec!["link"]);
        assert_eq!(written[0]["busy"], json!(true));
        assert_eq!(written[0]["tick_age_ms"], 301.0);
        assert_eq!(written[0]["reason"], "no heartbeat for 301 ms");
        // While busy: a link only, no state change, no record.
        router.handle(busy(false, 560.0));
        assert_eq!(a.take().unwrap(), vec![link(560.0)]);
        assert!(records(&rx).is_empty());
    }

    fn status(router: &mut Router) -> RouterStatus {
        let (reply, mut answer) = oneshot::channel();
        assert!(router.handle(RouterMsg::Status { reply }));
        answer.try_recv().expect("answered at once")
    }

    #[test]
    fn a_client_gets_the_layout_revision_only_once_one_is_served() {
        let dir = tempfile::tempdir().unwrap();
        let mut router = router(dir.path());
        let first = attach(&mut router, 1);
        assert_eq!(
            first.take().unwrap(),
            vec![ServerMsg::Hub {
                key: HUB_STAGE_AUT.into(),
                value: json!(false)
            }],
            "no layout yet: no revision"
        );
        router.handle(RouterMsg::Layout {
            rev: 1,
            stage: None,
            targets: BTreeMap::new(),
            strips: Vec::new(),
        });
        assert_eq!(first.take().unwrap(), vec![ServerMsg::Layout { rev: 1 }]);
        let second = attach(&mut router, 2);
        assert!(
            second
                .take()
                .unwrap()
                .contains(&ServerMsg::Layout { rev: 1 })
        );
        let now = status(&mut router);
        assert_eq!(now.clients, 2);
        assert!(now.unresolved.is_empty());
        router.handle(RouterMsg::Detach { client: 1 });
        assert_eq!(status(&mut router).clients, 1);
        assert!(!router.handle(RouterMsg::Stop));
    }

    #[tokio::test]
    async fn the_strips_tracks_are_followed_for_their_groups() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        let layout = |rev, strips: Vec<(String, String)>| RouterMsg::Layout {
            rev,
            stage: None,
            targets: BTreeMap::new(),
            strips,
        };
        router.handle(layout(1, vec![("band".into(), "Keys 1".into())]));
        assert_eq!(
            router.subs.subscriptions("band"),
            2,
            "the keeper's watches of the band's tracks and visible tracks (#58)"
        );
        let now = status(&mut router);
        assert_eq!(now.subscriptions["band"], 0, "the hub's own read");
        assert_eq!(now.unfolded, BTreeMap::new(), "offline: no group known");
        // A new layout without it lets it go; an instance the hub does not
        // know is refused (logged) and kept nowhere.
        router.handle(layout(2, vec![("nowhere".into(), "X".into())]));
        assert_eq!(router.subs.subscriptions("band"), 0);
        assert_eq!(
            cached_value(&Cached::Value {
                value: json!(1),
                display: None
            }),
            Some(&json!(1))
        );
        assert_eq!(cached_value(&Cached::Error("gone".into())), None);
    }

    #[tokio::test]
    async fn a_failed_read_of_the_strips_groups_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, _records) = offline_router(dir.path());
        router.handle(RouterMsg::Layout {
            rev: 1,
            stage: None,
            targets: BTreeMap::new(),
            strips: vec![("band".into(), "Keys 1".into())],
        });
        // The read goes to an offline instance: its failure comes back…
        let failed = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the read's answer")
            .unwrap();
        let RouterMsg::UnfoldRead {
            ref instance,
            ref outcome,
            ..
        } = failed
        else {
            panic!("the read's answer")
        };
        assert_eq!(
            (instance.as_str(), outcome),
            ("band", &Err("instance offline".to_string()))
        );
        router.handle(failed);
        // …and the read is due again after RETRY_MS.
        let started = std::time::Instant::now();
        let retry = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a retry")
            .unwrap();
        assert!(
            matches!(retry, RouterMsg::UnfoldRetry { ref instance } if instance == "band"),
            "a retry of the band"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(unfold::RETRY_MS - 100));
        // The retry reads again: its answer comes back too.
        router.handle(retry);
        let again = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the second read's answer")
            .unwrap();
        assert!(matches!(again, RouterMsg::UnfoldRead { .. }));
    }

    #[test]
    fn a_stage_write_failure_says_why() {
        assert_eq!(write_failure(&Ok(vec![json!({"ok": true})])), None);
        assert_eq!(
            write_failure(&Ok(vec![json!({"ok": false})])),
            Some(r#"[{"ok":false}]"#.to_string())
        );
        assert_eq!(write_failure(&Ok(vec![])), Some("[]".to_string()));
        assert_eq!(
            write_failure(&Err(LiveError::Offline)),
            Some("instance offline".to_string())
        );
    }

    #[test]
    fn subbed_carries_the_cached_state() {
        assert_eq!(
            subbed("k", None),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: None,
                display: None,
                error: None
            }
        );
        assert_eq!(
            subbed(
                "k",
                Some(&Cached::Value {
                    value: json!(0.5),
                    display: Some("-6.0 dB".into())
                })
            ),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: Some(json!(0.5)),
                display: Some("-6.0 dB".into()),
                error: None
            }
        );
        assert_eq!(
            subbed("k", Some(&Cached::Error("gone".into()))),
            ServerMsg::Subbed {
                sub: "k".into(),
                value: None,
                display: None,
                error: Some("gone".into())
            }
        );
    }
}
