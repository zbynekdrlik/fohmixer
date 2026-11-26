//! The LUFS devices' listener (#151, spec §4.4, §5.2): one tokio task on a
//! UDP socket bound to `127.0.0.1:<[lufs] port>` and nothing else (no
//! config can widen it: the address is fixed here). Each datagram is
//! parsed (`fohmixer_proto::lufs::wire::parse`, pure) and passed through
//! the per-key bound (`rules::lufs::bound`) before it reaches the router as
//! [`RouterMsg::Lufs`]; a refused, dropped or full datagram is only counted
//! ([`Counters`], `/api/status` `lufs.listener`), never one record each. A
//! key that starts dropping is one `lufs` record (warn) and one warn line,
//! and one more when it was under the bound for 10 s.
//!
//! A bind that fails is one `lufs` record with its error and an error line,
//! shown in `/api/status`; the hub runs on without LUFS.
//!
//! **The polls (#264):** a device of the polled form names the port it
//! listens on (`Reading.port`); the same task sends each such port
//! [`wire::poll_datagram`] of its key's kind at that kind's rate
//! ([`poll_every`]: a LUFS device every 200 ms, a meter every 33 ms, one
//! round per kind, [`POLLED_KINDS`]) from a socket of its own (bound to
//! `127.0.0.1`, never read: an ICMP reset from a port that closed lands
//! there, not in the listener's receive), and the device answers each with
//! one reading, pulled in Max's main thread. A port is polled while its key
//! is heard ([`Polls`]): a key lost (`rules::lufs::is_stale`, 1 s for a
//! LUFS key, 0.5 s for a meter key) drops it, so a device switched off (it
//! answers nothing) or gone is polled for one stale window at most, and a
//! device moved (a new key, the same port) is polled once per round.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::client::LufsListenerStatus;
use fohmixer_proto::lufs::wire::{self, DeviceKind, MAX_DATAGRAM};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::EventLog;
use crate::link::server::Tap;
use crate::router::RouterMsg;
use crate::rules::lufs::bound::{Admit, Bound, Dropping, FORGET_MS, Key};
use crate::rules::lufs::{is_stale, kind_word};

/// The reason a receive error is counted under.
pub const RECV_ERROR: &str = "recv";
/// How often the bound forgets its unheard keys.
pub const SWEEP: Duration = Duration::from_secs(1);
/// The pause after a receive error (a socket error never spins the task).
pub const ERROR_PAUSE: Duration = Duration::from_millis(10);

/// The listener's counts and state, read by `/api/status`.
#[derive(Debug, Default)]
pub struct Counters {
    taken: AtomicU64,
    dropped: AtomicU64,
    full: AtomicU64,
    polled: AtomicU64,
    poll_failed: AtomicU64,
    polling: AtomicU64,
    refused: Mutex<BTreeMap<String, u64>>,
    addr: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
}

impl Counters {
    /// One datagram refused for `reason`.
    pub fn refuse(&self, reason: &str) {
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        *refused.entry(reason.to_string()).or_insert(0) += 1;
    }

    /// One datagram the bound took, dropped or found no room for.
    pub fn admitted(&self, admit: Admit) {
        let counter = match admit {
            Admit::Take => &self.taken,
            Admit::Drop => &self.dropped,
            Admit::Full => &self.full,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// One round of polls: `sent` datagrams went, `failed` did not, to
    /// `ports` ports.
    pub fn polled(&self, sent: u64, failed: u64, ports: u64) {
        self.polled.fetch_add(sent, Ordering::Relaxed);
        self.poll_failed.fetch_add(failed, Ordering::Relaxed);
        self.polling.store(ports, Ordering::Relaxed);
    }

    fn set_addr(&self, addr: Option<String>, error: Option<String>) {
        *self.addr.lock().unwrap_or_else(PoisonError::into_inner) = addr;
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = error;
    }

    /// `/api/status` `lufs.listener`.
    pub fn status(&self) -> LufsListenerStatus {
        LufsListenerStatus {
            addr: self
                .addr
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            error: self
                .error
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            taken: self.taken.load(Ordering::Relaxed),
            refused: self
                .refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            dropped: self.dropped.load(Ordering::Relaxed),
            full: self.full.load(Ordering::Relaxed),
            polled: self.polled.load(Ordering::Relaxed),
            poll_failed: self.poll_failed.load(Ordering::Relaxed),
            polling: self.polling.load(Ordering::Relaxed),
        }
    }
}

/// The ports to poll (#264): each polled-form key's port and when the key
/// was last heard. A key is kept only while the bound keeps it (at most
/// `bound::MAX_KEYS`), and is dropped once lost.
#[derive(Debug, Default)]
pub struct Polls {
    keys: BTreeMap<Key, (u16, Instant)>,
}

impl Polls {
    /// A reading the bound kept (taken or dropped, never `Full`): its key's
    /// port, heard at `now`. A pushed-form reading names none.
    pub fn heard(&mut self, key: &Key, port: Option<u16>, now: Instant) {
        if let Some(port) = port {
            self.keys.insert(key.clone(), (port, now));
        }
    }

    /// The ports of `kind`'s keys to poll at `now`, each once: the lost
    /// keys of every kind are dropped first.
    pub fn due(&mut self, now: Instant, kind: DeviceKind) -> BTreeSet<u16> {
        self.keys.retain(|key, (_, heard)| !lost(key, *heard, now));
        self.keys
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .map(|(_, (port, _))| *port)
            .collect()
    }

    /// The ports polled now, of every kind, each once.
    pub fn ports(&self) -> usize {
        let ports: BTreeSet<u16> = self.keys.values().map(|(port, _)| *port).collect();
        ports.len()
    }

    /// The keys polled.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// No key polled.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Whether a key last heard at `heard` is lost at `now` (the LUFS rule's
/// stale window; a clock that went back is no loss).
pub fn lost(key: &Key, heard: Instant, now: Instant) -> bool {
    let ms = now.saturating_duration_since(heard).as_millis();
    is_stale(key.kind, 0, u64::try_from(ms).unwrap_or(u64::MAX))
}

/// The address a poll goes to: the device's port on `127.0.0.1`.
pub fn poll_addr(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// The only address the listener binds: `127.0.0.1:<port>`.
pub fn bind_addr(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// The `lufs` record of the listener bound or failed.
pub fn listen_fields(addr: &str, error: Option<&str>) -> Value {
    match error {
        None => json!({"what": "listening", "addr": addr}),
        Some(error) => json!({"what": "listening", "addr": addr, "error": error, "warn": true}),
    }
}

/// The `lufs` record of a key's change of dropping.
pub fn dropping_fields(key: &Key, change: Dropping) -> Value {
    let started = change == Dropping::Started;
    let what = if started { "dropping" } else { "under_bound" };
    let mut fields = json!({
        "what": what,
        "kind": kind_word(key.kind),
        "device": key.device,
        "parents": key.parents,
    });
    if started {
        fields["warn"] = json!(true);
    }
    fields
}

/// Binds `127.0.0.1:<port>` and starts the listener (inside a tokio
/// runtime): its address and task, or the bind's error (recorded and in
/// the counters' status). With a `tap` (#184, the instance link) every
/// datagram read is also handed to it as read, before it is parsed: the
/// dialling hub parses and bounds it itself.
pub fn spawn(
    port: u16,
    tx: mpsc::UnboundedSender<RouterMsg>,
    counters: Arc<Counters>,
    events: EventLog,
    tap: Option<Tap>,
) -> io::Result<(SocketAddr, JoinHandle<()>)> {
    let wanted = bind_addr(port);
    let bound = udp_socket(wanted).and_then(|socket| Ok((socket, poll_socket()?)));
    let (socket, poller) = match bound {
        Ok(sockets) => sockets,
        Err(e) => {
            let error = e.to_string();
            tracing::error!(addr = %wanted, error = %error, "lufs: cannot listen for the LUFS devices; the hub runs on without them");
            events.record(
                "lufs",
                listen_fields(&wanted.to_string(), Some(error.as_str())),
            );
            counters.set_addr(None, Some(error));
            return Err(e);
        }
    };
    let addr = socket.local_addr()?;
    tracing::info!(addr = %addr, "lufs: listening for the LUFS devices");
    events.record("lufs", listen_fields(&addr.to_string(), None));
    counters.set_addr(Some(addr.to_string()), None);
    let task = tokio::spawn(listen(socket, poller, tx, counters, events, tap));
    Ok((addr, task))
}

/// A non-blocking tokio UDP socket bound to `addr`.
fn udp_socket(addr: SocketAddrV4) -> io::Result<tokio::net::UdpSocket> {
    let socket = std::net::UdpSocket::bind(addr)?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket)
}

/// The polls' socket: `127.0.0.1`, any port, non-blocking (a send that
/// would block is a failed poll, never a wait in the listener's task).
fn poll_socket() -> io::Result<std::net::UdpSocket> {
    let socket = std::net::UdpSocket::bind(bind_addr(0))?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// The kinds the hub polls, each in a round of its own.
pub const POLLED_KINDS: [DeviceKind; 2] = [DeviceKind::Lufs, DeviceKind::Meter];

/// How often a kind's round runs ([`DeviceKind::poll_every_ms`]).
pub fn poll_every(kind: DeviceKind) -> Duration {
    Duration::from_millis(kind.poll_every_ms())
}

/// Sends one poll of `kind` to each of its ports due at `now` from
/// `poller`, counted; the ports polled now are every kind's.
pub fn poll_round(
    poller: &std::net::UdpSocket,
    polls: &mut Polls,
    kind: DeviceKind,
    now: Instant,
    counters: &Counters,
) {
    let ports = polls.due(now, kind);
    let poll = wire::poll_datagram(kind);
    let mut failed = 0;
    for port in &ports {
        if let Err(e) = poller.send_to(&poll, poll_addr(*port)) {
            tracing::debug!(port, error = %e, "lufs: a poll could not be sent");
            failed += 1;
        }
    }
    let (sent, failed) = round_counts(ports.len(), failed);
    let polling = u64::try_from(polls.ports()).unwrap_or(u64::MAX);
    counters.polled(sent, failed, polling);
}

/// A round's counts: the polls sent and failed, of `ports` ports with
/// `failed` sends refused.
pub fn round_counts(ports: usize, failed: u64) -> (u64, u64) {
    let ports = u64::try_from(ports).unwrap_or(u64::MAX);
    (ports.saturating_sub(failed), failed)
}

/// A kind's round clock: its first tick at once, then on its grid: a late
/// tick (Windows wakes a timer up to ~15.6 ms late) never moves the next
/// one out, and a missed tick is skipped (a stalled task never sends a
/// burst of polls).
fn round_clock(kind: DeviceKind) -> tokio::time::Interval {
    let mut clock = tokio::time::interval(poll_every(kind));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    clock
}

/// One datagram's way: parsed, bounded, counted, to the router when taken.
/// False once the router is gone.
/// With `polls` (this hub's own listener, never a link's) a kept
/// reading's port is polled from then on.
pub fn handle_datagram(
    data: &[u8],
    now: Instant,
    bound: &mut Bound,
    polls: Option<&mut Polls>,
    counters: &Counters,
    events: &EventLog,
    tx: &mpsc::UnboundedSender<RouterMsg>,
) -> bool {
    let reading = match wire::parse(data) {
        Ok(reading) => reading,
        Err(e) => {
            counters.refuse(e.reason());
            return true;
        }
    };
    let key = Key::of(&reading);
    let (admit, change) = bound.admit(&key, now);
    counters.admitted(admit);
    if admit != Admit::Full
        && let Some(polls) = polls
    {
        polls.heard(&key, reading.port, now);
    }
    if let Some(change) = change {
        let fields = dropping_fields(&key, change);
        match change {
            Dropping::Started => {
                tracing::warn!(device = key.device, parents = ?key.parents, "lufs: a device sends over its bound: dropping");
            }
            Dropping::Ended => {
                tracing::info!(device = key.device, parents = ?key.parents, "lufs: a device is under its bound again");
            }
        }
        events.record("lufs", fields);
    }
    if admit != Admit::Take {
        return true;
    }
    tx.send(RouterMsg::Lufs { reading }).is_ok()
}

/// The LUFS datagrams an instance link passes on (#184, the dialling
/// hub's side): each goes through [`handle_datagram`] with the hub's own
/// counters and a bound of the link's own, as if this hub's listener had
/// read it. One per linked instance.
pub struct LinkedLufs {
    bound: Mutex<(Bound, Instant)>,
    counters: Arc<Counters>,
    events: EventLog,
    tx: mpsc::UnboundedSender<RouterMsg>,
}

impl LinkedLufs {
    pub fn new(
        counters: Arc<Counters>,
        events: EventLog,
        tx: mpsc::UnboundedSender<RouterMsg>,
    ) -> Self {
        Self {
            bound: Mutex::new((Bound::new(), Instant::now())),
            counters,
            events,
            tx,
        }
    }

    /// One datagram from the link, at `now`: false once the router is gone.
    pub fn take(&self, data: &[u8], now: Instant) -> bool {
        let mut guard = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
        let (bound, swept) = &mut *guard;
        let alive = handle_datagram(
            data,
            now,
            bound,
            None,
            &self.counters,
            &self.events,
            &self.tx,
        );
        if sweep_due(*swept, now) {
            *swept = now;
            if let Some(cutoff) = now.checked_sub(Duration::from_millis(FORGET_MS)) {
                bound.forget_before(cutoff);
            }
        }
        alive
    }

    /// The keys its bound keeps.
    pub fn keys(&self) -> usize {
        self.bound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .len()
    }
}

/// Whether the bound forgets its unheard keys at `now`: the last sweep
/// (`swept`) is [`SWEEP`] or longer ago.
pub fn sweep_due(swept: Instant, now: Instant) -> bool {
    now.saturating_duration_since(swept) >= SWEEP
}

async fn listen(
    socket: tokio::net::UdpSocket,
    poller: std::net::UdpSocket,
    tx: mpsc::UnboundedSender<RouterMsg>,
    counters: Arc<Counters>,
    events: EventLog,
    tap: Option<Tap>,
) {
    // One byte over the limit, so a datagram too long reads as too long.
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut bound = Bound::new();
    let mut polls = Polls::default();
    let mut swept = Instant::now();
    let [lufs, meter] = POLLED_KINDS;
    let mut lufs_clock = round_clock(lufs);
    let mut meter_clock = round_clock(meter);
    loop {
        let received = tokio::select! {
            received = socket.recv(&mut buf) => received,
            _ = lufs_clock.tick() => {
                poll_round(&poller, &mut polls, lufs, Instant::now(), &counters);
                continue;
            }
            _ = meter_clock.tick() => {
                poll_round(&poller, &mut polls, meter, Instant::now(), &counters);
                continue;
            }
        };
        let n = match received {
            Ok(n) => n,
            Err(e) => {
                // A datagram larger than the buffer is an error on Windows;
                // an ICMP reset of an earlier send too: counted, never fatal.
                tracing::debug!(error = %e, "lufs: a receive failed");
                counters.refuse(RECV_ERROR);
                tokio::time::sleep(ERROR_PAUSE).await;
                continue;
            }
        };
        if let Some(tap) = &tap {
            // No link open: nobody subscribed, nothing kept.
            let _ = tap.send(Arc::from(&buf[..n]));
        }
        let now = Instant::now();
        if !handle_datagram(
            &buf[..n],
            now,
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx,
        ) {
            return;
        }
        if sweep_due(swept, now) {
            swept = now;
            if let Some(cutoff) = now.checked_sub(Duration::from_millis(FORGET_MS)) {
                bound.forget_before(cutoff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::lufs::wire::{DeviceKind, Reading, encode};

    fn datagram(device: i32) -> Vec<u8> {
        encode(&Reading {
            kind: DeviceKind::Lufs,
            device,
            version: "v1.19.0".to_string(),
            parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
            first: -300,
            second: -253,
            port: None,
        })
    }

    #[test]
    fn the_address_is_localhost_only() {
        assert_eq!(bind_addr(39110).to_string(), "127.0.0.1:39110");
        assert_eq!(*bind_addr(0).ip(), Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn a_datagram_is_taken_bounded_or_refused_and_counted() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let counters = Counters::default();
        let (events, records) = EventLog::channel(16);
        let mut bound = Bound::new();
        let t0 = Instant::now();
        for _ in 0..11 {
            assert!(handle_datagram(
                &datagram(1),
                t0,
                &mut bound,
                None,
                &counters,
                &events,
                &tx
            ));
        }
        assert!(handle_datagram(
            b"junk", t0, &mut bound, None, &counters, &events, &tx
        ));
        let status = counters.status();
        assert_eq!((status.taken, status.dropped, status.full), (10, 1, 0));
        assert_eq!(status.refused.get("truncated"), Some(&1));
        let mut got = 0;
        while let Ok(RouterMsg::Lufs { reading }) = rx.try_recv() {
            assert_eq!(reading.second, -253);
            got += 1;
        }
        assert_eq!(got, 10);
        let record = records.try_recv().expect("one dropping record");
        assert_eq!(record["ev"], "lufs");
        assert_eq!(record["what"], "dropping");
        assert!(records.try_recv().is_err(), "once per key");
        drop(rx);
        assert!(
            !handle_datagram(
                &datagram(1),
                t0 + Duration::from_secs(2),
                &mut bound,
                None,
                &counters,
                &events,
                &tx
            ),
            "the router is gone"
        );
    }

    #[test]
    fn a_linked_datagram_is_parsed_bounded_and_counted_as_the_listeners() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let counters = Arc::new(Counters::default());
        let (events, _records) = EventLog::channel(16);
        let linked = LinkedLufs::new(Arc::clone(&counters), events, tx);
        let t0 = Instant::now();
        assert_eq!(linked.keys(), 0, "nothing heard yet");
        for _ in 0..11 {
            assert!(linked.take(&datagram(2), t0));
        }
        assert!(linked.take(b"junk", t0));
        let status = counters.status();
        assert_eq!((status.taken, status.dropped), (10, 1), "its own bound");
        assert_eq!(status.refused.get("truncated"), Some(&1));
        let mut got = 0;
        while let Ok(RouterMsg::Lufs { reading }) = rx.try_recv() {
            assert_eq!(reading.device, 2);
            got += 1;
        }
        assert_eq!(got, 10);
        // After the sweep's second the key's window is new again.
        assert!(linked.take(&datagram(2), t0 + Duration::from_millis(1500)));
        assert_eq!(counters.status().taken, 11);
        assert_eq!(linked.keys(), 1);
        assert!(linked.take(&datagram(4), t0 + Duration::from_millis(1500)));
        assert_eq!(linked.keys(), 2, "a second device is a second key");
        // A key unheard for FORGET_MS is forgotten at the next sweep.
        let later = t0 + Duration::from_millis(1500 + FORGET_MS + 1);
        assert!(linked.take(&datagram(3), later));
        assert_eq!(linked.keys(), 1, "devices 2 and 4 forgotten, device 3 kept");
        drop(rx);
        assert!(!linked.take(&datagram(2), later + Duration::from_secs(3)));
    }

    #[tokio::test]
    async fn the_tap_gets_every_datagram_as_read() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let counters = Arc::new(Counters::default());
        let (events, _records) = EventLog::channel(16);
        let tap = crate::link::server::tap();
        let mut heard = tap.subscribe();
        let (addr, task) = spawn(0, tx, counters, events, Some(tap.clone()), false).unwrap();
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.send_to(&datagram(4), addr).unwrap();
        sender.send_to(b"junk", addr).unwrap();
        let first = tokio::time::timeout(Duration::from_secs(5), heard.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&*first, datagram(4).as_slice());
        let second = tokio::time::timeout(Duration::from_secs(5), heard.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&*second, b"junk", "before it is parsed");
        let reading = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap();
        assert!(matches!(reading, Some(RouterMsg::Lufs { .. })));
        task.abort();
    }

    #[test]
    fn the_records_fields() {
        assert_eq!(
            listen_fields("127.0.0.1:39110", None),
            json!({"what": "listening", "addr": "127.0.0.1:39110"})
        );
        assert_eq!(
            listen_fields("127.0.0.1:39110", Some("in use"))["warn"],
            json!(true)
        );
        let key = Key::of(&wire::parse(&datagram(3)).unwrap());
        let started = dropping_fields(&key, Dropping::Started);
        assert_eq!(started["what"], "dropping");
        assert_eq!(started["warn"], true);
        assert_eq!(started["device"], 3);
        let ended = dropping_fields(&key, Dropping::Ended);
        assert_eq!(ended["what"], "under_bound");
        assert!(ended.get("warn").is_none());
    }

    #[test]
    fn the_bound_is_swept_once_a_second() {
        // PR #155's mutation gate: the sweep's decision, on its boundary.
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(SWEEP, ms(1000));
        assert!(!sweep_due(t0, t0));
        assert!(!sweep_due(t0, t0 + ms(999)));
        assert!(sweep_due(t0, t0 + ms(1000)));
        assert!(sweep_due(t0, t0 + ms(2500)));
        assert!(!sweep_due(t0 + ms(1000), t0), "a clock that went back");
    }

    #[test]
    fn counters_count_each_kind() {
        let counters = Counters::default();
        counters.admitted(Admit::Take);
        counters.admitted(Admit::Drop);
        counters.admitted(Admit::Drop);
        counters.admitted(Admit::Full);
        counters.refuse("tags");
        counters.refuse("tags");
        counters.refuse(RECV_ERROR);
        let status = counters.status();
        assert_eq!((status.taken, status.dropped, status.full), (1, 2, 1));
        assert_eq!(status.refused.get("tags"), Some(&2));
        assert_eq!(status.refused.get("recv"), Some(&1));
        assert_eq!((status.addr, status.error), (None, None));
    }

    fn polled_datagram(device: i32, port: u16) -> Vec<u8> {
        encode(&Reading {
            kind: DeviceKind::Lufs,
            device,
            version: "v1.58.0".to_string(),
            parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
            first: -300,
            second: -253,
            port: Some(port),
        })
    }

    fn key(kind: DeviceKind, device: i32) -> Key {
        Key {
            kind,
            device,
            parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
        }
    }

    #[test]
    fn a_polled_key_is_polled_until_it_is_lost() {
        // #264: the port is polled while its key is heard; a LUFS key is
        // lost after 1 s, a meter key after 0.5 s (the rule's windows).
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut polls = Polls::default();
        assert!(polls.is_empty());
        polls.heard(&key(DeviceKind::Lufs, 1), None, t0);
        assert!(polls.is_empty(), "a pushed-form reading names no port");
        polls.heard(&key(DeviceKind::Lufs, 1), Some(41_000), t0);
        assert!(!polls.is_empty());
        polls.heard(&key(DeviceKind::Lufs, 2), Some(41_000), t0 + ms(300));
        polls.heard(&key(DeviceKind::Meter, 3), Some(42_000), t0);
        assert_eq!(polls.len(), 3);
        assert_eq!(polls.ports(), 2, "a port shared by two keys counts once");
        assert_eq!(
            polls.due(t0 + ms(499), DeviceKind::Lufs),
            BTreeSet::from([41_000]),
            "a port shared by two keys (a device moved) is polled once"
        );
        assert_eq!(
            polls.due(t0 + ms(499), DeviceKind::Meter),
            BTreeSet::from([42_000]),
            "each kind's round polls its own keys only"
        );
        assert!(polls.due(t0 + ms(500), DeviceKind::Meter).is_empty());
        assert_eq!(polls.len(), 2, "the meter key lost at 500 ms");
        assert_eq!(polls.ports(), 1);
        assert_eq!(
            polls.due(t0 + ms(999), DeviceKind::Lufs),
            BTreeSet::from([41_000])
        );
        assert_eq!(
            polls.due(t0 + ms(1000), DeviceKind::Lufs),
            BTreeSet::from([41_000])
        );
        assert_eq!(polls.len(), 1, "key 1 lost at 1000 ms, key 2 heard later");
        // Heard again: kept another window.
        polls.heard(&key(DeviceKind::Lufs, 2), Some(41_000), t0 + ms(1200));
        assert_eq!(
            polls.due(t0 + ms(2199), DeviceKind::Lufs),
            BTreeSet::from([41_000])
        );
        // A round of the other kind drops the lost keys too.
        assert!(polls.due(t0 + ms(2200), DeviceKind::Meter).is_empty());
        assert!(polls.is_empty());
        assert_eq!(polls.ports(), 0);
    }

    #[test]
    fn each_kind_is_polled_at_its_own_rate() {
        // #264: the meter every 33 ms (30 a second), LUFS every 200 ms.
        assert_eq!(poll_every(DeviceKind::Meter), Duration::from_millis(33));
        assert_eq!(poll_every(DeviceKind::Lufs), Duration::from_millis(200));
        assert_eq!(POLLED_KINDS, [DeviceKind::Lufs, DeviceKind::Meter]);
    }

    #[test]
    fn the_sweep_covers_the_ports_a_device_draws_from() {
        // #264, the DECIDED of 26.11: the patches' `random 10000` + `40000`.
        assert_eq!(
            sweep_ports(),
            wire::POLL_PORT_MIN..wire::POLL_PORT_MIN + wire::POLL_PORT_SPAN
        );
        assert_eq!(sweep_ports().len(), 10_000);
    }

    #[test]
    fn a_sweep_sends_each_port_the_meters_poll_once_and_counts_it_apart() {
        // A meter already running (a hub restarted under a running Live)
        // answers the sweep's poll and is learned; the sweep's sends count
        // apart from the rounds' polls. Port 0 stands for a refused send.
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        device
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let port = device.local_addr().unwrap().port();
        let poller = poll_socket().unwrap();
        let counters = Counters::default();
        let mut ports = vec![0, port];
        ports.sort_unstable();
        assert_eq!(sweep_over(&poller, ports, &counters), (1, 1));
        let mut buf = [0u8; 64];
        let (n, from) = device.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], wire::poll_datagram(DeviceKind::Meter).as_slice());
        assert_eq!(from, poller.local_addr().unwrap());
        let status = counters.status();
        assert_eq!((status.swept, status.sweep_failed), (1, 1));
        assert_eq!((status.polled, status.poll_failed), (0, 0));
        counters.swept(2, 3);
        let status = counters.status();
        assert_eq!((status.swept, status.sweep_failed), (3, 4));
    }

    #[test]
    fn a_send_reset_by_an_earlier_ports_answer_is_tried_once_more() {
        // Windows reports an earlier datagram's ICMP port unreachable on the
        // socket's next send (WSAECONNRESET): that send went nowhere, so it
        // is tried again; any other refusal is a failed send.
        assert!(send_again(io::ErrorKind::ConnectionReset));
        assert!(!send_again(io::ErrorKind::WouldBlock));
        assert!(!send_again(io::ErrorKind::ConnectionRefused));
        assert!(!send_again(io::ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn the_poll_socket_is_named_in_the_status() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let counters = Arc::new(Counters::default());
        let (events, _records) = EventLog::channel(16);
        let (_, task) = spawn(0, tx, Arc::clone(&counters), events, None, false).unwrap();
        let from = counters.status().poll_from.expect("the poll socket");
        assert!(from.starts_with("127.0.0.1:"), "{from}");
        assert_eq!(counters.status().swept, 0, "no sweep asked");
        task.abort();
    }

    #[tokio::test]
    async fn a_rounds_clock_keeps_its_grid_after_a_late_tick() {
        // #264's review: on Windows a timer wakes up to ~15.6 ms late; with
        // `Delay` every late tick would push the next one out (33 ms then
        // reads ~47 ms, about 21 polls a second). `Skip` keeps the grid
        // (30 a second on average) and still never sends a burst.
        for kind in POLLED_KINDS {
            let clock = round_clock(kind);
            assert_eq!(clock.period(), poll_every(kind));
            assert_eq!(
                clock.missed_tick_behavior(),
                tokio::time::MissedTickBehavior::Skip
            );
        }
    }

    #[test]
    fn a_clock_that_went_back_loses_no_key() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let lufs = key(DeviceKind::Lufs, 1);
        assert!(!lost(&lufs, t0 + ms(5000), t0));
        assert!(!lost(&lufs, t0, t0 + ms(999)));
        assert!(lost(&lufs, t0, t0 + ms(1000)));
        assert!(!lost(&key(DeviceKind::Meter, 1), t0, t0 + ms(499)));
        assert!(lost(&key(DeviceKind::Meter, 1), t0, t0 + ms(500)));
        assert_eq!(poll_addr(41_234).to_string(), "127.0.0.1:41234");
    }

    #[test]
    fn a_kept_datagram_teaches_its_port_and_a_full_table_teaches_none() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let counters = Counters::default();
        let (events, _records) = EventLog::channel(16);
        let mut bound = Bound::new();
        let mut polls = Polls::default();
        let t0 = Instant::now();
        assert!(handle_datagram(
            &polled_datagram(1, 41_001),
            t0,
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx
        ));
        assert_eq!(polls.due(t0, DeviceKind::Lufs), BTreeSet::from([41_001]));
        // Over its bound (dropped, still kept): the port stays known.
        for _ in 0..12 {
            handle_datagram(
                &polled_datagram(1, 41_001),
                t0,
                &mut bound,
                Some(&mut polls),
                &counters,
                &events,
                &tx,
            );
        }
        assert!(counters.status().dropped > 0);
        assert_eq!(polls.len(), 1);
        // Fill the key table, then a new key finds no room: no port.
        for device in 2..=64 {
            handle_datagram(
                &datagram(device),
                t0,
                &mut bound,
                Some(&mut polls),
                &counters,
                &events,
                &tx,
            );
        }
        handle_datagram(
            &polled_datagram(99, 41_099),
            t0,
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx,
        );
        assert_eq!(counters.status().full, 1);
        assert_eq!(polls.due(t0, DeviceKind::Lufs), BTreeSet::from([41_001]));
        // A link's datagram (no polls) teaches nothing and still goes on.
        assert!(handle_datagram(
            &polled_datagram(1, 41_002),
            t0 + Duration::from_secs(2),
            &mut bound,
            None,
            &counters,
            &events,
            &tx
        ));
    }

    #[test]
    fn a_round_polls_each_due_port_of_its_kind_once_and_counts_it() {
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        device
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let port = device.local_addr().unwrap().port();
        let meter = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        meter
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let meter_port = meter.local_addr().unwrap().port();
        let poller = poll_socket().unwrap();
        let counters = Counters::default();
        let mut polls = Polls::default();
        let t0 = Instant::now();
        polls.heard(&key(DeviceKind::Lufs, 1), Some(port), t0);
        polls.heard(&key(DeviceKind::Lufs, 2), Some(port), t0);
        polls.heard(&key(DeviceKind::Meter, 5), Some(meter_port), t0);
        poll_round(&poller, &mut polls, DeviceKind::Lufs, t0, &counters);
        let mut buf = [0u8; 64];
        let n = device.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], wire::poll_datagram(DeviceKind::Lufs).as_slice());
        let status = counters.status();
        assert_eq!(
            (status.polled, status.poll_failed, status.polling),
            (1, 0, 2),
            "one LUFS poll sent; both kinds' ports polled now"
        );
        // The meter's round: its own poll, to its own port only.
        poll_round(&poller, &mut polls, DeviceKind::Meter, t0, &counters);
        let n = meter.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], wire::poll_datagram(DeviceKind::Meter).as_slice());
        assert_eq!(counters.status().polled, 2);
        poll_round(
            &poller,
            &mut polls,
            DeviceKind::Lufs,
            t0 + Duration::from_millis(1000),
            &counters,
        );
        let status = counters.status();
        assert_eq!((status.polled, status.polling), (2, 0), "every key lost");
        // A send the socket refuses (port 0) is counted failed, the good
        // one sent: two ports polled.
        polls.heard(
            &key(DeviceKind::Lufs, 3),
            Some(port),
            t0 + Duration::from_secs(2),
        );
        polls.heard(
            &key(DeviceKind::Lufs, 4),
            Some(0),
            t0 + Duration::from_secs(2),
        );
        poll_round(
            &poller,
            &mut polls,
            DeviceKind::Lufs,
            t0 + Duration::from_secs(2),
            &counters,
        );
        let status = counters.status();
        assert_eq!(
            (status.polled, status.poll_failed, status.polling),
            (3, 1, 2)
        );
        assert_eq!(round_counts(3, 1), (2, 1));
        assert_eq!(round_counts(2, 0), (2, 0));
        assert_eq!(round_counts(0, 0), (0, 0));
        counters.polled(2, 3, 4);
        let status = counters.status();
        assert_eq!(
            (status.polled, status.poll_failed, status.polling),
            (5, 4, 4)
        );
    }
}
