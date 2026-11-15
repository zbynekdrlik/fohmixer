//! Listen (#210): a track's sound, before its fader, to the pages as Opus.
//!
//! The `FohMixer Listen` devices announce themselves over UDP
//! (`listen/udp.rs`), the hub selects the one on the track a page listens
//! to (the same socket, [`Listen::next_select`]), that device streams its
//! samples over TCP (`listen/stream.rs`: Jitter chunks, 20 ms frames, Opus)
//! and every listening socket (`listen/session.rs`, `/ws/audio`) gets the
//! packets from one `broadcast` channel. One track at a time on a hub: a
//! page asking for another while one is heard is told `busy`; listeners of
//! the same track share it, and the last one leaving ends the selects.
//!
//! This file is the shared state; its decisions are small pure functions
//! ([`choose`], [`start_of`], [`select_now`]) with their own tests.

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
use tokio::sync::{Notify, broadcast};

use crate::events::EventLog;

/// The packets the channel keeps for a slow socket (1.28 s).
pub const PACKETS: usize = 64;

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
    /// Another track is heard.
    Busy,
}

/// A start of `instance`'s `set`'s `track` against the target now.
pub fn start_of(target: Option<&Target>, instance: &str, set: &str, track: &str) -> Start {
    match target {
        None => Start::New,
        Some(t) if t.instance == instance && t.set == set && t.track == track => Start::Join,
        Some(_) => Start::Busy,
    }
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
    packets: broadcast::Sender<Bytes>,
    /// Wakes the select loop at a new target, so the device starts at once.
    pub wake: Notify,
    pub events: EventLog,
}

/// Another track is heard (one at a time on a hub).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// One socket's hold on the target: dropped, it leaves.
pub struct Lease {
    listen: Arc<Listen>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.listen.leave();
    }
}

impl Listen {
    pub fn new(events: EventLog) -> Arc<Self> {
        let (packets, _) = broadcast::channel(PACKETS);
        Arc::new(Self {
            state: Mutex::new(State::default()),
            counters: Counters::default(),
            packets,
            wake: Notify::new(),
            events,
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A page starts listening to `track` of `instance` (its Live set
    /// `set`): a lease, or [`Busy`] while another track is heard.
    pub fn start(self: &Arc<Self>, instance: &str, set: &str, track: &str) -> Result<Lease, Busy> {
        let mut state = self.state();
        match start_of(state.target.as_ref(), instance, set, track) {
            Start::Busy => return Err(Busy),
            Start::Join => {
                if let Some(t) = state.target.as_mut() {
                    t.listeners += 1;
                }
            }
            Start::New => {
                state.turn = state.turn.wrapping_add(1);
                let target = Target {
                    instance: instance.to_string(),
                    set: set.to_string(),
                    track: track.to_string(),
                    listeners: 1,
                };
                tracing::info!(instance, set, track, turn = state.turn, "listen: started");
                self.events
                    .record("listen", target_fields("start", &target, state.turn));
                state.target = Some(target);
                self.wake.notify_one();
            }
        }
        Ok(Lease {
            listen: Arc::clone(self),
        })
    }

    fn leave(&self) {
        let mut state = self.state();
        let turn = state.turn;
        let Some(target) = state.target.as_mut() else {
            return;
        };
        target.listeners = target.listeners.saturating_sub(1);
        if target.listeners == 0
            && let Some(target) = state.target.take()
        {
            tracing::info!(instance = %target.instance, track = %target.track, turn, "listen: stopped");
            self.events
                .record("listen", target_fields("stop", &target, turn));
            self.wake.notify_one();
        }
    }

    /// An announce heard at `now`; devices unheard for [`FORGET_MS`] go.
    pub fn heard(&self, announce: Announce, now: Instant) {
        let mut state = self.state();
        state.devices.retain(|_, h| fresh(h.at, now));
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

    /// A packet to every listening socket.
    pub fn send_packet(&self, packet: Bytes) {
        self.counters.packets.fetch_add(1, Ordering::Relaxed);
        // No socket listening: nobody keeps it.
        let _ = self.packets.send(packet);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
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
) -> (Arc<Listen>, Vec<tokio::task::JoinHandle<()>>) {
    let listen = Listen::new(events.clone());
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
