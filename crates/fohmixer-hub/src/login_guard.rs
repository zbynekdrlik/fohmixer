//! Login protection (from iemmixer's `iem-server/src/login_guard.rs` @
//! 22372bc, trimmed to fohmixer: one engineer PIN, LAN only — spec D7 — so
//! no member dimension and no tunnel origin). Pure admission logic with the
//! clock injected, plus a bounded gate for argon2id work:
//!
//! - budgets count failures only; a client is keyed by its address (an IPv6
//!   client by its /64, so it cannot rotate addresses to reset its budget);
//! - per client: three free failures, then 1, 2, 4 … s;
//! - per client: 20 failures in 10 minutes → 60 s spacing;
//! - over all clients: over 30 failures in an hour → 5 s spacing between
//!   admitted attempts;
//! - every delay ≤ 60 s — never a lockout; the caller answers 429 +
//!   `Retry-After` before any hashing.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Consecutive failures per client that carry no delay.
pub const FREE_FAILURES: u32 = 3;
/// A client's streak is forgotten after this long without a failure.
pub const STREAK_DECAY: Duration = Duration::from_secs(15 * 60);
/// Upper bound of every delay (never a lockout).
pub const MAX_DELAY: Duration = Duration::from_secs(60);
/// Sliding window of the per-client budget.
pub const CLIENT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Failures per client within [`CLIENT_WINDOW`] before spacing applies.
pub const CLIENT_WINDOW_FAILURES: usize = 20;
/// Spacing once a client has used its window budget.
pub const CLIENT_SPACING: Duration = Duration::from_secs(60);
/// Sliding window of the hub-wide budget.
pub const GLOBAL_WINDOW: Duration = Duration::from_secs(60 * 60);
/// Failures within [`GLOBAL_WINDOW`] before hub-wide spacing applies.
pub const GLOBAL_WINDOW_FAILURES: usize = 30;
/// Spacing between admitted attempts over the hub-wide budget.
pub const GLOBAL_SPACING: Duration = Duration::from_secs(5);
/// Upper bound of tracked clients (memory bound under a flood).
pub const MAX_TRACKED: usize = 4096;
/// argon2id hashes running at once (each costs 19 MiB).
pub const HASH_CONCURRENCY: usize = 2;
/// Requests allowed to wait for a hashing slot; more get 429 at once.
pub const HASH_QUEUE: usize = 8;

/// The budget key of a peer address: IPv4 as is, IPv6 reduced to its /64 (a
/// single device usually holds a whole /64).
pub fn client_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

/// What a recorded failure changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureEffect {
    Counted,
    /// This failure pushed the hub over its hourly budget.
    GlobalBudgetExhausted,
}

/// Counters (diagnostics).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LoginStats {
    pub failures: u64,
    pub global_budget_trips: u64,
}

#[derive(Debug, Clone, Copy)]
struct Streak {
    failures: u32,
    last_failure: Instant,
}

#[derive(Debug, Default)]
struct Inner {
    streaks: HashMap<IpAddr, Streak>,
    clients: HashMap<IpAddr, VecDeque<Instant>>,
    global: VecDeque<Instant>,
    last_admitted: Option<Instant>,
    stats: LoginStats,
}

/// Delay owed after `failures` consecutive failures: none up to
/// [`FREE_FAILURES`] − 1, then 1, 2, 4 … s, capped at [`MAX_DELAY`].
pub fn streak_delay(failures: u32) -> Duration {
    if failures < FREE_FAILURES {
        return Duration::ZERO;
    }
    let exponent = failures - FREE_FAILURES;
    if exponent >= 6 {
        return MAX_DELAY;
    }
    Duration::from_secs(1u64 << exponent).min(MAX_DELAY)
}

fn prune(window: &mut VecDeque<Instant>, now: Instant, span: Duration) {
    while let Some(&oldest) = window.front() {
        if now.saturating_duration_since(oldest) >= span {
            window.pop_front();
        } else {
            break;
        }
    }
}

fn evict_oldest_streak(map: &mut HashMap<IpAddr, Streak>) {
    if let Some(key) = map
        .iter()
        .min_by_key(|(_, s)| s.last_failure)
        .map(|(k, _)| *k)
    {
        map.remove(&key);
    }
}

fn evict_oldest_client(map: &mut HashMap<IpAddr, VecDeque<Instant>>) {
    if let Some(key) = map
        .iter()
        .min_by_key(|(_, w)| w.back().copied())
        .map(|(k, _)| *k)
    {
        map.remove(&key);
    }
}

/// Failure bookkeeping shared by every login.
#[derive(Debug, Default)]
pub struct LoginGuard {
    inner: Mutex<Inner>,
}

impl LoginGuard {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Admission before any hashing. `Err(wait)` → answer 429 with
    /// `Retry-After`.
    pub fn check(&self, peer: IpAddr, now: Instant) -> Result<(), Duration> {
        let client = client_key(peer);
        let mut inner = self.lock();
        if let Some(streak) = inner.streaks.get(&client).copied() {
            let since = now.saturating_duration_since(streak.last_failure);
            if since >= STREAK_DECAY {
                inner.streaks.remove(&client);
            } else {
                let delay = streak_delay(streak.failures);
                if since < delay {
                    return Err(delay - since);
                }
            }
        }
        if let Some(window) = inner.clients.get_mut(&client) {
            prune(window, now, CLIENT_WINDOW);
            if window.len() >= CLIENT_WINDOW_FAILURES
                && let Some(&last) = window.back()
            {
                let since = now.saturating_duration_since(last);
                if since < CLIENT_SPACING {
                    return Err(CLIENT_SPACING - since);
                }
            }
        }
        prune(&mut inner.global, now, GLOBAL_WINDOW);
        if inner.global.len() > GLOBAL_WINDOW_FAILURES
            && let Some(last) = inner.last_admitted
        {
            let since = now.saturating_duration_since(last);
            if since < GLOBAL_SPACING {
                return Err(GLOBAL_SPACING - since);
            }
        }
        inner.last_admitted = Some(now);
        Ok(())
    }

    /// Count a failed PIN check.
    pub fn record_failure(&self, peer: IpAddr, now: Instant) -> FailureEffect {
        let client = client_key(peer);
        let mut inner = self.lock();
        let failures = match inner.streaks.get(&client) {
            Some(streak) if now.saturating_duration_since(streak.last_failure) < STREAK_DECAY => {
                streak.failures.saturating_add(1)
            }
            _ => 1,
        };
        if !inner.streaks.contains_key(&client) && inner.streaks.len() >= MAX_TRACKED {
            evict_oldest_streak(&mut inner.streaks);
        }
        inner.streaks.insert(
            client,
            Streak {
                failures,
                last_failure: now,
            },
        );

        if !inner.clients.contains_key(&client) && inner.clients.len() >= MAX_TRACKED {
            evict_oldest_client(&mut inner.clients);
        }
        let window = inner.clients.entry(client).or_default();
        prune(window, now, CLIENT_WINDOW);
        window.push_back(now);
        if window.len() > CLIENT_WINDOW_FAILURES {
            window.pop_front();
        }

        inner.stats.failures += 1;
        prune(&mut inner.global, now, GLOBAL_WINDOW);
        let before = inner.global.len();
        inner.global.push_back(now);
        if inner.global.len() > GLOBAL_WINDOW_FAILURES + 1 {
            inner.global.pop_front();
        }
        if before == GLOBAL_WINDOW_FAILURES {
            inner.stats.global_budget_trips += 1;
            FailureEffect::GlobalBudgetExhausted
        } else {
            FailureEffect::Counted
        }
    }

    /// A successful PIN check ends that client's streak.
    pub fn record_success(&self, peer: IpAddr) {
        self.lock().streaks.remove(&client_key(peer));
    }

    pub fn stats(&self) -> LoginStats {
        self.lock().stats
    }
}

/// Bounded concurrency for argon2id work: `concurrency` running, at most
/// `max_waiting` queued; beyond that `acquire` returns `None` at once.
#[derive(Debug)]
pub struct HashGate {
    permits: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_waiting: usize,
}

struct WaitingSlot<'a>(&'a AtomicUsize);

impl Drop for WaitingSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl HashGate {
    pub fn new(concurrency: usize, max_waiting: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency)),
            waiting: AtomicUsize::new(0),
            max_waiting,
        }
    }

    /// A hashing slot, or `None` when the queue is full. A cancelled waiter
    /// (client gone) frees its queue place.
    pub async fn acquire(&self) -> Option<OwnedSemaphorePermit> {
        if let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() {
            return Some(permit);
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.max_waiting {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let _slot = WaitingSlot(&self.waiting);
        Arc::clone(&self.permits).acquire_owned().await.ok()
    }

    /// Requests currently queued for a slot.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn lan(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn streak_delay_doubles_from_the_third_failure_and_caps_at_sixty_seconds() {
        let table = [
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 1),
            (4, 2),
            (5, 4),
            (6, 8),
            (7, 16),
            (8, 32),
            (9, 60),
            (10, 60),
            (u32::MAX, 60),
        ];
        for (failures, expected) in table {
            assert_eq!(
                streak_delay(failures),
                secs(expected),
                "failures={failures}"
            );
        }
    }

    #[test]
    fn three_failures_then_backoff() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(1);
        for _ in 0..3 {
            assert_eq!(guard.check(c, t0), Ok(()));
            guard.record_failure(c, t0);
        }
        assert_eq!(guard.check(c, t0), Err(secs(1)));
        assert_eq!(
            guard.check(c, t0 + Duration::from_millis(400)),
            Err(Duration::from_millis(600))
        );
        assert_eq!(guard.check(c, t0 + secs(1)), Ok(()));
        guard.record_failure(c, t0 + secs(1));
        assert_eq!(guard.check(c, t0 + secs(1)), Err(secs(2)));
    }

    #[test]
    fn success_clears_the_streak() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            guard.record_failure(lan(1), t0);
        }
        guard.record_success(lan(1));
        assert_eq!(guard.check(lan(1), t0), Ok(()));
    }

    #[test]
    fn the_streak_is_per_client() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            guard.record_failure(lan(1), t0);
        }
        assert_eq!(guard.check(lan(2), t0), Ok(()));
    }

    #[test]
    fn the_streak_decays_after_fifteen_quiet_minutes() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for _ in 0..5 {
            guard.record_failure(lan(1), t0);
        }
        assert_eq!(guard.check(lan(1), t0 + STREAK_DECAY), Ok(()));
        guard.record_failure(lan(1), t0 + STREAK_DECAY);
        assert_eq!(guard.check(lan(1), t0 + STREAK_DECAY), Ok(()));
    }

    #[test]
    fn a_streak_survives_fourteen_quiet_minutes() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(1);
        for _ in 0..5 {
            guard.record_failure(c, t0);
        }
        let later = t0 + secs(14 * 60);
        guard.record_failure(c, later);
        assert_eq!(
            guard.check(c, later),
            Err(secs(8)),
            "the sixth failure of the streak owes 8 s"
        );
    }

    #[test]
    fn a_failure_after_fifteen_quiet_minutes_starts_a_new_streak() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(1);
        for _ in 0..5 {
            guard.record_failure(c, t0);
        }
        let later = t0 + STREAK_DECAY;
        guard.record_failure(c, later);
        assert_eq!(guard.check(c, later), Ok(()));
    }

    #[test]
    fn a_client_is_spaced_sixty_seconds_after_twenty_failures_in_ten_minutes() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(2);
        for i in 0..20u64 {
            // A success in between ends each streak: only the window counts.
            assert_eq!(guard.check(c, t0 + secs(i)), Ok(()), "attempt {i}");
            guard.record_failure(c, t0 + secs(i));
            guard.record_success(c);
        }
        let last = t0 + secs(19);
        assert_eq!(guard.check(c, last + secs(10)), Err(secs(50)));
        assert_eq!(guard.check(c, last + secs(60)), Ok(()));
    }

    #[test]
    fn old_failures_leave_the_client_window() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for _ in 0..20u64 {
            guard.record_failure(lan(3), t0);
        }
        guard.record_success(lan(3));
        assert!(guard.check(lan(3), t0 + secs(1)).is_err());
        assert_eq!(guard.check(lan(3), t0 + CLIENT_WINDOW), Ok(()));
    }

    #[test]
    fn the_client_budget_spans_ten_minutes() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(7);
        for _ in 0..19u64 {
            guard.record_failure(c, t0);
        }
        let later = t0 + secs(9 * 60);
        guard.record_failure(c, later);
        guard.record_success(c);
        assert_eq!(guard.check(c, later), Err(CLIENT_SPACING));
    }

    #[test]
    fn the_hub_is_spaced_after_thirty_failures_an_hour() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let mut effects = Vec::new();
        for i in 0..31u8 {
            assert_eq!(guard.check(lan(i), t0), Ok(()));
            effects.push(guard.record_failure(lan(i), t0));
        }
        assert_eq!(
            effects
                .iter()
                .filter(|e| **e == FailureEffect::GlobalBudgetExhausted)
                .count(),
            1
        );
        assert_eq!(effects[30], FailureEffect::GlobalBudgetExhausted);
        assert_eq!(guard.check(lan(200), t0), Err(GLOBAL_SPACING));
        assert_eq!(guard.check(lan(200), t0 + GLOBAL_SPACING), Ok(()));
        assert_eq!(
            guard.check(lan(201), t0 + GLOBAL_SPACING + secs(1)),
            Err(secs(4))
        );
        assert_eq!(
            guard.stats(),
            LoginStats {
                failures: 31,
                global_budget_trips: 1
            }
        );
    }

    #[test]
    fn the_hub_budget_counts_the_last_hour_only() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for i in 0..31u8 {
            guard.record_failure(lan(i), t0);
        }
        let within = t0 + secs(59 * 60);
        assert_eq!(guard.check(lan(200), within), Ok(()));
        assert_eq!(
            guard.check(lan(201), within),
            Err(GLOBAL_SPACING),
            "59 minutes later the hub is still over its budget"
        );
        let after = t0 + GLOBAL_WINDOW;
        assert_eq!(guard.check(lan(202), after), Ok(()));
        assert_eq!(
            guard.check(lan(203), after),
            Ok(()),
            "an hour after the failures the hub is no longer spaced"
        );
    }

    #[test]
    fn every_delay_is_at_most_sixty_seconds() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let c = lan(77);
        for n in 0..200u64 {
            let at = t0 + Duration::from_millis(n * 10);
            if let Err(wait) = guard.check(c, at) {
                assert!(wait <= MAX_DELAY, "wait {wait:?}");
            }
            guard.record_failure(c, at);
        }
    }

    #[test]
    fn a_new_client_evicts_nobody_while_there_is_room() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let a = lan(10);
        for i in 0..4 {
            guard.record_failure(a, t0 + Duration::from_millis(i));
        }
        guard.record_failure(lan(11), t0 + secs(1));
        let inner = guard.inner.lock().unwrap();
        assert!(inner.streaks.contains_key(&a), "A's streak stays");
        assert!(inner.clients.contains_key(&a), "A's window stays");
        assert_eq!(inner.streaks.len(), 2);
        assert_eq!(inner.clients.len(), 2);
        drop(inner);
        assert!(guard.check(a, t0 + secs(1)).is_err(), "A is still delayed");
    }

    #[test]
    fn a_known_client_evicts_nobody_when_the_table_is_full() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        let client = |i: u32| IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
        for i in 0..(MAX_TRACKED as u32) {
            guard.record_failure(client(i), t0 + Duration::from_millis(u64::from(i)));
        }
        let newest = client(MAX_TRACKED as u32 - 1);
        guard.record_failure(newest, t0 + secs(10));
        let inner = guard.inner.lock().unwrap();
        assert_eq!(inner.streaks.len(), MAX_TRACKED);
        assert_eq!(inner.clients.len(), MAX_TRACKED);
        assert!(inner.streaks.contains_key(&client(0)), "the oldest stays");
        assert!(inner.clients.contains_key(&client(0)), "the oldest stays");
    }

    #[test]
    fn tracked_clients_are_bounded() {
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for i in 0..(MAX_TRACKED as u32 + 10) {
            let c = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
            guard.record_failure(c, t0 + Duration::from_millis(u64::from(i)));
        }
        let inner = guard.inner.lock().unwrap();
        assert!(inner.streaks.len() <= MAX_TRACKED);
        assert!(inner.clients.len() <= MAX_TRACKED);
        assert!(inner.global.len() <= GLOBAL_WINDOW_FAILURES + 1);
        // The oldest were evicted, the newest kept.
        let newest = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + MAX_TRACKED as u32 + 9));
        assert!(inner.streaks.contains_key(&newest));
        assert!(
            !inner
                .streaks
                .contains_key(&IpAddr::V4(Ipv4Addr::from(0x0a00_0000)))
        );
        assert!(
            !inner
                .clients
                .contains_key(&IpAddr::V4(Ipv4Addr::from(0x0a00_0000)))
        );
    }

    #[test]
    fn ipv6_clients_share_a_budget_per_64_prefix() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb:cccc:dddd:eeee".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(client_key(a), client_key(b), "one /64 is one client");
        assert_ne!(client_key(a), client_key(other));
        assert_eq!(client_key(a), "2001:db8:1:2::".parse::<IpAddr>().unwrap());
        let v4: IpAddr = "203.0.113.8".parse().unwrap();
        assert_eq!(client_key(v4), v4, "IPv4 keys are unchanged");
        let mapped: IpAddr = "::ffff:10.0.0.5".parse().unwrap();
        assert_eq!(client_key(mapped), lan(5), "a mapped IPv4 is the IPv4");
        let guard = LoginGuard::new();
        let t0 = Instant::now();
        for _ in 0..3 {
            guard.record_failure(a, t0);
        }
        assert!(
            guard.check(b, t0).is_err(),
            "a rotated address inherits the streak"
        );
        assert_eq!(guard.check(other, t0), Ok(()));
    }

    #[tokio::test]
    async fn hash_gate_refuses_beyond_concurrency_plus_queue() {
        let gate = Arc::new(HashGate::new(2, 8));
        let first = gate.acquire().await.expect("first permit");
        let second = gate.acquire().await.expect("second permit");
        let mut waiters = Vec::new();
        for _ in 0..8 {
            let g = Arc::clone(&gate);
            waiters.push(tokio::spawn(async move { g.acquire().await.is_some() }));
        }
        for _ in 0..1000 {
            if gate.waiting() == 8 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(gate.waiting(), 8);
        assert!(
            gate.acquire().await.is_none(),
            "the 11th concurrent request is refused at once"
        );
        drop(first);
        drop(second);
        for waiter in waiters {
            assert!(waiter.await.unwrap());
        }
        assert_eq!(gate.waiting(), 0);
    }

    #[tokio::test]
    async fn a_gate_without_a_queue_refuses_at_once() {
        let gate = HashGate::new(0, 0);
        let refused = tokio::time::timeout(Duration::from_secs(5), gate.acquire()).await;
        assert!(matches!(refused, Ok(None)), "expected an immediate refusal");
        assert_eq!(gate.waiting(), 0);
    }

    #[tokio::test]
    async fn a_cancelled_waiter_frees_its_queue_place() {
        let gate = Arc::new(HashGate::new(1, 1));
        let _held = gate.acquire().await.expect("permit");
        let g = Arc::clone(&gate);
        let waiter = tokio::spawn(async move { g.acquire().await.is_some() });
        for _ in 0..1000 {
            if gate.waiting() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(gate.waiting(), 1);
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(gate.waiting(), 0);
    }
}
