use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::config::InstanceCfg;

const SECOND: Duration = Duration::from_secs(1);

#[test]
fn the_script_is_missing_only_while_live_runs_and_the_instance_was_offline_60_s() {
    assert_eq!(SCRIPT_MISSING_AFTER, Duration::from_secs(60));
    assert_eq!(CHECK_EVERY, Duration::from_secs(5));
    let just_short = SCRIPT_MISSING_AFTER - Duration::from_millis(1);
    assert!(!script_missing(true, Some(just_short)));
    assert!(script_missing(true, Some(SCRIPT_MISSING_AFTER)));
    assert!(script_missing(true, Some(Duration::from_secs(3600))));
    // Live not running: the Live is simply closed.
    assert!(!script_missing(false, Some(SCRIPT_MISSING_AFTER)));
    // Never offline (online, or not seen offline).
    assert!(!script_missing(true, None));
    assert!(!script_missing(false, None));
}

#[test]
fn a_live_is_any_ableton_live_exe_and_never_its_indexer() {
    assert!(is_live_exe("Ableton Live 12 Suite.exe"));
    assert!(is_live_exe("ableton live 12 suite.exe"));
    assert!(is_live_exe("AbletonLive12.exe"));
    assert!(!is_live_exe("Ableton Index.exe"));
    assert!(!is_live_exe("Live.exe"));
    assert!(!is_live_exe("my Ableton Live.exe"));
    assert!(!is_live_exe(""));
}

#[test]
fn the_watch_counts_only_while_offline_with_live_running_and_says_each_change_once() {
    let start = Instant::now();
    let at = |s: u64| start + Duration::from_secs(s);
    let mut watch = Watch::default();
    // Online: nothing.
    assert_eq!(watch.look("mbc", true, true, at(0)), None);
    // Offline from 5 s with Live running: still nothing at 64 s, missing at 65 s.
    assert_eq!(watch.look("mbc", false, true, at(5)), None);
    assert_eq!(watch.look("mbc", false, true, at(64)), None);
    assert_eq!(watch.look("mbc", false, true, at(65)), Some(true));
    // Said once.
    assert_eq!(watch.look("mbc", false, true, at(70)), None);
    // Live closed: no longer missing (a closed Live is `not running`).
    assert_eq!(watch.look("mbc", false, false, at(75)), Some(false));
    assert_eq!(watch.look("mbc", false, false, at(200)), None);
    // Live started again after a long outage: its own start counts from
    // its first look (Live answers about a minute after it starts), never
    // from the outage before it.
    assert_eq!(watch.look("mbc", false, true, at(205)), None);
    assert_eq!(watch.look("mbc", false, true, at(264)), None);
    assert_eq!(watch.look("mbc", false, true, at(265)), Some(true));
    // Online: back, and the count starts over at the next offline look.
    assert_eq!(watch.look("mbc", true, true, at(270)), Some(false));
    assert_eq!(watch.look("mbc", false, true, at(275)), None);
    assert_eq!(watch.look("mbc", false, true, at(334)), None);
    assert_eq!(watch.look("mbc", false, true, at(335)), Some(true));
}

#[test]
fn only_a_hub_with_one_local_live_is_watched() {
    assert!(!watched(0));
    assert!(watched(1));
    assert!(!watched(2));
}

#[test]
fn each_instance_has_its_own_count() {
    let start = Instant::now();
    let at = |s: u64| start + Duration::from_secs(s);
    let mut watch = Watch::default();
    assert_eq!(watch.look("band", false, true, at(0)), None);
    assert_eq!(watch.look("master", false, true, at(30)), None);
    assert_eq!(watch.look("band", false, true, at(60)), Some(true));
    assert_eq!(watch.look("master", false, true, at(60)), None);
    assert_eq!(watch.look("master", false, true, at(90)), Some(true));
    assert_eq!(watch.look("band", true, true, at(95)), Some(false));
    assert_eq!(watch.look("master", false, true, at(95)), None);
}

#[test]
fn the_change_lines() {
    assert_eq!(
        change_line(true),
        "script missing: Live runs, but its FohMixer script has not answered for 60 s"
    );
    assert_eq!(change_line(false), "script missing no longer");
}

#[test]
fn no_live_runs_beside_a_hub_off_windows() {
    #[cfg(not(windows))]
    assert!(!live_running());
}

fn offline(name: &str, link: Option<&str>) -> LiveHandle {
    // Nothing listens on the port: the instance stays offline.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let cfg = InstanceCfg {
        name: name.into(),
        port: if link.is_some() { 0 } else { port },
        link: link.map(str::to_string),
    };
    let events: crate::live::client::Events = Arc::new(|_, _| {});
    LiveHandle::spawn(&cfg, events).0
}

#[tokio::test(start_paused = true)]
async fn the_task_tells_the_router_of_a_local_instance_only_and_each_change() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let running = Arc::new(AtomicBool::new(true));
    let probe = {
        let running = Arc::clone(&running);
        move || running.load(Ordering::SeqCst)
    };
    let live = vec![
        offline("mbc", None),
        offline("far", Some("ws://10.0.0.5:39201")),
    ];
    let task = tokio::spawn(watch(live, tx, probe, CHECK_EVERY));
    // 55 s offline: nothing yet.
    tokio::time::sleep(Duration::from_secs(55) + SECOND).await;
    assert!(rx.try_recv().is_err());
    // 60 s: the local instance's script is missing; the linked one is
    // never watched.
    tokio::time::sleep(Duration::from_secs(5)).await;
    match rx.try_recv() {
        Ok(RouterMsg::ScriptMissing { instance, missing }) => {
            assert_eq!((instance.as_str(), missing), ("mbc", true));
        }
        _ => panic!("no verdict"),
    }
    assert!(rx.try_recv().is_err());
    // Live closed: the verdict goes back.
    running.store(false, Ordering::SeqCst);
    tokio::time::sleep(CHECK_EVERY).await;
    match rx.try_recv() {
        Ok(RouterMsg::ScriptMissing { instance, missing }) => {
            assert_eq!((instance.as_str(), missing), ("mbc", false));
        }
        _ => panic!("no verdict"),
    }
    // The router gone: the next change (60 s after Live runs again) ends
    // the task.
    drop(rx);
    running.store(true, Ordering::SeqCst);
    tokio::time::sleep(SCRIPT_MISSING_AFTER + CHECK_EVERY).await;
    tokio::time::timeout(SECOND, task)
        .await
        .expect("the task ends")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_hub_with_two_local_lives_is_not_watched() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let live = vec![offline("band", None), offline("master", None)];
    let task = tokio::spawn(watch(live, tx, || true, CHECK_EVERY));
    tokio::time::timeout(SECOND, task)
        .await
        .expect("the task ends at once")
        .unwrap();
    assert!(rx.try_recv().is_err());
}
