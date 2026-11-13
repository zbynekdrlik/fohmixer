//! `script missing` (#184 PR D, spec §6.5): a Live update may replace the
//! folder the FohMixer script was copied into, and Live then runs without
//! it. While Live runs on this PC and a LOCAL instance (one with a port,
//! never a linked one) has been offline for [`SCRIPT_MISSING_AFTER`], the
//! hub says its script is missing: the router tells the pages
//! (`ServerMsg::Instance.script_missing`), `/api/status` shows it and the
//! event log gets one `script` record per change. The install copies the
//! script again.
//!
//! The decisions are pure ([`script_missing`], [`Watch`]); the task
//! ([`watch`]) reads each local instance's snapshot and the process probe
//! every [`CHECK_EVERY`]; the probe ([`live_running`]) lists the processes
//! on Windows and is `false` elsewhere (no Live runs beside a test hub).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::live::client::LiveHandle;
use crate::router::RouterMsg;

/// How long a local instance stays offline while Live runs before its
/// script counts as missing.
pub const SCRIPT_MISSING_AFTER: Duration =
    Duration::from_secs(fohmixer_proto::client::SCRIPT_MISSING_AFTER_S);

/// How often the watch looks.
pub const CHECK_EVERY: Duration = Duration::from_secs(5);

/// Whether a local instance's script is missing: Live runs and the instance
/// has been offline for at least [`SCRIPT_MISSING_AFTER`] (`offline_for`
/// none: online, or never seen offline).
pub fn script_missing(live_running: bool, offline_for: Option<Duration>) -> bool {
    live_running && offline_for.is_some_and(|d| d >= SCRIPT_MISSING_AFTER)
}

/// Whether a process's executable name is an Ableton Live (the band's or
/// the master's renamed copy, `Ableton Live 12 Suite.exe`; Live's indexer,
/// `Ableton Index.exe`, is not a Live). Case-insensitive, as the install's
/// `Test-FohLiveRunning`.
pub fn is_live_exe(exe: &str) -> bool {
    let lower = exe.to_ascii_lowercase();
    lower.starts_with("ableton live") || lower.starts_with("abletonlive")
}

/// The watch's state: since when each instance is offline, and what it
/// said last.
#[derive(Debug, Default)]
pub struct Watch {
    offline_since: BTreeMap<String, Instant>,
    missing: BTreeSet<String>,
}

impl Watch {
    /// One look at `instance` at `now`: online or offline, and whether Live
    /// runs. Returns the new verdict when it changed.
    pub fn look(
        &mut self,
        instance: &str,
        online: bool,
        live_running: bool,
        now: Instant,
    ) -> Option<bool> {
        let offline_for = if online {
            self.offline_since.remove(instance);
            None
        } else {
            let since = *self
                .offline_since
                .entry(instance.to_string())
                .or_insert(now);
            Some(now.saturating_duration_since(since))
        };
        let missing = script_missing(live_running, offline_for);
        let changed = if missing {
            self.missing.insert(instance.to_string())
        } else {
            self.missing.remove(instance)
        };
        changed.then_some(missing)
    }
}

/// The router's message for one change.
pub fn change_msg(instance: &str, missing: bool) -> RouterMsg {
    RouterMsg::ScriptMissing {
        instance: instance.to_string(),
        missing,
    }
}

/// The watch task: every `period` each LOCAL instance of `live` is looked
/// at with `probe` (asked once per look); each change goes to the router.
/// Ends when a change finds the router gone (the hub's stop aborts it).
pub async fn watch<P>(
    live: Vec<LiveHandle>,
    router: mpsc::UnboundedSender<RouterMsg>,
    probe: P,
    period: Duration,
) where
    P: Fn() -> bool + Send + 'static,
{
    let local: Vec<LiveHandle> = live.into_iter().filter(|h| !h.linked()).collect();
    let mut state = Watch::default();
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let now = Instant::now();
        let running = probe();
        for handle in &local {
            let online = handle.snapshot().online;
            if let Some(missing) = state.look(handle.name(), online, running, now)
                && router.send(change_msg(handle.name(), missing)).is_err()
            {
                return;
            }
        }
    }
}

/// Whether an Ableton Live runs on this PC (a ToolHelp snapshot of the
/// processes; a snapshot that cannot be taken says no).
#[cfg(windows)]
pub fn live_running() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: a snapshot handle we own and close; the entry's size is set
    // as the API asks.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more && !found {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            found = is_live_exe(&String::from_utf16_lossy(&entry.szExeFile[..len]));
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// No Live runs beside a hub on another platform (the tests, the E2E).
#[cfg(not(windows))]
pub fn live_running() -> bool {
    false
}

/// The log line of a change.
pub fn change_line(missing: bool) -> &'static str {
    if missing {
        "script missing: Live runs, but its FohMixer script has not answered for 60 s"
    } else {
        "script missing no longer"
    }
}

#[cfg(test)]
mod tests;
