//! The browser side of the store: the socket, the timers and the signals
//! that carry out the decisions of `Conn` (`conn.rs`), `net`, the intent
//! store (`intent.rs`), the dropout watch (`behave::link`) and the pure
//! store types. Everything here is web glue (web_sys, JS closures, Leptos
//! signals and timers), driven by the E2E suite in Chromium and WebKit.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use fohmixer_proto::client::{CLOSE_RELOAD, ClientMsg, LiveCommand, ServerMsg};
use fohmixer_proto::layout::Layout;
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::conn::{self, Conn, Tick};
use super::intent::Intents;
use super::{InstanceView, ResultFn, Slot, TOKEN_KEY, next_range, slot_failure};
use crate::behave::link::{self, DropoutWatch};
use crate::binding::{SubSpec, unfold_targets};
use crate::dom;
use crate::net::{self, Decision, LayoutFetch};

mod writes;

/// A parameter's range (`min`, `max`) as Live reports it.
type Range = ArcRwSignal<Option<(f64, f64)>>;

/// What hears why a control's write failed.
type FailFn = Box<dyn FnOnce(String)>;

/// The open socket and the handlers it calls (kept alive with it).
struct Socket {
    ws: web_sys::WebSocket,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Socket {
    /// Whether the socket takes messages (not connecting, not closing).
    fn open(&self) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN
    }

    /// Detaches the handlers and closes the socket.
    fn close(self) {
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

struct Inner {
    token: String,
    conn: Conn,
    slots: HashMap<String, ArcRwSignal<Slot>>,
    ranges: HashMap<(String, String), Range>,
    socket: Option<Socket>,
    pending: HashMap<String, ResultFn>,
    /// The controls' writes waiting for their ack, with what hears each
    /// one's failure (#43).
    intents: Intents<FailFn>,
    /// The link's dropouts (#43).
    watch: DropoutWatch,
}

impl Inner {
    /// Every field spelled out (no struct-update base: see
    /// `.claude/rules/hub-rust.md`).
    fn new(token: String) -> Self {
        Self {
            token,
            conn: Conn::default(),
            slots: HashMap::new(),
            ranges: HashMap::new(),
            socket: None,
            pending: HashMap::new(),
            intents: Intents::default(),
            watch: DropoutWatch::default(),
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
    /// The hub values (STAGE AUT), as the hub last sent them on this
    /// socket (none while disconnected).
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
    /// socket; and starts the dropout watch's own tick (`tick_link`), which
    /// runs until the store stops.
    pub fn start(self) {
        self.connect();
        self.tick_link();
    }

    /// The dropout watch's own tick (#43), every `PING_MS` until the store
    /// stops, whatever the socket does: it keeps ticking while the page
    /// reconnects, so a lost socket counts from the page's next on-time tick.
    fn tick_link(self) {
        set_timeout(
            move || {
                if self.stopped() {
                    return;
                }
                let _ = self
                    .inner
                    .try_update_value(|i| i.watch.tick(dom::epoch_now()));
                self.tick_link();
            },
            Duration::from_millis(conn::PING_MS),
        );
    }

    /// Ends the store (the surface unmounts): closes the socket, stops
    /// reconnecting.
    pub fn stop(self) {
        let socket = self
            .inner
            .try_update_value(|i| {
                i.conn.stop();
                i.pending.clear();
                i.socket.take()
            })
            .flatten();
        if let Some(socket) = socket {
            socket.close();
        }
    }

    fn stopped(self) -> bool {
        self.inner
            .try_with_value(|i| i.conn.stopped())
            .unwrap_or(true)
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
                    self.show_layout(served.rev, served.layout);
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
        let Some(wait) = self.inner.try_update_value(|i| i.conn.retry_delay()) else {
            return;
        };
        set_timeout(
            move || {
                if !self.stopped() {
                    self.connect();
                }
            },
            Duration::from_millis(wait as u64),
        );
    }

    /// Shows a served layout. Only a different layout replaces the one on
    /// screen (`conn::replaces`): the controls under the engineer's fingers
    /// survive a reconnect (I4).
    fn show_layout(self, rev: u64, layout: Layout) {
        let _ = self.inner.try_update_value(|i| i.conn.showing(rev));
        if self.layout_note.try_with_untracked(Option::is_some) == Some(true) {
            let _ = self.layout_note.try_set(None);
        }
        let current = self.layout.try_get_untracked().flatten();
        if conn::replaces(current.as_deref(), &layout) {
            let _ = self.layout.try_set(Some(Arc::new(layout)));
        }
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
            self.on_close(Some(event.code()));
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let socket = Socket {
            ws,
            _on_message: on_message,
            _on_close: on_close,
        };
        let now = dom::now();
        let Some((number, old)) = self
            .inner
            .try_update_value(|i| (i.conn.opened(now), i.socket.replace(socket)))
        else {
            return;
        };
        if let Some(old) = old {
            old.close();
        }
        self.watch(number);
    }

    /// The watchdog of socket `number` (`Conn::tick`): every `PING_MS`
    /// until that socket is gone. It tells the dropout watch of each ping.
    fn watch(self, number: u64) {
        set_timeout(
            move || {
                let Some((tick, ping)) = self.inner.try_update_value(|i| {
                    let open = i.socket.as_ref().is_some_and(Socket::open);
                    let (now, epoch) = (dom::now(), dom::epoch_now());
                    i.conn.set_hidden(dom::hidden());
                    let tick = i.conn.tick(number, now, open);
                    let ping = (tick == Tick::Ping).then(|| i.conn.ping(now, epoch));
                    if let Some(ClientMsg::Ping { n, .. }) = &ping {
                        i.watch.ping(*n, epoch);
                    }
                    (tick, ping)
                }) else {
                    return;
                };
                match tick {
                    Tick::Done => return,
                    Tick::Wait => {}
                    Tick::Ping => {
                        if let Some(ping) = &ping {
                            self.send(ping);
                        }
                    }
                    Tick::NoHello => {
                        let now = dom::wall_now();
                        if net::on_missing_hello(now, net::last_reload()) == Decision::Reload {
                            net::reload(now, "no hello from the hub");
                        } else {
                            self.drop_socket("no hello from the hub");
                        }
                        return;
                    }
                    Tick::Silent => {
                        self.drop_socket("the hub has been silent for 3 s");
                        return;
                    }
                }
                self.watch(number);
            },
            Duration::from_millis(conn::PING_MS),
        );
    }

    /// Drops a socket that went silent (half-open): as if it had closed.
    fn drop_socket(self, why: &str) {
        dom::log(&format!("dropping the hub socket: {why}"));
        let socket = self.inner.try_update_value(|i| i.socket.take()).flatten();
        if let Some(socket) = socket {
            socket.close();
        }
        self.on_close(None);
    }

    /// The socket closed (`code`) or was dropped (`None`).
    fn on_close(self, code: Option<u16>) {
        let Some((closed, pending)) = self.inner.try_update_value(|i| {
            i.socket = None;
            // A socket that said hello: a dropout until the next hello.
            i.watch.lost(dom::epoch_now());
            (i.conn.closed(), std::mem::take(&mut i.pending))
        }) else {
            return;
        };
        if !closed.reconnect {
            return;
        }
        dom::log(&format!(
            "the hub socket closed (code {code:?}): reconnecting"
        ));
        if closed.lost {
            crate::diag::disconnected();
        }
        let _ = self.connected.try_set(false);
        let _ = self.hub.try_set(BTreeMap::new());
        let _ = self.instances.try_update(|all| {
            for view in all.values_mut() {
                view.online = false;
                view.busy = false;
            }
        });
        // The controls keep Live's last values, stale, and take touches (L2).
        self.mark_stale();
        for done in pending.into_values() {
            done(Err("the hub connection closed".to_string()));
        }
        if code == Some(CLOSE_RELOAD) {
            let now = dom::wall_now();
            if net::on_missing_hello(now, net::last_reload()) == Decision::Reload {
                net::reload(now, "the hub asked for a reload");
                return;
            }
        }
        self.retry();
    }

    fn on_text(self, text: &str) {
        let (now, epoch) = (dom::now(), dom::epoch_now());
        let _ = self.inner.try_update_value(|i| {
            i.conn.heard(now);
            i.watch.heard(epoch);
        });
        self.send_reports();
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
            } => {
                if !self.apply(&sub, value, display, error) {
                    dom::log(&format!("a value for {sub}, no page wants it: dropped"));
                }
            }
            ServerMsg::Values { items } => {
                // One line per message: leaving a page with meters drops
                // every meter value still on its way.
                let mut dropped = 0usize;
                let mut first = None;
                for item in items {
                    if !self.apply(&item.sub, item.value, item.display, item.error) {
                        dropped += 1;
                        first.get_or_insert(item.sub);
                    }
                }
                if let Some(first) = first {
                    dom::log(&format!(
                        "{dropped} values for keys no page wants dropped, e.g. {first}"
                    ));
                }
            }
            ServerMsg::Instance {
                name,
                online,
                busy,
                set_name,
            } => self.on_instance(
                name,
                InstanceView {
                    online,
                    busy,
                    set_name,
                },
            ),
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
            ServerMsg::Pong { n, t, .. } => {
                let _ = self.inner.try_update_value(|i| {
                    let rtt = i.conn.pong(n, t, epoch);
                    i.watch.pong(n, rtt);
                });
            }
            ServerMsg::Ack { items } => self.on_ack(&items),
            // Live's health: the link counter's input (PR C).
            ServerMsg::Link { .. } => {}
        }
    }

    /// Sends the finished dropouts to the hub's event log (#43, a
    /// `trace`); kept for the next try when the socket cannot take them.
    fn send_reports(self) {
        let reports = self
            .inner
            .try_update_value(|i| i.watch.take_reports())
            .unwrap_or_default();
        let Some(trace) = link::trace(&reports) else {
            return;
        };
        for report in &reports {
            dom::log(&format!(
                "the hub link dropped out for {:.0} ms{}",
                report.ms,
                if report.socket_lost {
                    " (the socket was lost)"
                } else {
                    ""
                }
            ));
        }
        if !self.send(&trace) {
            let _ = self.inner.try_update_value(|i| i.watch.requeue(reports));
        }
    }

    fn on_hello(self, proto: u32, build: &str, min_client_proto: u32) {
        let now = dom::wall_now();
        if net::on_hello(proto, min_client_proto, build, now, net::last_reload())
            == Decision::Reload
        {
            let page = fohmixer_proto::VERSION;
            net::reload(
                now,
                &format!(
                    "hub {build} (this page {page}) serves UI protocol {min_client_proto}..={proto}"
                ),
            );
            return;
        }
        let Some(hello) = self.inner.try_update_value(|i| {
            i.watch.hello(dom::epoch_now());
            i.conn.hello(dom::now())
        }) else {
            return;
        };
        dom::log(&format!("connected to hub {build}"));
        let _ = self.connected.try_set(true);
        crate::diag::connected();
        self.forget_unwanted();
        self.send_reports();
        for spec in &hello.specs {
            self.send_sub(spec);
        }
        if hello.auto_refresh {
            // Spec F6: the automatic refresh a second after the load.
            set_timeout(
                move || self.refresh(),
                Duration::from_millis(crate::behave::timing::AUTO_REFRESH_MS as u64),
            );
        }
    }

    /// An instance's new state: its slots wait while it is offline; its
    /// parameter ranges are read again when it is back, idle again or on
    /// another set.
    fn on_instance(self, name: String, view: InstanceView) {
        let old = self
            .instances
            .try_with_untracked(|all| all.get(&name).cloned())
            .flatten();
        let change = conn::instance_change(old.as_ref(), &view);
        let back = conn::back_online(old.as_ref(), &view);
        if change.pending {
            self.mark_pending(Some(name.as_str()));
        }
        let _ = self.instances.try_update(|all| {
            all.insert(name.clone(), view);
        });
        if change.ranges {
            self.fetch_ranges(Some(name.as_str()));
        }
        if back {
            self.resend(&name);
        }
    }

    fn on_layout(self, rev: u64) {
        if self.inner.try_with_value(|i| i.conn.shows(rev)) != Some(false) {
            return;
        }
        leptos::task::spawn_local(async move {
            let token = self.token();
            let answer = net::fetch_text("GET", "/api/layout", Some(&token), None).await;
            match answer.map(|(status, body)| net::layout_fetch(status, &body)) {
                Ok(LayoutFetch::Layout(served)) => {
                    dom::log(&format!("layout revision {}", served.rev));
                    self.show_layout(served.rev, served.layout);
                }
                Ok(LayoutFetch::Unauthorized) => self.logout(),
                Ok(LayoutFetch::NoLayout(why) | LayoutFetch::Failed(why)) | Err(why) => {
                    dom::log(&format!("cannot load layout revision {rev}: {why}"));
                }
            }
        });
    }

    /// A subscription's new state from the hub; false when no page wants its
    /// key: the hub sent it before it read the page's unsub, and nothing
    /// keeps that slot current any more, so it is not kept (#43, I8).
    fn apply(
        self,
        key: &str,
        value: Option<Value>,
        display: Option<String>,
        error: Option<String>,
    ) -> bool {
        if !self
            .inner
            .try_with_value(|i| i.conn.wants(key))
            .unwrap_or(false)
        {
            return false;
        }
        let Some(slot) = Slot::from_item(value, display, error, dom::now()) else {
            return true;
        };
        if let Some(value) = slot.value() {
            self.live_value(key, value);
        }
        let signal = self
            .inner
            .try_with_value(|i| i.slots.get(key).cloned())
            .flatten();
        if let Some(signal) = signal {
            let _ = signal.try_set(slot);
        }
        true
    }

    /// Every wanted slot (of one instance) back to `Pending` (I8).
    fn mark_pending(self, instance: Option<&str>) {
        let signals: Vec<ArcRwSignal<Slot>> = self
            .inner
            .try_with_value(|i| {
                i.conn
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

    /// Every wanted slot keeps its value, marked stale (#43, §4.2): the hub
    /// connection was lost, not Live.
    fn mark_stale(self) {
        let signals: Vec<ArcRwSignal<Slot>> = self
            .inner
            .try_with_value(|i| {
                i.conn
                    .keys_of(None)
                    .iter()
                    .filter_map(|k| i.slots.get(k).cloned())
                    .collect()
            })
            .unwrap_or_default();
        for signal in signals {
            let _ = signal.try_update(|slot| {
                *slot = std::mem::replace(slot, Slot::Pending).into_stale();
            });
        }
    }

    fn send(self, msg: &ClientMsg) -> bool {
        let Ok(text) = serde_json::to_string(msg) else {
            return false;
        };
        self.inner
            .try_with_value(|i| match &i.socket {
                // Only an open socket: one the hub is closing would log
                // "already in CLOSING or CLOSED state".
                Some(socket) if i.conn.ready() && socket.open() => {
                    socket.ws.send_with_str(&text).is_ok()
                }
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
        RwSignal::from(self.slot_signal(&spec.key()))
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

    /// Subscribes the pages on screen: the difference to the last set.
    pub fn set_wanted(self, specs: Vec<SubSpec>) {
        let Some((change, ready, count)) = self.inner.try_update_value(|i| {
            let (change, ready) = i.conn.want(specs);
            (change, ready, i.conn.wanted_len())
        }) else {
            return;
        };
        let _ = self.subscribed.try_set(count);
        for key in &change.removed {
            if ready {
                self.send(&ClientMsg::Unsub { sub: key.clone() });
            }
        }
        // Slot::rewanted: Pending while the hub is connected, a known value
        // kept stale during an outage (#43, L2).
        for spec in &change.added {
            let _ = self
                .slot_signal(&spec.key())
                .try_update(|slot| *slot = std::mem::replace(slot, Slot::Pending).rewanted(ready));
            if ready {
                self.send_sub(spec);
            }
        }
        for key in &change.removed {
            let _ = self
                .slot_signal(key)
                .try_update(|slot| *slot = std::mem::replace(slot, Slot::Pending).rewanted(ready));
        }
    }

    /// The hub is connected again: a slot no page wants now (left during the
    /// outage, kept stale then) has nothing to keep it current, so it waits
    /// for a fresh value when it is wanted again (`Conn::wants`, #43).
    fn forget_unwanted(self) {
        let signals: Vec<ArcRwSignal<Slot>> = self
            .inner
            .try_with_value(|i| {
                i.slots
                    .iter()
                    .filter(|(key, _)| !i.conn.wants(key))
                    .map(|(_, slot)| slot.clone())
                    .collect()
            })
            .unwrap_or_default();
        for signal in signals {
            let _ = signal.try_set(Slot::Pending);
        }
    }

    /// REFRESH ALL (spec F6, F7): unsubscribe everything, unfold the
    /// configured groups, subscribe everything again (the hub resolves
    /// every name afresh). Every control waits for its new value (I8).
    pub fn refresh(self) {
        let Some(Some(specs)) = self.inner.try_with_value(|i| i.conn.refresh()) else {
            return;
        };
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
        self.fetch_ranges(None);
        let _ = self.refreshes.try_update(|n| *n += 1);
    }

    /// Sends a command batch; `done` gets the result slots (or why none).
    pub fn cmd(self, instance: &str, commands: Vec<LiveCommand>, done: Option<ResultFn>) {
        let Some(id) = self.inner.try_update_value(|i| i.conn.next_id()) else {
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

    /// `set_prop` of one property through a `cmd` (REFRESH ALL's unfold);
    /// `failed` hears why it failed (spec I6: shown, never retried).
    fn set_prop(
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

    /// A parameter's range (`min`, `max`), read from Live when first asked,
    /// when its instance comes back or loads another set, and on every
    /// refresh.
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

    /// Reads every known range again (of one instance, or all).
    fn fetch_ranges(self, instance: Option<&str>) {
        let all: Vec<((String, String), Range)> = self
            .inner
            .try_with_value(|i| {
                i.ranges
                    .iter()
                    .filter(|((name, _), _)| instance.is_none_or(|n| name == n))
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
                let _ = signal.try_update(|range| *range = next_range(*range, &outcome));
            })),
        );
    }
}
