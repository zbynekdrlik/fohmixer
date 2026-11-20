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
