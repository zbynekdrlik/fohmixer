//! The hub connection and Live's state for the surface (S4 design note
//! §4): the `LiveStore` context owns the client WebSocket (reconnecting in
//! under 2 s, never giving up), the handshake, the subscriptions (each key
//! once, only the pages on screen), one signal per subscription, the
//! instances' states and the hub values.
//!
//! I8: every subscription starts `Pending` and goes back to `Pending` when
//! the hub connection drops, its instance goes offline, or REFRESH ALL
//! resubscribes; a control accepts input only while its slot holds Live's
//! value. I5: a binding that does not resolve is an `Error` slot, shown red
//! and disabled.
//!
//! The pure parts (`Slot`, `Wanted`, `Badge`, `slot_failure`, `range_from`)
//! are unit-tested here; `LiveStore` is the browser glue around them.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use fohmixer_proto::client::{CLOSE_RELOAD, ClientMsg, LiveCommand, ServerMsg};
use fohmixer_proto::layout::Layout;
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::binding::{SubSpec, unfold_targets};
use crate::dom;
use crate::net::{self, Decision, LayoutFetch};

/// What the surface knows of one subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// No value from Live yet: the control is disabled (I8).
    Pending,
    /// Live's value and display string, received at `at` (page clock, ms).
    Value {
        value: Value,
        display: Option<String>,
        at: f64,
    },
    /// The binding does not resolve (a missing or ambiguous name): red and
    /// disabled (I5).
    Error(String),
}

impl Slot {
    /// The slot a `subbed` or `values` item leaves: an error, a value, or
    /// `None` when it carries neither (the hub waits for Live).
    pub fn from_item(
        value: Option<Value>,
        display: Option<String>,
        error: Option<String>,
        at: f64,
    ) -> Option<Self> {
        match (error, value) {
            (Some(error), _) => Some(Self::Error(error)),
            (None, Some(value)) => Some(Self::Value { value, display, at }),
            (None, None) => None,
        }
    }

    /// Whether Live's value is here (the control may take input).
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Value { .. })
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Value { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Live's value as a number.
    pub fn number(&self) -> Option<f64> {
        self.value().and_then(Value::as_f64)
    }

    /// Live's value as a flag (`mute`, `solo`).
    pub fn flag(&self) -> Option<bool> {
        self.value().and_then(Value::as_bool)
    }

    /// Live's display string.
    pub fn display(&self) -> Option<&str> {
        match self {
            Self::Value { display, .. } => display.as_deref(),
            _ => None,
        }
    }

    /// When the value arrived.
    pub fn at(&self) -> Option<f64> {
        match self {
            Self::Value { at, .. } => Some(*at),
            _ => None,
        }
    }
}

/// What a new wanted set changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Keys to unsubscribe.
    pub removed: Vec<String>,
    /// Subscriptions to make.
    pub added: Vec<SubSpec>,
}

/// The subscriptions the surface wants: each key once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    specs: BTreeMap<String, SubSpec>,
}

impl Wanted {
    /// Replaces the wanted set with `specs`: what to unsubscribe and
    /// subscribe (both in key order; duplicates count once).
    pub fn replace(&mut self, specs: Vec<SubSpec>) -> Change {
        let new: BTreeMap<String, SubSpec> = specs.into_iter().map(|s| (s.key(), s)).collect();
        let removed = self
            .specs
            .keys()
            .filter(|k| !new.contains_key(*k))
            .cloned()
            .collect();
        let added = new
            .iter()
            .filter(|(k, _)| !self.specs.contains_key(*k))
            .map(|(_, s)| s.clone())
            .collect();
        self.specs = new;
        Change { removed, added }
    }

    /// Every wanted subscription, in key order.
    pub fn specs(&self) -> Vec<SubSpec> {
        self.specs.values().cloned().collect()
    }

    /// The wanted keys of `instance` (every key for `None`).
    pub fn keys_of(&self, instance: Option<&str>) -> Vec<String> {
        self.specs
            .iter()
            .filter(|(_, s)| instance.is_none_or(|i| s.instance == i))
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

/// One instance as the hub reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceView {
    pub online: bool,
    pub busy: bool,
    pub set_name: String,
}

/// A connection badge's state (spec §2.5 robustness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Online,
    Busy,
    Offline,
}

impl Badge {
    pub fn of(view: &InstanceView) -> Self {
        match (view.online, view.busy) {
            (false, _) => Self::Offline,
            (true, true) => Self::Busy,
            (true, false) => Self::Online,
        }
    }

    /// The badge's `data-state` and CSS class.
    pub fn name(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Offline => "offline",
        }
    }
}

/// Why a one-command `cmd` did not do its work, if it did not: the
/// script's error slot or the hub's refusal (spec I6: never retried).
pub fn slot_failure(outcome: &Result<Vec<Value>, String>) -> Option<String> {
    match outcome {
        Ok(slots) => match slots.first() {
            Some(slot) if slot.get("ok") == Some(&json!(true)) => None,
            Some(slot) => Some(
                slot.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("failed")
                    .to_string(),
            ),
            None => Some("no result".to_string()),
        },
        Err(e) => Some(e.clone()),
    }
}

/// A parameter's `min` and `max` from the result of a
/// `get_prop min` + `get_prop max` batch; `None` unless both are numbers
/// and the range is not empty.
pub fn range_from(outcome: &Result<Vec<Value>, String>) -> Option<(f64, f64)> {
    let slots = outcome.as_ref().ok()?;
    let number = |i: usize| -> Option<f64> {
        let slot = slots.get(i)?;
        (slot.get("ok") == Some(&json!(true)))
            .then(|| slot.get("data").and_then(Value::as_f64))
            .flatten()
    };
    let (min, max) = (number(0)?, number(1)?);
    (max > min).then_some((min, max))
}

/// The key under which the engineer's token is stored.
pub const TOKEN_KEY: &str = "fohmixer_token";

/// What hears a command's result slots (or why there are none).
pub type ResultFn = Box<dyn FnOnce(Result<Vec<Value>, String>)>;
/// A parameter's range (`min`, `max`) as Live reports it.
type Range = ArcRwSignal<Option<(f64, f64)>>;

/// The open socket and the handlers it calls (kept alive with it).
struct Socket {
    ws: web_sys::WebSocket,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Socket {
    /// Detaches the handlers and closes the socket.
    fn close(self) {
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

struct Inner {
    token: String,
    wanted: Wanted,
    slots: HashMap<String, ArcRwSignal<Slot>>,
    ranges: HashMap<(String, String), Range>,
    socket: Option<Socket>,
    ready: bool,
    hello_seen: bool,
    attempt: u32,
    next_id: u64,
    pending: HashMap<String, ResultFn>,
    rev: u64,
    stopped: bool,
    refreshed: bool,
}

impl Inner {
    /// Every field spelled out (no struct-update base: see
    /// `.claude/rules/hub-rust.md`).
    fn new(token: String) -> Self {
        Self {
            token,
            wanted: Wanted::default(),
            slots: HashMap::new(),
            ranges: HashMap::new(),
            socket: None,
            ready: false,
            hello_seen: false,
            attempt: 0,
            next_id: 0,
            pending: HashMap::new(),
            rev: 0,
            stopped: false,
            refreshed: false,
        }
    }
}

/// The surface's hub connection (a context; `Copy`).
#[derive(Clone, Copy)]
pub struct LiveStore {
    inner: StoredValue<Inner, LocalStorage>,
    /// The engineer's token; `None` sends the app to the login.
    session: RwSignal<Option<String>>,
    /// The served layout.
    pub layout: RwSignal<Option<Arc<Layout>>>,
    /// Why there is no layout (the hub's words), while there is none.
    pub layout_note: RwSignal<Option<String>>,
    /// The instances, by name.
    pub instances: RwSignal<BTreeMap<String, InstanceView>>,
    /// The hub values (STAGE AUT).
    pub hub: RwSignal<BTreeMap<String, Value>>,
    /// The hub socket is up and said hello.
    pub connected: RwSignal<bool>,
    /// How many subscriptions the pages on screen hold.
    pub subscribed: RwSignal<usize>,
    /// How many times REFRESH ALL (or the automatic refresh) ran.
    pub refreshes: RwSignal<u32>,
}

impl LiveStore {
    /// A store for `token`; `session` is the app's token signal.
    pub fn new(token: String, session: RwSignal<Option<String>>) -> Self {
        Self {
            inner: StoredValue::new_local(Inner::new(token)),
            session,
            layout: RwSignal::new(None),
            layout_note: RwSignal::new(None),
            instances: RwSignal::new(BTreeMap::new()),
            hub: RwSignal::new(BTreeMap::new()),
            connected: RwSignal::new(false),
            subscribed: RwSignal::new(0),
            refreshes: RwSignal::new(0),
        }
    }

    /// Connects: fetches the layout (checking the token), then opens the
    /// socket.
    pub fn start(self) {
        self.connect();
    }

    /// Ends the store (the surface unmounts): closes the socket, stops
    /// reconnecting.
    pub fn stop(self) {
        let socket = self
            .inner
            .try_update_value(|i| {
                i.stopped = true;
                i.pending.clear();
                i.socket.take()
            })
            .flatten();
        if let Some(socket) = socket {
            socket.close();
        }
    }

    fn stopped(self) -> bool {
        self.inner.try_with_value(|i| i.stopped).unwrap_or(true)
    }

    fn token(self) -> String {
        self.inner
            .try_with_value(|i| i.token.clone())
            .unwrap_or_default()
    }

    /// The token was refused: forget it and show the login (once).
    fn logout(self) {
        dom::log("the hub refused the token: back to the login");
        self.stop();
        dom::storage_remove(TOKEN_KEY);
        let _ = self.session.try_set(None);
    }

    fn connect(self) {
        leptos::task::spawn_local(async move {
            let token = self.token();
            let answer = net::fetch_text("GET", "/api/layout", Some(&token), None).await;
            if self.stopped() {
                return;
            }
            match answer.map(|(status, body)| net::layout_fetch(status, &body)) {
                Ok(LayoutFetch::Layout(served)) => {
                    self.set_layout(served.rev, served.layout);
                    self.open_socket();
                }
                Ok(LayoutFetch::Unauthorized) => self.logout(),
                Ok(LayoutFetch::NoLayout(why)) => {
                    dom::log(&format!("the hub serves no layout: {why}"));
                    let _ = self.layout_note.try_set(Some(why));
                    self.open_socket();
                }
                Ok(LayoutFetch::Failed(why)) | Err(why) => {
                    dom::log(&format!("the hub did not answer: {why}"));
                    self.retry();
                }
            }
        });
    }

    /// Tries to connect again after the reconnect delay.
    fn retry(self) {
        let Some(attempt) = self.inner.try_update_value(|i| {
            i.attempt += 1;
            i.attempt - 1
        }) else {
            return;
        };
        let wait = net::reconnect_delay(attempt);
        set_timeout(
            move || {
                if !self.stopped() {
                    self.connect();
                }
            },
            std::time::Duration::from_millis(wait as u64),
        );
    }

    fn set_layout(self, rev: u64, layout: Layout) {
        let _ = self.inner.try_update_value(|i| i.rev = rev);
        let _ = self.layout.try_set(Some(Arc::new(layout)));
        let _ = self.layout_note.try_set(None);
    }

    fn open_socket(self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let url = net::ws_url(
            &location.protocol().unwrap_or_default(),
            &location.host().unwrap_or_default(),
            &self.token(),
        );
        let ws = match web_sys::WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(e) => {
                dom::error(&format!("cannot open the hub socket: {e:?}"));
                self.retry();
                return;
            }
        };
        let on_message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            if let Some(text) = event.data().as_string() {
                self.on_text(&text);
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        let on_close = Closure::wrap(Box::new(move |event: web_sys::CloseEvent| {
            self.on_close(event.code());
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let socket = Socket {
            ws: ws.clone(),
            _on_message: on_message,
            _on_close: on_close,
        };
        let old = self
            .inner
            .try_update_value(|i| {
                i.hello_seen = false;
                i.socket.replace(socket)
            })
            .flatten();
        if let Some(old) = old {
            old.close();
        }
        set_timeout(
            move || self.check_hello(&ws),
            std::time::Duration::from_millis(net::HELLO_TIMEOUT_MS as u64),
        );
    }

    /// The hello timeout: a socket that is open without a hello talks to
    /// a hub that does not speak this protocol.
    fn check_hello(self, ws: &web_sys::WebSocket) {
        let silent = self
            .inner
            .try_with_value(|i| {
                !i.stopped && !i.hello_seen && i.socket.as_ref().is_some_and(|s| s.ws == *ws)
            })
            .unwrap_or(false);
        if silent && ws.ready_state() == web_sys::WebSocket::OPEN {
            let now = dom::now();
            if net::on_missing_hello(now, net::last_reload()) == Decision::Reload {
                net::reload(now, "no hello from the hub");
            }
        }
    }

    fn on_close(self, code: u16) {
        let Some((stopped, pending)) = self.inner.try_update_value(|i| {
            i.socket = None;
            i.ready = false;
            (i.stopped, std::mem::take(&mut i.pending))
        }) else {
            return;
        };
        if stopped {
            return;
        }
        dom::log(&format!(
            "the hub socket closed (code {code}): reconnecting"
        ));
        let _ = self.connected.try_set(false);
        let _ = self.instances.try_update(|all| {
            for view in all.values_mut() {
                view.online = false;
                view.busy = false;
            }
        });
        self.mark_pending(None);
        for done in pending.into_values() {
            done(Err("the hub connection closed".to_string()));
        }
        if code == CLOSE_RELOAD {
            let now = dom::now();
            if net::on_missing_hello(now, net::last_reload()) == Decision::Reload {
                net::reload(now, "the hub asked for a reload");
                return;
            }
        }
        self.retry();
    }

    fn on_text(self, text: &str) {
        let msg = match serde_json::from_str::<ServerMsg>(text) {
            Ok(msg) => msg,
            Err(e) => {
                dom::error(&format!("unreadable hub message: {e}"));
                return;
            }
        };
        match msg {
            ServerMsg::Hello {
                proto,
                build,
                min_client_proto,
            } => self.on_hello(proto, &build, min_client_proto),
            ServerMsg::Subbed {
                sub,
                value,
                display,
                error,
            } => self.apply(&sub, value, display, error),
            ServerMsg::Values { items } => {
                for item in items {
                    self.apply(&item.sub, item.value, item.display, item.error);
                }
            }
            ServerMsg::Instance {
                name,
                online,
                busy,
                set_name,
            } => self.on_instance(name, online, busy, set_name),
            ServerMsg::Hub { key, value } => {
                let _ = self.hub.try_update(|hub| {
                    hub.insert(key, value);
                });
            }
            ServerMsg::Layout { rev } => self.on_layout(rev),
            ServerMsg::Result { id, data } => self.finish(&id, Ok(data)),
            ServerMsg::Error {
                id: Some(id),
                message,
            } => self.finish(&id, Err(message)),
            ServerMsg::Error { id: None, message } => {
                dom::log(&format!("the hub refused a request: {message}"));
            }
        }
    }

    fn on_hello(self, proto: u32, build: &str, min_client_proto: u32) {
        let now = dom::now();
        if net::on_hello(proto, min_client_proto, now, net::last_reload()) == Decision::Reload {
            net::reload(
                now,
                &format!("hub {build} serves UI protocol {min_client_proto}..={proto}"),
            );
            return;
        }
        let Some((specs, first)) = self.inner.try_update_value(|i| {
            i.ready = true;
            i.hello_seen = true;
            i.attempt = 0;
            let first = !i.refreshed;
            i.refreshed = true;
            (i.wanted.specs(), first)
        }) else {
            return;
        };
        dom::log(&format!("connected to hub {build}"));
        let _ = self.connected.try_set(true);
        for spec in &specs {
            self.send_sub(spec);
        }
        self.fetch_ranges();
        if first {
            // Spec F6: the automatic refresh a second after the load.
            set_timeout(
                move || self.refresh(),
                std::time::Duration::from_millis(crate::behave::timing::AUTO_REFRESH_MS as u64),
            );
        }
    }

    fn on_instance(self, name: String, online: bool, busy: bool, set_name: String) {
        if !online {
            self.mark_pending(Some(name.as_str()));
        }
        let _ = self.instances.try_update(|all| {
            all.insert(
                name,
                InstanceView {
                    online,
                    busy,
                    set_name,
                },
            );
        });
    }

    fn on_layout(self, rev: u64) {
        if self.inner.try_with_value(|i| i.rev) == Some(rev) {
            return;
        }
        leptos::task::spawn_local(async move {
            let token = self.token();
            let answer = net::fetch_text("GET", "/api/layout", Some(&token), None).await;
            match answer.map(|(status, body)| net::layout_fetch(status, &body)) {
                Ok(LayoutFetch::Layout(served)) => {
                    dom::log(&format!("layout revision {}", served.rev));
                    self.set_layout(served.rev, served.layout);
                }
                Ok(LayoutFetch::Unauthorized) => self.logout(),
                Ok(LayoutFetch::NoLayout(why) | LayoutFetch::Failed(why)) | Err(why) => {
                    dom::log(&format!("cannot load layout revision {rev}: {why}"));
                }
            }
        });
    }

    /// A subscription's new state from the hub.
    fn apply(
        self,
        key: &str,
        value: Option<Value>,
        display: Option<String>,
        error: Option<String>,
    ) {
        let Some(slot) = Slot::from_item(value, display, error, dom::now()) else {
            return;
        };
        let signal = self
            .inner
            .try_with_value(|i| i.slots.get(key).cloned())
            .flatten();
        if let Some(signal) = signal {
            let _ = signal.try_set(slot);
        }
    }

    /// Every wanted slot (of one instance) back to `Pending` (I8).
    fn mark_pending(self, instance: Option<&str>) {
        let signals: Vec<ArcRwSignal<Slot>> = self
            .inner
            .try_with_value(|i| {
                i.wanted
                    .keys_of(instance)
                    .iter()
                    .filter_map(|k| i.slots.get(k).cloned())
                    .collect()
            })
            .unwrap_or_default();
        for signal in signals {
            let _ = signal.try_set(Slot::Pending);
        }
    }

    fn send(self, msg: &ClientMsg) -> bool {
        let Ok(text) = serde_json::to_string(msg) else {
            return false;
        };
        self.inner
            .try_with_value(|i| match &i.socket {
                Some(socket) if i.ready => socket.ws.send_with_str(&text).is_ok(),
                _ => false,
            })
            .unwrap_or(false)
    }

    fn send_sub(self, spec: &SubSpec) {
        self.send(&ClientMsg::Sub {
            instance: spec.instance.clone(),
            target: spec.target.clone(),
            prop: spec.prop.clone(),
            display: spec.display,
        });
    }

    /// The slot of a subscription (created `Pending`); the signal lives as
    /// long as the caller's owner, the slot as long as the store.
    pub fn slot(self, spec: &SubSpec) -> RwSignal<Slot> {
        let key = spec.key();
        self.inner
            .try_update_value(|i| {
                i.slots
                    .entry(key)
                    .or_insert_with(|| ArcRwSignal::new(Slot::Pending))
                    .clone()
            })
            .map_or_else(|| RwSignal::new(Slot::Pending), RwSignal::from)
    }

    /// Subscribes the pages on screen: the difference to the last set.
    pub fn set_wanted(self, specs: Vec<SubSpec>) {
        let Some((change, ready, count)) = self.inner.try_update_value(|i| {
            let change = i.wanted.replace(specs);
            (change, i.ready, i.wanted.len())
        }) else {
            return;
        };
        let _ = self.subscribed.try_set(count);
        for key in &change.removed {
            if ready {
                self.send(&ClientMsg::Unsub { sub: key.clone() });
            }
        }
        for spec in &change.added {
            let _ = self.slot_signal(&spec.key()).try_set(Slot::Pending);
            if ready {
                self.send_sub(spec);
            }
        }
        for key in &change.removed {
            let _ = self.slot_signal(key).try_set(Slot::Pending);
        }
    }

    fn slot_signal(self, key: &str) -> ArcRwSignal<Slot> {
        self.inner
            .try_update_value(|i| {
                i.slots
                    .entry(key.to_string())
                    .or_insert_with(|| ArcRwSignal::new(Slot::Pending))
                    .clone()
            })
            .unwrap_or_else(|| ArcRwSignal::new(Slot::Pending))
    }

    /// REFRESH ALL (spec F6, F7): unsubscribe everything, unfold the
    /// configured groups, subscribe everything again (the hub resolves
    /// every name afresh). Every control waits for its new value (I8).
    pub fn refresh(self) {
        let Some((specs, ready)) = self.inner.try_with_value(|i| (i.wanted.specs(), i.ready))
        else {
            return;
        };
        if !ready {
            return;
        }
        dom::log(&format!("refresh: {} subscriptions", specs.len()));
        for spec in &specs {
            self.send(&ClientMsg::Unsub { sub: spec.key() });
        }
        self.mark_pending(None);
        if let Some(layout) = self.layout.try_get_untracked().flatten() {
            for (instance, target) in unfold_targets(&layout.config) {
                self.set_prop(&instance, &target, "fold_state", json!(false), None);
            }
        }
        for spec in &specs {
            self.send_sub(spec);
        }
        self.fetch_ranges();
        let _ = self.refreshes.try_update(|n| *n += 1);
    }

    /// Sends a command batch; `done` gets the result slots (or why none).
    pub fn cmd(self, instance: &str, commands: Vec<LiveCommand>, done: Option<ResultFn>) {
        let Some(id) = self.inner.try_update_value(|i| {
            i.next_id += 1;
            format!("c{}", i.next_id)
        }) else {
            return;
        };
        let sent = self.send(&ClientMsg::Cmd {
            id: id.clone(),
            instance: instance.to_string(),
            commands,
        });
        match (sent, done) {
            (true, Some(done)) => {
                let _ = self.inner.try_update_value(|i| i.pending.insert(id, done));
            }
            (false, Some(done)) => done(Err("not connected to the hub".to_string())),
            (_, None) => {}
        }
    }

    fn finish(self, id: &str, outcome: Result<Vec<Value>, String>) {
        let done = self
            .inner
            .try_update_value(|i| i.pending.remove(id))
            .flatten();
        if let Some(done) = done {
            done(outcome);
        }
    }

    /// `set_prop` of one property; `failed` hears why it failed (spec I6:
    /// shown, never retried).
    pub fn set_prop(
        self,
        instance: &str,
        target: &str,
        prop: &str,
        value: Value,
        failed: Option<Box<dyn FnOnce(String)>>,
    ) {
        let what = format!("{instance} {target} {prop}");
        let done: ResultFn = Box::new(move |outcome: Result<Vec<Value>, String>| {
            if let Some(why) = slot_failure(&outcome) {
                dom::log(&format!("set {what} failed: {why}"));
                if let Some(failed) = failed {
                    failed(why);
                }
            }
        });
        self.cmd(
            instance,
            vec![LiveCommand {
                target: Value::String(target.to_string()),
                name: "set_prop".to_string(),
                args: json!({"prop": prop, "value": value}),
            }],
            Some(done),
        );
    }

    /// Sets a hub value (STAGE AUT).
    pub fn set_hub(self, key: &str, value: Value) {
        self.send(&ClientMsg::SetHub {
            key: key.to_string(),
            value,
        });
    }

    /// A parameter's range (`min`, `max`), read from Live when first asked
    /// and again on every (re)connection and refresh.
    pub fn range(self, instance: &str, target: &str) -> RwSignal<Option<(f64, f64)>> {
        let key = (instance.to_string(), target.to_string());
        let Some((signal, fresh)) = self.inner.try_update_value(|i| {
            let fresh = !i.ranges.contains_key(&key);
            let signal = i
                .ranges
                .entry(key)
                .or_insert_with(|| ArcRwSignal::new(None))
                .clone();
            (signal, fresh)
        }) else {
            return RwSignal::new(None);
        };
        if fresh {
            self.fetch_range(instance, target, signal.clone());
        }
        RwSignal::from(signal)
    }

    fn fetch_ranges(self) {
        let all: Vec<((String, String), Range)> = self
            .inner
            .try_with_value(|i| {
                i.ranges
                    .iter()
                    .map(|(k, s)| (k.clone(), s.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for ((instance, target), signal) in all {
            self.fetch_range(&instance, &target, signal);
        }
    }

    fn fetch_range(self, instance: &str, target: &str, signal: Range) {
        let get = |prop: &str| LiveCommand {
            target: Value::String(target.to_string()),
            name: "get_prop".to_string(),
            args: json!({ "prop": prop }),
        };
        self.cmd(
            instance,
            vec![get("min"), get("max")],
            Some(Box::new(move |outcome: Result<Vec<Value>, String>| {
                let _ = signal.try_set(range_from(&outcome));
            })),
        );
    }
}

#[cfg(test)]
mod tests;
