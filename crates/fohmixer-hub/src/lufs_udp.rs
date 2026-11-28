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
//!
//! **The start's sweep (#264, the DECIDED of 26.11):** a device of either
//! kind sends only when polled (its load and Device On turning 1 aside;
//! the LUFS device since its 10 s re-read rides the poll), so a hub
//! restarted under a running Live would never learn its port. At its start
//! the hub sends each kind's poll once to every port of the devices' range
//! ([`sweep`], over `config::LufsCfg.sweep`) from the poll socket; every
//! running device answers its own and is learned, its `route` dropping the
//! other kind's; a foreign program in the range sees two unknown OSC
//! datagrams once. Its sends count apart (`swept`, `sweep_failed`). A device
//! has no announce of its own after its load: one whose key went stale (a
//! Live main thread stalled past the stale window) comes back by answering
//! the polls queued at its `udpreceive @defer 1` meanwhile, whose datagram
//! names its port again (`Polls::heard`).
//!
//! **A shared port (#264, the design comment 7228766032):** two devices
//! can draw one poll port, and on the FOH PC Windows gives its datagrams to
//! the first binder only, so the other one is deaf. `Polls` keeps each
//! key's port in the hub's one `poll_ports::PortTable` (shared with
//! Listen: the kinds share the range): when the port is found shared
//! (`poll_ports`' two triggers) and a LUFS or meter key hears there, the
//! listener sends the port one [`wire::move_datagram`] of its kind
//! ([`send_moves`], at most once a port every `MOVE_EVERY_MS`), one `lufs`
//! record `shared_port` (warn) with both keys, and polls the port on for
//! both keys' kinds for that window; the device that hears draws another
//! port, a `moved` record names it, and the deaf one answers on the old
//! port from then on.

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
use crate::poll_ports::{PortKey, PortTable, Shared};
use crate::router::RouterMsg;
use crate::rules::lufs::bound::{Admit, Bound, Dropping, FORGET_MS, Key};
use crate::rules::lufs::kind_word;

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
    swept: AtomicU64,
    sweep_failed: AtomicU64,
    refused: Mutex<BTreeMap<String, u64>>,
    addr: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
    poll_from: Mutex<Option<String>>,
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

    /// The start's sweep (#264): `sent` polls went, `failed` did not.
    pub fn swept(&self, sent: u64, failed: u64) {
        self.swept.fetch_add(sent, Ordering::Relaxed);
        self.sweep_failed.fetch_add(failed, Ordering::Relaxed);
    }

    fn set_poll_from(&self, from: Option<String>) {
        *self
            .poll_from
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = from;
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
            poll_from: self
                .poll_from
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            swept: self.swept.load(Ordering::Relaxed),
            sweep_failed: self.sweep_failed.load(Ordering::Relaxed),
        }
    }
}

/// The ports to poll (#264): each polled-form key's port and when the key
/// was last heard, in the hub's one `poll_ports::PortTable` (its own one
/// in a test). A key is kept only while the bound keeps it (at most
/// `bound::MAX_KEYS`), and is dropped once lost.
#[derive(Debug, Default)]
pub struct Polls {
    table: Arc<PortTable>,
}

/// Whether a table key is a LUFS or meter key of `kind`.
fn of_kind(key: &PortKey, kind: DeviceKind) -> bool {
    matches!(key, PortKey::Lufs(key) if key.kind == kind)
}

impl Polls {
    /// The polls of the hub's one table.
    pub fn with_table(table: Arc<PortTable>) -> Self {
        Self { table }
    }

    /// A reading the bound kept (taken or dropped, never `Full`): its key's
    /// port, heard at `now`; the port it named before when it moved. A
    /// pushed-form reading names none. A port found shared waits for
    /// [`send_moves`].
    pub fn heard(&mut self, key: &Key, port: Option<u16>, now: Instant) -> Option<u16> {
        self.table.heard(PortKey::Lufs(key.clone()), port?, now)
    }

    /// The shared ports found where a LUFS or meter key hears, taken.
    pub fn take_shared(&mut self) -> Vec<Shared<PortKey>> {
        self.table.take(PortKey::is_lufs)
    }

    /// The ports of `kind`'s keys to poll at `now`, each once, and the ports
    /// moved within the window for a key of `kind`: the lost keys of every
    /// kind are dropped first.
    pub fn due(&mut self, now: Instant, kind: DeviceKind) -> BTreeSet<u16> {
        self.table.due(now, |key| of_kind(key, kind))
    }

    /// The ports polled now for the LUFS and meter keys, each once.
    pub fn ports(&self) -> usize {
        self.table.ports_of(PortKey::is_lufs)
    }

    /// The keys polled.
    pub fn len(&self) -> usize {
        self.table.len_of(PortKey::is_lufs)
    }

    /// No key polled.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The `lufs` record of a shared port's move (#264): the key moved (it
/// answers there), the one that stays (a LUFS, meter or Listen device),
/// the move's address and whether the socket took it. Warn class.
pub fn shared_fields(shared: &Shared<PortKey>, sent: bool) -> Value {
    json!({
        "what": "shared_port",
        "port": shared.port,
        "moved": shared.hearing.fields(),
        "stays": shared.deaf.fields(),
        "move": shared.hearing.move_address(),
        "sent": sent,
        "warn": true,
    })
}

/// The `lufs` record of a key that names a new poll port (#264).
pub fn moved_fields(key: &Key, from: u16, to: u16) -> Value {
    json!({
        "what": "moved",
        "kind": kind_word(key.kind),
        "device": key.device,
        "parents": key.parents,
        "from": from,
        "to": to,
    })
}

/// Sends each shared port found its move ([`wire::move_datagram`] of the
/// hearing key's kind) from `poller`: one `shared_port` record and one warn
/// line each. How many moves it handled.
pub fn send_moves(poller: &std::net::UdpSocket, polls: &mut Polls, events: &EventLog) -> u64 {
    let shared = polls.take_shared();
    for share in &shared {
        let sent = refused_sends(poller, &share.hearing.move_datagram(), [share.port]) == 0;
        tracing::warn!(
            port = share.port,
            moved = ?share.hearing,
            stays = ?share.deaf,
            sent,
            "lufs: two devices drew one poll port: moved the one that hears"
        );
        events.record("lufs", shared_fields(share, sent));
    }
    u64::try_from(shared.len()).unwrap_or(u64::MAX)
}

/// How often the hub reads the OS's UDP table (Windows), after its first
/// read right after the start's sweep.
pub const OS_TABLE_EVERY: Duration = Duration::from_secs(30);

/// Every kind's move, address and datagram: a port the OS's table found
/// shared names no key, so its first binder's kind is unknown, and a
/// device's `route` drops the other kinds' moves.
pub fn every_move() -> Vec<(&'static str, Vec<u8>)> {
    Vec::new()
}

/// The `lufs` record of a port the OS's table found shared (#264): its
/// port, the processes that bind it, the moves' addresses and whether the
/// socket took them all. Warn class.
pub fn os_shared_fields(port: u16, pids: usize, sent: bool) -> Value {
    let _ = (port, pids, sent);
    json!({})
}

/// The OS's UDP table read (`rows`, port and pid; Windows,
/// `udp_table::rows`): each port of `range` two processes bind and no move
/// of it within its window gets every kind's move ([`every_move`]) from
/// `poller`, one `shared_port` record (`found: "os_table"`) and one warn
/// line, and is polled on for the window for every kind, so the deaf device
/// is learned. A read that failed is one warn line, nothing moved. How many
/// ports it moved.
pub fn os_moves(
    poller: &std::net::UdpSocket,
    polls: &mut Polls,
    rows: io::Result<Vec<(u16, u32)>>,
    range: &std::ops::Range<u16>,
    now: Instant,
    events: &EventLog,
) -> u64 {
    let _ = (poller, polls, rows, range, now, events);
    0
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
    sweep_ports: Option<std::ops::Range<u16>>,
) -> io::Result<(SocketAddr, JoinHandle<()>)> {
    let table = Arc::default();
    spawn_with(port, tx, counters, events, tap, sweep_ports, table)
}

/// [`spawn`] with the hub's one port table (#264: shared with Listen).
pub fn spawn_with(
    port: u16,
    tx: mpsc::UnboundedSender<RouterMsg>,
    counters: Arc<Counters>,
    events: EventLog,
    tap: Option<Tap>,
    sweep_ports: Option<std::ops::Range<u16>>,
    table: Arc<PortTable>,
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
    counters.set_poll_from(poller.local_addr().ok().map(|a| a.to_string()));
    if let Some(ports) = sweep_ports {
        sweep(&poller, ports, &counters);
    }
    let polls = Polls::with_table(table);
    let task = tokio::spawn(listen(socket, poller, tx, counters, events, tap, polls));
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
pub(crate) fn poll_socket() -> io::Result<std::net::UdpSocket> {
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
    let failed = refused_sends(poller, &poll, ports.iter().copied());
    let (sent, failed) = round_counts(ports.len(), failed);
    let polling = u64::try_from(polls.ports()).unwrap_or(u64::MAX);
    counters.polled(sent, failed, polling);
}

/// The start's sweep (#264, the DECIDED of 26.11): each kind's poll
/// ([`POLLED_KINDS`]) once to each of `ports` (the hub's:
/// `config::LufsCfg.sweep`, the devices' whole range), so a device running
/// from before the hub's start (a hub restarted under a running Live: both
/// kinds send only when polled) answers and is learned. Non-blocking: a
/// refused send is counted, never waited for. Its counts, sent and failed,
/// also on one info line.
pub fn sweep(
    poller: &std::net::UdpSocket,
    ports: impl IntoIterator<Item = u16>,
    counters: &Counters,
) -> (u64, u64) {
    let ports: Vec<u16> = ports.into_iter().collect();
    let (mut sent, mut failed) = (0, 0);
    for kind in POLLED_KINDS {
        let poll = wire::poll_datagram(kind);
        let refused = refused_sends(poller, &poll, ports.iter().copied());
        let (kind_sent, kind_failed) = round_counts(ports.len(), refused);
        sent += kind_sent;
        failed += kind_failed;
    }
    counters.swept(sent, failed);
    tracing::info!(
        sent,
        failed,
        "lufs: polled every device port once at start, each kind (a device running from before answers)"
    );
    (sent, failed)
}

/// Sends `poll` to each of `ports`: how many sends the socket refused.
pub(crate) fn refused_sends(
    poller: &std::net::UdpSocket,
    poll: &[u8],
    ports: impl IntoIterator<Item = u16>,
) -> u64 {
    let refused = ports
        .into_iter()
        .filter(|port| !send_poll(poller, poll, *port))
        .count();
    u64::try_from(refused).unwrap_or(u64::MAX)
}

/// One poll to `port`: whether it went. A send refused for an earlier
/// datagram's answer ([`send_again`]) is tried once more.
fn send_poll(poller: &std::net::UdpSocket, poll: &[u8], port: u16) -> bool {
    let first = poller.send_to(poll, poll_addr(port));
    let sent = match first {
        Err(e) if send_again(e.kind()) => poller.send_to(poll, poll_addr(port)),
        other => other,
    };
    if let Err(e) = &sent {
        tracing::debug!(port, error = %e, "a device poll could not be sent");
    }
    sent.is_ok()
}

/// Whether a send refused with `kind` is tried once more: Windows reports an
/// earlier datagram's ICMP port unreachable on the socket's next send
/// (WSAECONNRESET), and that send went nowhere (the sweep sends to every
/// closed port of the range).
pub fn send_again(kind: io::ErrorKind) -> bool {
    kind == io::ErrorKind::ConnectionReset
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
    grid_clock(poll_every(kind))
}

/// A clock of `every` on its grid: its first tick at once, a late tick
/// never moves the next one out, a missed tick is skipped (the Listen
/// polls' too, #264).
pub(crate) fn grid_clock(every: Duration) -> tokio::time::Interval {
    let mut clock = tokio::time::interval(every);
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
        && let (Some(from), Some(to)) = (polls.heard(&key, reading.port, now), reading.port)
    {
        tracing::info!(device = key.device, parents = ?key.parents, from, to, "lufs: a device names a new poll port");
        events.record("lufs", moved_fields(&key, from, to));
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
    mut polls: Polls,
) {
    // One byte over the limit, so a datagram too long reads as too long.
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut bound = Bound::new();
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
        send_moves(&poller, &mut polls, &events);
        if sweep_due(swept, now) {
            swept = now;
            if let Some(cutoff) = now.checked_sub(Duration::from_millis(FORGET_MS)) {
                bound.forget_before(cutoff);
            }
        }
    }
}

#[cfg(test)]
mod tests;
