//! The key writes paced by the transport (#190, dasio#22's reading of the
//! 2026-11-19 service: band Live stalled after a key write to all 12
//! Auto-Tunes in one batch while the set stood, the playhead jumping from
//! song to song, four such batches in 600 ms). Stopped, a song's key waits
//! until the playhead stayed in that song 500 ms (a newer song starts the
//! wait over), then goes one target per 40 ms, each its own call and
//! read-back, Live's bar last; a play start's keys start 300 ms after it;
//! playing, a key change stays one batch.

use std::time::Duration;

use super::*;

/// The F#m (song 3) writes of the band's two targets and of Live's bar:
/// the PA (Auto-Tune, its Key at 0.75 and Scale at 2/28), the stream (its
/// slots at 0), so every value is its step's centre.
fn f_sharp_m() -> [Vec<Value>; 3] {
    let [root, scale] = set_bar(6, "Minor");
    [
        vec![set_ref("k6", 6.0 / 11.0), set_ref("s6", 1.0 / 28.0)],
        vec![set_ref("k7", 6.0 / 11.0), set_ref("s7", 1.0 / 28.0)],
        vec![root, scale],
    ]
}

/// Both targets found with their values heard, the song map read and Live
/// heard stopped: nothing written (out of every song).
fn stopped_with_both(router: &mut Router, script: &mut Script) {
    router.handle(layout(1, vec![pa(), stream()]));
    find(router, PA, rack(6, ""));
    find(router, STREAM, rack(7, ""));
    heard(router, PA, (0.75, 2.0 / 28.0));
    heard(router, STREAM, (0.0, 0.0));
    map_read(router, 1, songs());
    transport(router, false);
    no_call(script);
}

/// `by` of Live's time passes on tokio's paused clock: every message the
/// hub's own timers sent meanwhile is handled (a walk's is dropped: no
/// test here answers a walk); the calls are left to the test.
async fn pass(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>, by: Duration) {
    tokio::time::advance(by).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    while let Ok(msg) = own.try_recv() {
        if !matches!(
            msg,
            RouterMsg::AutotuneFound { .. }
                | RouterMsg::AutotuneWalkFailed { .. }
                | RouterMsg::AutotuneWalkRetry { .. }
        ) {
            router.handle(msg);
        }
    }
}

const MS: Duration = Duration::from_millis(1);

#[tokio::test(start_paused = true)]
async fn a_stopped_run_of_jumps_writes_only_the_song_it_stops_in_one_target_per_40_ms() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    let _ = steps(&rx);
    // The playhead jumps through songs 1, 2 and 3 within 400 ms (AbleSet's
    // song list): nothing goes while it jumps.
    deliver(&mut router, TIME, 10.0);
    no_call(&mut script);
    pass(&mut router, &mut own, 200 * MS).await;
    deliver(&mut router, TIME, 130.0);
    no_call(&mut script);
    pass(&mut router, &mut own, 200 * MS).await;
    deliver(&mut router, TIME, 250.0);
    // Song 3 stays: its key goes 500 ms after the playhead came into it,
    // never a moment before.
    pass(&mut router, &mut own, 499 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    let [pa_write, stream_write, bar_write] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    // One target per 40 ms, each its own call and read-back.
    pass(&mut router, &mut own, 39 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        stream_write
    );
    // Live's bar last, alone.
    pass(&mut router, &mut own, 39 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, bar_write);
    pass(&mut router, &mut own, 2000 * MS).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("F#m", "song", &["Vox 2"], false),
            key_write("F#m", "song", &["#8"], false),
            key_write("F#m", "song", &[], true)
        ]
    );
    let settle = |song: u16, key: &str, act: &str| json!({"step": "key_settle", "instance": "band", "song": song, "key": key, "act": act});
    assert_eq!(
        of(&got, "key_settle"),
        vec![
            settle(1, "D", "waits"),
            settle(2, "D", "restarted"),
            settle(3, "F#m", "restarted"),
            settle(3, "F#m", "ends")
        ]
    );
    assert_eq!(of(&got, "readback").len(), 2);
    assert_eq!(of(&got, "bar"), vec![bar("F#m", "confirmed", Some("F#m"))]);
    assert_eq!(
        states(&mut router),
        vec![Some(KeyState::Confirmed), Some(KeyState::Confirmed)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_play_starts_keys_go_300_ms_after_it_one_target_per_40_ms() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    // Song 3 picked while stopped, and the play started 200 ms later,
    // within the settle: nothing on the start's own millisecond.
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 200 * MS).await;
    let _ = steps(&rx);
    transport(&mut router, true);
    no_call(&mut script);
    // The settle's own end (500 ms after the jump) writes nothing: the keys
    // go 300 ms after the start.
    pass(&mut router, &mut own, 299 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    let [pa_write, stream_write, bar_write] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    pass(&mut router, &mut own, 39 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        stream_write
    );
    pass(&mut router, &mut own, 40 * MS).await;
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, bar_write);
    pass(&mut router, &mut own, 2000 * MS).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_settle"),
        vec![
            json!({"step": "key_settle", "instance": "band", "song": 3, "key": "F#m",
                    "act": "after_start"})
        ]
    );
    assert_eq!(of(&got, "key_write").len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_jump_out_of_every_song_within_the_settle_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 100 * MS).await;
    // Out of every song: the waiting key no longer matches the song in
    // force, so it is dropped; the keys stay as they are.
    deliver(&mut router, TIME, 305.0);
    pass(&mut router, &mut own, 2000 * MS).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(of(&got, "key_write"), Vec::<Value>::new());
    assert_eq!(
        of(&got, "key_settle").last(),
        Some(
            &json!({"step": "key_settle", "instance": "band", "song": 3, "key": "F#m",
                     "act": "dropped"})
        )
    );
}

#[test]
fn a_spreads_target_leaves_the_bar_to_its_own_turn() {
    use super::super::{BarWrite, wait_of};
    assert!(BarWrite::Own.goes(false));
    assert!(BarWrite::Own.goes(true));
    assert!(!BarWrite::Owed.goes(false));
    assert!(BarWrite::Owed.goes(true));
    assert!(!BarWrite::Later.goes(false));
    assert!(!BarWrite::Later.goes(true), "a held write's bar waits too");
    assert_eq!(wait_of(500.0), 500 * MS);
    assert_eq!(wait_of(0.5), Duration::from_micros(500));
    assert_eq!(wait_of(-3.0), Duration::ZERO, "never below 0");
}

#[test]
fn a_target_written_its_key_since_the_pace_began_is_not_written_again() {
    use super::super::written_since;
    let f_sharp_m = Key {
        root: 6,
        minor: true,
    };
    let d = Key {
        root: 2,
        minor: false,
    };
    assert!(written_since(8, 7, Some(f_sharp_m), f_sharp_m));
    assert!(
        !written_since(7, 7, Some(f_sharp_m), f_sharp_m),
        "before it"
    );
    assert!(!written_since(6, 7, Some(f_sharp_m), f_sharp_m));
    assert!(!written_since(8, 7, Some(d), f_sharp_m), "another key");
    assert!(!written_since(8, 7, None, f_sharp_m));
}

#[tokio::test(start_paused = true)]
async fn a_target_found_and_written_within_the_settle_is_not_written_again() {
    // A reconnect: the song start waits while the walks answer, and each
    // target found then is written `found` at once; the spread passes it on.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 100 * MS).await;
    router.autotune_key_forget(PA);
    heard(&mut router, PA, (0.75, 2.0 / 28.0));
    let [pa_write, stream_write, bar_write] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    // The settle's end: the stream at once, the bar 40 ms on.
    pass(&mut router, &mut own, 400 * MS).await;
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        stream_write
    );
    pass(&mut router, &mut own, 40 * MS).await;
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, bar_write);
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![
            key_write("F#m", "found", &["Vox 2"], false),
            key_write("F#m", "song", &["#8"], false),
            key_write("F#m", "song", &[], true)
        ]
    );
}

/// A `key_settle` record of the band.
fn key_settle(song: u16, key: &str, act: &str) -> Value {
    json!({"step": "key_settle", "instance": "band", "song": song, "key": key, "act": act})
}

#[tokio::test(start_paused = true)]
async fn a_target_with_nothing_to_write_passes_its_turn_on_at_once() {
    // The PA found but its values not yet heard: owed, never written blind
    // (#140 PR 2b); its turn goes to the stream at once, no 40 ms lost.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6, ""));
    find(&mut router, STREAM, rack(7, ""));
    heard(&mut router, STREAM, (0.0, 0.0));
    map_read(&mut router, 1, songs());
    transport(&mut router, false);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 500 * MS).await;
    let [_, stream_write, bar_write] = f_sharp_m();
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        stream_write
    );
    pass(&mut router, &mut own, 39 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, bar_write);
    assert_eq!(router.autotune.targets[PA].key_wrote, None, "owed");
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![
            key_write("F#m", "song", &["#8"], false),
            key_write("F#m", "song", &[], true)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_turn_before_the_playhead_stayed_waits_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 100 * MS).await;
    // A turn of the settle's own generation 100 ms in (no timer of
    // tokio's fires early): it waits the 400 ms still to go.
    let generation = router.autotune.key_paces["band"].generation;
    router.autotune_key_turn("band", generation);
    no_call(&mut script);
    pass(&mut router, &mut own, 399 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    let [pa_write, ..] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    // A stale turn does nothing.
    router.autotune_key_turn("band", generation);
    no_call(&mut script);
    assert_eq!(
        of(&steps(&rx), "key_settle"),
        vec![key_settle(3, "F#m", "waits"), key_settle(3, "F#m", "ends")]
    );
}

#[tokio::test(start_paused = true)]
async fn a_play_start_within_a_spread_puts_its_rest_300_ms_after_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 500 * MS).await;
    let [pa_write, stream_write, bar_write] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    // 20 ms on, Live starts: the stream and the bar wait for 300 ms after
    // the start, not for the spread's next 40 ms.
    pass(&mut router, &mut own, 20 * MS).await;
    let _ = steps(&rx);
    transport(&mut router, true);
    pass(&mut router, &mut own, 299 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        stream_write
    );
    pass(&mut router, &mut own, 40 * MS).await;
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, bar_write);
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    assert_eq!(
        of(&steps(&rx), "key_settle"),
        vec![key_settle(3, "F#m", "after_start")]
    );
}

#[tokio::test(start_paused = true)]
async fn a_key_change_while_playing_on_is_one_batch_and_drops_the_starts_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    transport(&mut router, true);
    // 100 ms after the start a jump into song 3: spread from 300 ms.
    pass(&mut router, &mut own, 100 * MS).await;
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 199 * MS).await;
    no_call(&mut script);
    pass(&mut router, &mut own, MS).await;
    let [pa_write, ..] = f_sharp_m();
    assert_eq!(confirm(&mut router, &mut own, &mut script).await, pa_write);
    // 320 ms after the start a jump into song 1 (D): Live has played on,
    // one batch of both and the bar now, F#m's rest dropped.
    pass(&mut router, &mut own, 20 * MS).await;
    deliver(&mut router, TIME, 10.0);
    let write = confirm(&mut router, &mut own, &mut script).await;
    assert_eq!(write.len(), 6, "{write:?}");
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_settle"),
        vec![
            key_settle(3, "F#m", "after_start"),
            key_settle(3, "F#m", "dropped")
        ]
    );
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("F#m", "song", &["Vox 2"], false),
            key_write("D", "song", &["Vox 2", "#8"], true)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_hold_within_a_spread_leaves_its_rest_owed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 500 * MS).await;
    confirm(&mut router, &mut own, &mut script).await;
    // A track-list change holds the band: the next turn writes nothing;
    // the stream and the bar owe F#m (the release's walk writes it found).
    router.autotune_hold("band", "tracks");
    pass(&mut router, &mut own, 40 * MS).await;
    no_call(&mut script);
    assert!(router.autotune.targets[PA].key_wrote.is_some(), "written");
    assert_eq!(router.autotune.targets[STREAM].key_wrote, None);
    assert!(router.autotune.positions["band"].bar_owed);
    assert!(router.autotune.key_paces.is_empty());
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![key_write("F#m", "song", &["Vox 2"], false)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_held_set_owes_its_song_start_at_once_and_waits_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    router.autotune_hold("band", "tracks");
    let _ = steps(&rx);
    deliver(&mut router, TIME, 250.0);
    assert_eq!(router.autotune.targets[PA].key_wrote, None);
    assert!(router.autotune.positions["band"].bar_owed);
    assert!(router.autotune.key_paces.is_empty());
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    assert_eq!(of(&steps(&rx), "key_settle"), Vec::<Value>::new());
}

#[tokio::test(start_paused = true)]
async fn a_disconnect_within_the_settle_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    stopped_with_both(&mut router, &mut script);
    deliver(&mut router, TIME, 250.0);
    pass(&mut router, &mut own, 100 * MS).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    pass(&mut router, &mut own, 1000 * MS).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(of(&got, "key_write"), Vec::<Value>::new());
    assert_eq!(
        of(&got, "key_settle"),
        vec![
            key_settle(3, "F#m", "waits"),
            key_settle(3, "F#m", "offline")
        ]
    );
}
