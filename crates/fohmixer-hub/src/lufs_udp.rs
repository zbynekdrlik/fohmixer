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

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::client::LufsListenerStatus;
use fohmixer_proto::lufs::wire::{self, MAX_DATAGRAM};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::EventLog;
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
        }
    }
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
    let mut fields = json!({
        "what": if started { "dropping" } else { "under_bound" },
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
/// the counters' status).
pub fn spawn(
    port: u16,
    tx: mpsc::UnboundedSender<RouterMsg>,
    counters: Arc<Counters>,
    events: EventLog,
) -> io::Result<(SocketAddr, JoinHandle<()>)> {
    let wanted = bind_addr(port);
    let bound = std::net::UdpSocket::bind(wanted).and_then(|socket| {
        socket.set_nonblocking(true)?;
        tokio::net::UdpSocket::from_std(socket)
    });
    let socket = match bound {
        Ok(socket) => socket,
        Err(e) => {
            let error = e.to_string();
            tracing::error!(addr = %wanted, error = %error, "lufs: cannot listen for the LUFS devices; the hub runs on without them");
            events.record("lufs", listen_fields(&wanted.to_string(), Some(&error)));
            counters.set_addr(None, Some(error));
            return Err(e);
        }
    };
    let addr = socket.local_addr()?;
    tracing::info!(addr = %addr, "lufs: listening for the LUFS devices");
    events.record("lufs", listen_fields(&addr.to_string(), None));
    counters.set_addr(Some(addr.to_string()), None);
    let task = tokio::spawn(listen(socket, tx, counters, events));
    Ok((addr, task))
}

/// One datagram's way: parsed, bounded, counted, to the router when taken.
/// False once the router is gone.
pub fn handle_datagram(
    data: &[u8],
    now: Instant,
    bound: &mut Bound,
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

async fn listen(
    socket: tokio::net::UdpSocket,
    tx: mpsc::UnboundedSender<RouterMsg>,
    counters: Arc<Counters>,
    events: EventLog,
) {
    // One byte over the limit, so a datagram too long reads as too long.
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut bound = Bound::new();
    let mut swept = Instant::now();
    loop {
        let n = match socket.recv(&mut buf).await {
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
        let now = Instant::now();
        if !handle_datagram(&buf[..n], now, &mut bound, &counters, &events, &tx) {
            return;
        }
        if now.saturating_duration_since(swept) >= SWEEP {
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
                &counters,
                &events,
                &tx
            ));
        }
        assert!(handle_datagram(
            b"junk", t0, &mut bound, &counters, &events, &tx
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
                &counters,
                &events,
                &tx
            ),
            "the router is gone"
        );
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
}
