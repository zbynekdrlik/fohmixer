//! Listen (#210): a track's sound, before its fader, to the pages as Opus.
//!
//! The `FohMixer Listen` devices announce themselves over UDP
//! (`listen/udp.rs`), the hub selects the one on the track a page listens
//! to (the same socket, [`Listen::next_select`]), that device streams its
//! samples over TCP (`listen/stream.rs`: Jitter chunks, 20 ms frames, Opus)
//! and every listening socket (`listen/session.rs`, `/ws/audio`) gets the
//! packets from one `broadcast` channel. One track at a time on a hub, the
//! newest start wins (the main session's ruling on #210's review: a page
//! left open must never hold it): a start of another track takes it, a
//! `taken` record, and the pages listening to the old one are told `taken`
//! through the turn ([`Listen::turns`]) and stop; every packet carries the
//! turn it was sent in, so a page never hears another track's. Listeners
//! of the same track share it, and the last one leaving ends the selects.
//!
//! A device is picked by its track's and Live set's NAMES (its announce's
//! parents), never by the instance: the set's name is read again at every
//! decision ([`SetNames`]), so a set loaded while a page listens follows;
//! a track renamed meanwhile is no longer found (accepted on the review).
//!
//! This file is the shared state; its decisions are small pure functions
//! ([`choose`], [`start_of`], [`select_now`], [`selecting`]) with their own
//! tests.

pub mod encoder;
pub mod session;
pub mod stream;
pub mod udp;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;
use fohmixer_proto::listen::wire::{Announce, FORGET_MS, Select};
use fohmixer_proto::listen::{ListenDeviceStatus, ListenStatus, ListenTargetStatus};
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, watch};

use crate::events::EventLog;

/// The packets the channel keeps for a slow socket (1.28 s).
pub const PACKETS: usize = 64;
/// The most devices kept at once (both Lives' listenable tracks fit many
/// times; a flood of ids is counted, never kept).
pub const MAX_DEVICES: usize = 256;

/// Whether a device's announce is kept: one already known always, a new one
/// while fewer than [`MAX_DEVICES`] are.
pub fn keeps(known: bool, devices: usize) -> bool {
    known || devices < MAX_DEVICES
}

/// A device that matches a listened track, or why none does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Device(i32),
    NoDevice,
    TwoDevices,
}

impl Choice {
    /// The status's words for no device.
    pub fn problem(self) -> Option<&'static str> {
        match self {
            Choice::Device(_) => None,
            Choice::NoDevice => Some("no device"),
            Choice::TwoDevices => Some("two devices"),
        }
    }
}

/// A device heard: its last announce and when.
#[derive(Debug, Clone)]
pub struct Heard {
    pub announce: Announce,
    pub at: Instant,
}

/// Whether a device heard at `at` still counts at `now`.
pub fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < Duration::from_millis(FORGET_MS)
}

/// The one fresh device on `track` of the Live set `set`.
pub fn choose(devices: &BTreeMap<i32, Heard>, set: &str, track: &str, now: Instant) -> Choice {
    let mut found = devices
        .values()
        .filter(|h| fresh(h.at, now) && h.announce.track_and_set() == Some((track, set)));
    match (found.next(), found.next()) {
        (Some(one), None) => Choice::Device(one.announce.device),
        (None, _) => Choice::NoDevice,
        (Some(_), Some(_)) => Choice::TwoDevices,
    }
}

/// The track listened to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub instance: String,
    pub set: String,
    pub track: String,
    pub listeners: usize,
}

/// What a page's start does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Nothing heard: the track becomes the target.
    New,
    /// The same track is heard: one more listener.
    Join,
    /// Another track is heard: the start takes the hub's one track from it.
    Take,
}

/// A start of `instance`'s `set`'s `track` against the target now.
pub fn start_of(target: Option<&Target>, instance: &str, set: &str, track: &str) -> Start {
    match target {
        None => Start::New,
        Some(t) if t.instance == instance && t.set == set && t.track == track => Start::Join,
        Some(_) => Start::Take,
    }
}

/// Whether the select loop has anything to send: while a target exists, or
/// while the stop (a select of 0) is still owed after one ended (`last`:
/// the device the last select named).
pub fn selecting(has_target: bool, last: Option<i32>) -> bool {
    has_target || last.is_some_and(|id| id != 0)
}

/// The Live set's name of an instance as its script told it now; none while
/// it is unknown (the hub's `HubInner::set_name`).
pub type SetNames = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A [`SetNames`] that knows no set: the target keeps the name it started
/// with (the tests' and a hub without Live instances).
pub fn no_sets() -> SetNames {
    Arc::new(|_| None)
}

/// The device the next select names: the chosen one while a target has
/// one, 0 (stop) while a target has none, and once more 0 after the target
/// ended (`last`: the device the last select named); none otherwise.
pub fn select_now(choice: Option<Choice>, last: Option<i32>) -> Option<i32> {
    match choice {
        Some(Choice::Device(id)) => Some(id),
        Some(_) => Some(0),
        None => last.filter(|&id| id != 0).map(|_| 0),
    }
}

#[derive(Debug, Default)]
struct State {
    devices: BTreeMap<i32, Heard>,
    target: Option<Target>,
    turn: i32,
    last_select: Option<i32>,
}

/// The counts `/api/status` shows.
#[derive(Debug, Default)]
pub struct Counters {
    pub frames: AtomicU64,
    pub packets: AtomicU64,
    pub selects: AtomicU64,
    pub connections: AtomicUsize,
    refused: Mutex<BTreeMap<String, u64>>,
    bound: Mutex<(Option<String>, Option<String>)>,
}

impl Counters {
    /// One thing refused for `reason`.
    pub fn refuse(&self, reason: &str) {
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        *refused.entry(reason.to_string()).or_insert(0) += 1;
    }

    /// The address bound, or why not.
    pub fn set_bound(&self, addr: Option<String>, error: Option<String>) {
        *self.bound.lock().unwrap_or_else(PoisonError::into_inner) = (addr, error);
    }

    fn refused(&self) -> BTreeMap<String, u64> {
        self.refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn bound(&self) -> (Option<String>, Option<String>) {
        self.bound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The `listen` record of a start that took the track from `from`'s
/// listeners, by the page `client` at `peer`.
pub fn taken_fields(from: &Target, to: &Target, client: u64, peer: &str) -> Value {
    let side = |t: &Target| json!({"instance": t.instance, "set": t.set, "track": t.track, "listeners": t.listeners});
    json!({"what": "taken", "from": side(from), "to": side(to), "client": client, "peer": peer})
}

/// The `listen` record of a target's start or end.
pub fn target_fields(what: &str, target: &Target, turn: i32) -> Value {
    json!({
        "what": what,
        "instance": target.instance,
        "set": target.set,
        "track": target.track,
        "turn": turn,
    })
}

/// Listen's shared state: the devices heard, the target, the packets'
/// channel and the counts.
pub struct Listen {
    state: Mutex<State>,
    pub counters: Counters,
    /// The packets, each with the turn it was sent in.
    packets: broadcast::Sender<(i32, Bytes)>,
    /// The turn now: a page whose turn is no longer it was taken.
    turns: watch::Sender<i32>,
    /// Wakes the select loop at a new target, so the device starts at once.
    pub wake: Notify,
    pub events: EventLog,
    sets: SetNames,
}

/// One socket's hold on the target of its turn: dropped, it leaves (a
/// lease of an older turn, its track taken, leaves nothing).
pub struct Lease {
    listen: Arc<Listen>,
    turn: i32,
}

impl Lease {
    /// The turn it holds.
    pub fn turn(&self) -> i32 {
        self.turn
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.listen.leave(self.turn);
    }
}

impl Listen {
    pub fn new(events: EventLog, sets: SetNames) -> Arc<Self> {
        let (packets, _) = broadcast::channel(PACKETS);
        Arc::new(Self {
            state: Mutex::new(State::default()),
            counters: Counters::default(),
            packets,
            turns: watch::Sender::new(0),
            wake: Notify::new(),
            events,
            sets,
        })
    }

    /// The state, the target's set name read again ([`SetNames`]; kept
    /// while the instance's set is unknown).
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(t) = state.target.as_mut()
            && let Some(set) = (self.sets)(&t.instance)
        {
            t.set = set;
        }
        state
    }

    /// A page (`client` at `peer`) starts listening to `track` of
    /// `instance` (its Live set `set`): one more listener of the same
    /// track, or the track from now on, taken from its listeners.
    pub fn start(
        self: &Arc<Self>,
        instance: &str,
        set: &str,
        track: &str,
        client: u64,
        peer: &str,
    ) -> Lease {
        let mut state = self.state();
        let to = Target {
            instance: instance.to_string(),
            set: set.to_string(),
            track: track.to_string(),
            listeners: 1,
        };
        match start_of(state.target.as_ref(), instance, set, track) {
            Start::Join => {
                if let Some(t) = state.target.as_mut() {
                    t.listeners += 1;
                }
            }
            start => {
                state.turn = state.turn.wrapping_add(1);
                if let (Start::Take, Some(from)) = (start, state.target.take()) {
                    tracing::info!(from = ?from.track, to = ?track, listeners = from.listeners, client, peer = ?peer, "listen: a start took the track");
                    self.events
                        .record("listen", taken_fields(&from, &to, client, peer));
                }
                tracing::info!(instance = ?instance, set = ?set, track = ?track, turn = state.turn, "listen: started");
                self.events
                    .record("listen", target_fields("start", &to, state.turn));
                state.target = Some(to);
                self.turns.send_replace(state.turn);
                self.wake.notify_one();
            }
        }
        Lease {
            listen: Arc::clone(self),
            turn: state.turn,
        }
    }

    /// The turn now, to watch: a page holding an older one was taken.
    pub fn turns(&self) -> watch::Receiver<i32> {
        self.turns.subscribe()
    }

    /// Whether the select loop has a select to send ([`selecting`]).
    pub fn selecting(&self) -> bool {
        let state = self.state();
        selecting(state.target.is_some(), state.last_select)
    }

    fn leave(&self, turn: i32) {
        let mut state = self.state();
        if state.turn != turn {
            return;
        }
        let Some(target) = state.target.as_mut() else {
            return;
        };
        target.listeners = target.listeners.saturating_sub(1);
        if target.listeners == 0
            && let Some(target) = state.target.take()
        {
            tracing::info!(instance = ?target.instance, track = ?target.track, turn, "listen: stopped");
            self.events
                .record("listen", target_fields("stop", &target, turn));
            self.wake.notify_one();
        }
    }

    /// An announce heard at `now`; devices unheard for [`FORGET_MS`] go,
    /// and a new one past [`MAX_DEVICES`] is counted (`devices`), not kept.
    pub fn heard(&self, announce: Announce, now: Instant) {
        let mut state = self.state();
        state.devices.retain(|_, h| fresh(h.at, now));
        let known = state.devices.contains_key(&announce.device);
        if !keeps(known, state.devices.len()) {
            self.counters.refuse("devices");
            return;
        }
        state
            .devices
            .insert(announce.device, Heard { announce, at: now });
    }

    /// Whether `device`'s stream is the one listened to at `now`.
    pub fn accepts(&self, device: i32, now: Instant) -> bool {
        let state = self.state();
        state.target.as_ref().is_some_and(|t| {
            choose(&state.devices, &t.set, &t.track, now) == Choice::Device(device)
        })
    }

    /// The select to send at `now`, if any (counted and remembered).
    pub fn next_select(&self, now: Instant) -> Option<Select> {
        let mut state = self.state();
        let choice = state
            .target
            .as_ref()
            .map(|t| choose(&state.devices, &t.set, &t.track, now));
        let device = select_now(choice, state.last_select)?;
        state.last_select = Some(device);
        self.counters.selects.fetch_add(1, Ordering::Relaxed);
        Some(Select {
            device,
            turn: state.turn,
        })
    }

    /// A packet to every listening socket, with the turn it is sent in.
    pub fn send_packet(&self, packet: Bytes) {
        let turn = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .turn;
        self.counters.packets.fetch_add(1, Ordering::Relaxed);
        // No socket listening: nobody keeps it.
        let _ = self.packets.send((turn, packet));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(i32, Bytes)> {
        self.packets.subscribe()
    }

    /// `/api/status` `listen` at `now`.
    pub fn status(&self, now: Instant) -> ListenStatus {
        let state = self.state();
        let devices = state
            .devices
            .values()
            .filter(|h| fresh(h.at, now))
            .map(|h| {
                let (track, set) = h.announce.track_and_set().unwrap_or(("", ""));
                ListenDeviceStatus {
                    device: h.announce.device,
                    version: h.announce.version.clone(),
                    track: track.to_string(),
                    set: set.to_string(),
                    sample_rate: h.announce.sample_rate,
                    sending: h.announce.sending,
                    age_ms: u64::try_from(now.saturating_duration_since(h.at).as_millis())
                        .unwrap_or(u64::MAX),
                }
            })
            .collect();
        let target = state.target.as_ref().map(|t| {
            let choice = choose(&state.devices, &t.set, &t.track, now);
            ListenTargetStatus {
                instance: t.instance.clone(),
                set: t.set.clone(),
                track: t.track.clone(),
                listeners: t.listeners,
                device: match choice {
                    Choice::Device(id) => Some(id),
                    _ => None,
                },
                problem: choice.problem().map(str::to_string),
            }
        });
        let (addr, error) = self.counters.bound();
        ListenStatus {
            addr,
            error,
            devices,
            target,
            connections: self.counters.connections.load(Ordering::Relaxed),
            frames: self.counters.frames.load(Ordering::Relaxed),
            packets: self.counters.packets.load(Ordering::Relaxed),
            selects: self.counters.selects.load(Ordering::Relaxed),
            refused: self.counters.refused(),
        }
    }
}

/// Binds `127.0.0.1:<port>` for both: TCP first (a port 0 picks a free
/// one), then UDP on the same number.
pub fn bind(port: u16) -> std::io::Result<(std::net::TcpListener, std::net::UdpSocket)> {
    let tcp = std::net::TcpListener::bind(udp::bind_addr(port))?;
    let bound = tcp.local_addr()?.port();
    let udp = std::net::UdpSocket::bind(udp::bind_addr(bound))?;
    Ok((tcp, udp))
}

/// Starts Listen (inside a tokio runtime): its state and tasks. A port that
/// cannot be bound is one `listen` record with its error and an error line,
/// shown in `/api/status`; the hub runs on, every page told `no_source`.
pub fn spawn(
    port: u16,
    select_port: u16,
    events: EventLog,
    sets: SetNames,
) -> (Arc<Listen>, Vec<tokio::task::JoinHandle<()>>) {
    let listen = Listen::new(events.clone(), sets);
    let wanted = udp::bind_addr(port).to_string();
    let started = bind(port).and_then(|(tcp, socket)| {
        let addr = tcp.local_addr()?.to_string();
        let tasks = vec![
            stream::spawn(tcp, Arc::clone(&listen))?,
            udp::spawn(socket, select_port, Arc::clone(&listen))?,
        ];
        Ok((addr, tasks))
    });
    match started {
        Ok((addr, tasks)) => {
            tracing::info!(addr = %addr, select_port, "listen: listening for the Listen devices");
            events.record("listen", udp::listen_fields(&addr, None));
            listen.counters.set_bound(Some(addr), None);
            (listen, tasks)
        }
        Err(e) => {
            let error = e.to_string();
            tracing::error!(addr = %wanted, error = %error, "listen: cannot listen for the Listen devices; the hub runs on without Listen");
            events.record("listen", udp::listen_fields(&wanted, Some(&error)));
            listen.counters.set_bound(None, Some(error));
            (listen, Vec::new())
        }
    }
}

#[cfg(test)]
mod tests;
