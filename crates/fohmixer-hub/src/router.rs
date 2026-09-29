//! The hub's router: one task that owns the subscription table, the client
//! outboxes and the STAGE AUT rule (S3 design note §3, §4, §6).
//!
//! Everything reaches it as a message, in order: the instances' events (a
//! subscription's result stays ordered with the value pushes of the same
//! connection), the clients' requests, layout changes. It never waits: it
//! writes into outboxes and queues requests to the instances' tasks. It also
//! runs the layout's check for unresolved names (spec §2.5 D4) when a layout
//! is accepted and when an instance connects.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use fohmixer_proto::client::{HUB_STAGE_AUT, ServerMsg, StageAutStatus, Unresolved, ValueItem};
use fohmixer_proto::layout::Binding;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::live::client::{LiveError, LiveEvent, LiveHandle};
use crate::live::names::NameCheck;
use crate::live::subs::{Cached, ClientId, Outgoing, Subs};
use crate::outbox::Outbox;
use crate::rules::{HubState, StageAut};

/// The router's own subscriber: the STAGE AUT rule (clients start at 1).
pub const STAGE_CLIENT: ClientId = 0;

/// A message to the router.
pub enum RouterMsg {
    /// An event of an instance's task.
    Live {
        instance: String,
        event: LiveEvent,
    },
    /// A client connected: its outbox.
    Attach {
        client: ClientId,
        outbox: Arc<Outbox>,
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
    /// A client's connection ended.
    Detach {
        client: ClientId,
    },
    /// A new layout is served: its revision, its STAGE AUT binding and its
    /// check targets per instance (for the unresolved-names check: the
    /// bindings and the groups to unfold, `live::names::layout_targets`).
    Layout {
        rev: u64,
        stage: Option<Binding>,
        targets: BTreeMap<String, Vec<String>>,
    },
    /// The router's part of `/api/status`.
    Status {
        reply: oneshot::Sender<RouterStatus>,
    },
    /// The hub stops: close every client.
    Stop,
}

/// The router's part of `/api/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterStatus {
    pub subscriptions: BTreeMap<String, usize>,
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

/// The router task's state.
pub struct Router {
    subs: Subs,
    names: NameCheck,
    clients: HashMap<ClientId, Arc<Outbox>>,
    live: BTreeMap<String, LiveHandle>,
    states: BTreeMap<String, InstanceState>,
    stage: StageAut,
    stage_target: Option<StageTarget>,
    layout_rev: u64,
    data_dir: PathBuf,
}

impl Router {
    /// A router over these instances, STAGE AUT persisted as `stage_aut`.
    pub fn new(live: BTreeMap<String, LiveHandle>, stage_aut: bool, data_dir: PathBuf) -> Self {
        Self {
            subs: Subs::new(live.keys().cloned()),
            names: NameCheck::default(),
            clients: HashMap::new(),
            states: live
                .keys()
                .map(|k| (k.clone(), InstanceState::default()))
                .collect(),
            live,
            stage: StageAut::new(stage_aut),
            stage_target: None,
            layout_rev: 0,
            data_dir,
        }
    }

    /// Serves messages until [`RouterMsg::Stop`] (or every sender is gone),
    /// then closes every client.
    pub async fn run(mut self, mut rx: mpsc::UnboundedReceiver<RouterMsg>) {
        while let Some(msg) = rx.recv().await {
            let go_on = self.handle(msg);
            self.flush();
            if !go_on {
                break;
            }
        }
        for outbox in self.clients.values() {
            outbox.close();
        }
        tracing::info!("router stopped");
    }

    fn handle(&mut self, msg: RouterMsg) -> bool {
        match msg {
            RouterMsg::Live { instance, event } => self.live_event(&instance, event),
            RouterMsg::Attach { client, outbox } => {
                for name in self.states.keys() {
                    self.send_instance(&outbox, name);
                }
                outbox.hub(HUB_STAGE_AUT, json!(self.stage.is_on()));
                if self.layout_rev > 0 {
                    outbox.layout(self.layout_rev);
                }
                self.clients.insert(client, outbox);
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
            RouterMsg::Detach { client } => {
                self.subs.drop_client(client);
                self.clients.remove(&client);
            }
            RouterMsg::Layout {
                rev,
                stage,
                targets,
            } => {
                self.layout_rev = rev;
                for outbox in self.clients.values() {
                    outbox.layout(rev);
                }
                self.set_stage_binding(stage.as_ref());
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
                        .map(|n| (n.clone(), self.subs.subscriptions(n)))
                        .collect(),
                    listeners: names.map(|n| (n.clone(), self.subs.listeners(n))).collect(),
                    stage_aut: StageAutStatus {
                        on: self.stage.is_on(),
                        writes: self.stage.writes(),
                    },
                    clients: self.clients.len(),
                    unresolved: self.names.unresolved(),
                });
            }
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
            LiveEvent::Busy { busy } => {
                if let Some(state) = self.states.get_mut(instance) {
                    state.busy = busy;
                }
                self.broadcast_instance(instance);
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

    /// Sends the table's and the name check's requests and hands out the
    /// table's values.
    fn flush(&mut self) {
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
        for (client, item) in self.subs.take_deliveries() {
            if client == STAGE_CLIENT {
                if let Some(value) = &item.value {
                    self.stage_value(value);
                }
            } else if let Some(outbox) = self.clients.get(&client) {
                outbox.value(item);
            }
        }
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

    fn router(dir: &std::path::Path) -> Router {
        Router::new(BTreeMap::new(), false, dir.to_path_buf())
    }

    fn attach(router: &mut Router, client: ClientId) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
        });
        outbox
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
