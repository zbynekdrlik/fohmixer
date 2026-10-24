use super::*;
use fohmixer_proto::songmap::{ClipName, Cue, build};

fn cue(name: &str, t: f64) -> Cue {
    Cue {
        name: name.to_string(),
        time: Some(t),
    }
}

fn clip(name: &str, t: f64) -> ClipName {
    ClipName {
        name: name.to_string(),
        start: Some(t),
    }
}

fn major(root: u8) -> Key {
    Key { root, minor: false }
}

/// A position out of every song.
const OUT: InForce = (None, None);

/// Song 031 in F from 22766 to its SONG END at 23501, its key clip to G at
/// 22897 (spec §2.1; invented title).
fn map_031() -> SongMap {
    build(
        &[
            cue("031 Test {F 95 4/4}", 22766.0),
            cue("SONG END", 23501.0),
        ],
        &[clip("Key {G}", 22897.0)],
    )
}

#[test]
fn auto_tune_artist_is_found_by_its_key_and_scale_names() {
    let names = ["Device On", "Input Type", "Key", "Scale", "Retune Speed"];
    assert_eq!(
        device_of(&names),
        Ok(KeyParams {
            device: KeyDevice::AutoTune,
            key: 2,
            scale: 3
        })
    );
    assert_eq!((KEY_NAME, SCALE_NAME), ("Key", "Scale"));
}

#[test]
fn audiogridder_is_found_by_its_first_two_slots() {
    let names = [
        "Device On",
        "0:(unused):(unassigned)",
        "1:(unused):(unassigned)",
        "2:(unused):(unassigned)",
    ];
    assert_eq!(
        device_of(&names),
        Ok(KeyParams {
            device: KeyDevice::Gridder,
            key: 1,
            scale: 2
        })
    );
    assert_eq!((GRIDDER_KEY, GRIDDER_SCALE), (1, 2));
}

#[test]
fn a_device_without_key_parameters_is_missing() {
    assert_eq!(
        device_of(&["Device On", "Mix"]),
        Err(KeyMissing::NoKeyParams)
    );
    assert_eq!(
        device_of(&["Device On", "Key", "Key", "Scale"]),
        Err(KeyMissing::NoKeyParams)
    );
    assert_eq!(
        device_of(&[
            "Device On",
            "1:(unused):(unassigned)",
            "0:(unused):(unassigned)"
        ]),
        Err(KeyMissing::NoKeyParams)
    );
    // Both slots are needed, each where AudioGridder has it.
    assert_eq!(
        device_of(&["Device On", "0:(unused):(unassigned)", "Mix"]),
        Err(KeyMissing::NoKeyParams)
    );
    assert_eq!(
        device_of(&["Device On", "Mix", "1:(unused):(unassigned)"]),
        Err(KeyMissing::NoKeyParams)
    );
    assert_eq!(
        device_of(&["0:(unused):(unassigned)", "1:(unused):(unassigned)"]),
        Err(KeyMissing::NoKeyParams),
        "slot 0 must be parameter 1"
    );
    assert_eq!(device_of(&[]), Err(KeyMissing::NoKeyParams));
    // One Key without a Scale (or the other way) is no Auto-Tune Artist.
    assert_eq!(
        device_of(&["Device On", "Key", "Mix"]),
        Err(KeyMissing::NoKeyParams)
    );
    assert_eq!(
        device_of(&["Device On", "Mix", "Scale"]),
        Err(KeyMissing::NoKeyParams)
    );
}

#[test]
fn auto_tune_wins_when_both_match() {
    let names = ["Device On", "0:a", "1:b", "Key", "Scale"];
    assert_eq!(
        device_of(&names),
        Ok(KeyParams {
            device: KeyDevice::AutoTune,
            key: 3,
            scale: 4
        })
    );
}

#[test]
fn a_missing_key_device_says_why_in_words() {
    assert_eq!(
        KeyMissing::NoChain.to_string(),
        "the Latencies rack has no chain"
    );
    assert_eq!(
        KeyMissing::Plugins(2).to_string(),
        "chain 0 holds 2 plug-ins, not one"
    );
    assert_eq!(
        KeyMissing::Plugins(0).to_string(),
        "chain 0 holds 0 plug-ins, not one"
    );
    assert_eq!(
        KeyMissing::NoKeyParams.to_string(),
        "the plug-in has no Key/Scale parameters"
    );
}

#[test]
fn a_key_device_is_named_as_the_status_names_it() {
    assert_eq!(KeyDevice::AutoTune.name(), "auto_tune");
    assert_eq!(KeyDevice::Gridder.name(), "gridder");
    assert_eq!(
        serde_json::to_value([KeyDevice::AutoTune, KeyDevice::Gridder]).unwrap(),
        serde_json::json!(["auto_tune", "gridder"])
    );
}

#[test]
fn the_shown_key_reads_the_steps_half_up() {
    // The spike's slot 0 at 100/127 showed KEY A, Major.
    assert_eq!(
        shows(0.787401556968689, 0.007874015718698502),
        Some(Key {
            root: 9,
            minor: false
        })
    );
    assert_eq!(
        shows(10.0 / 11.0, 1.0 / 28.0),
        Some(Key {
            root: 10,
            minor: true
        })
    );
    assert_eq!(shows(0.5, 0.0), Some(major(6)), "5.5 rounds up");
    assert_eq!(
        shows(0.0, 2.0 / 28.0),
        None,
        "Chromatic, Auto-Tune's default"
    );
    assert_eq!(shows(1.0, 1.0), None, "the last scale step");
    assert_eq!(shows(1.0, 0.0), Some(major(11)));
    // A float32 echo of a step's centre reads that step.
    assert_eq!(shows(0.1818181872367859, 0.0), Some(major(2)));
}

#[test]
fn no_tempo_means_no_lookahead() {
    assert_eq!((LOOKAHEAD_MS, MAX_LOOKAHEAD_BEATS), (35.0, 1.0));
    assert_eq!(lookahead_beats(None), 0.0);
    assert_eq!(lookahead_beats(Some(0.0)), 0.0);
    assert_eq!(lookahead_beats(Some(f64::NAN)), 0.0);
    assert_eq!(lookahead_beats(Some(f64::INFINITY)), 0.0);
    assert_eq!(lookahead_beats(Some(-60.0)), 0.0);
    assert!((lookahead_beats(Some(120.0)) - 0.07).abs() < 1e-12);
    assert!((lookahead_beats(Some(60.0)) - 0.035).abs() < 1e-12);
    assert_eq!(lookahead_beats(Some(5000.0)), 1.0);
}

#[test]
fn a_key_clip_is_in_force_one_tick_early_only_while_playing() {
    // 031 → G at 22897 (spec §2.1), 120 bpm: 0.07 beats ahead.
    let m = map_031();
    assert_eq!(
        in_force(&m, 22896.94, true, Some(120.0)),
        (Some(31), Some(major(7)))
    );
    assert_eq!(
        in_force(&m, 22896.92, true, Some(120.0)),
        (Some(31), Some(major(5)))
    );
    assert_eq!(
        in_force(&m, 22896.94, false, Some(120.0)),
        (Some(31), Some(major(5)))
    );
    // No tempo heard yet: no lookahead while playing either.
    assert_eq!(
        in_force(&m, 22896.94, true, None),
        (Some(31), Some(major(5)))
    );
    assert_eq!(
        in_force(&m, 22897.0, false, None),
        (Some(31), Some(major(7)))
    );
    // Out of the song.
    assert_eq!(in_force(&m, 23501.0, false, None), (None, None));
    assert_eq!(in_force(&m, 22765.0, false, None), (None, None));
}

#[test]
fn the_key_in_force_uses_the_cue_time_as_live_reports_it() {
    let m = build(
        &[
            cue("009 Test {D 100}", 10960.000052291458),
            cue("SONG END", 11200.000052291458),
        ],
        &[],
    );
    assert_eq!(
        in_force(&m, 10960.000052291458, false, None),
        (Some(9), Some(major(2)))
    );
    assert_eq!(in_force(&m, 10960.0, false, None), (None, None));
}

#[test]
fn a_keyless_song_is_a_song_without_a_key_in_force() {
    // PR 2a review M2: song 088 has no key (spec §2.1); it is still a song.
    let m = build(
        &[
            cue("087 Test {E 100}", 100.0),
            cue("SONG END", 200.0),
            cue("088 Test", 300.0),
            cue("SONG END", 400.0),
        ],
        &[],
    );
    assert_eq!(in_force(&m, 350.0, false, None), (Some(88), None));
    assert_eq!(in_force(&m, 250.0, false, None), OUT);
    assert_eq!(in_force(&m, 150.0, false, None), (Some(87), Some(major(4))));
    // The lookahead moves the song too: 299.95 at 120 bpm is in 088.
    assert_eq!(in_force(&m, 299.95, true, Some(120.0)), (Some(88), None));
    assert_eq!(in_force(&m, 299.95, false, Some(120.0)), OUT);
}

#[test]
fn change_time_names_the_clip_start_in_force() {
    let m = map_031();
    assert_eq!(change_time(&m, 31, 22897.0), Some(22897.0));
    assert_eq!(change_time(&m, 31, 23000.0), Some(22897.0));
    assert_eq!(change_time(&m, 31, 22896.9), None);
    assert_eq!(change_time(&m, 32, 22897.0), None);
    // Two changes: the later one once passed.
    let two = build(
        &[cue("133 Test {G 100}", 100.0), cue("SONG END", 300.0)],
        &[clip("Key {G#}", 136.5), clip("Key {A}", 169.5)],
    );
    assert_eq!(change_time(&two, 133, 150.0), Some(136.5));
    assert_eq!(change_time(&two, 133, 169.5), Some(169.5));
    assert_eq!(change_time(&two, 133, 200.0), Some(169.5));
}

#[test]
fn every_why_has_its_word() {
    let words = [
        (KeyWhy::Song, "song"),
        (KeyWhy::Change, "change"),
        (KeyWhy::NoKey, "no_key"),
        (KeyWhy::Span, "span"),
        (KeyWhy::Found, "found"),
        (KeyWhy::Retry, "retry"),
    ];
    for (why, word) in words {
        assert_eq!(why.word(), word);
        assert_eq!(serde_json::to_value(why).unwrap(), serde_json::json!(word));
    }
}

#[test]
fn the_follower_starts_a_song_once_then_follows_its_changes() {
    let (f, g) = (major(5), major(7));
    let mut k = KeyFollow::new();
    assert_eq!(k.now(), OUT);
    assert_eq!(k.on_position(OUT), None, "nothing in force, nothing left");
    assert_eq!(k.on_position((Some(31), Some(f))), Some(KeyWhy::Song));
    assert_eq!(k.now(), (Some(31), Some(f)));
    assert_eq!(k.on_position((Some(31), Some(f))), None);
    assert_eq!(k.on_position((Some(31), Some(g))), Some(KeyWhy::Change));
    assert_eq!(k.now(), (Some(31), Some(g)));
    assert_eq!(k.on_position(OUT), Some(KeyWhy::Span));
    assert_eq!(k.now(), OUT);
    assert_eq!(k.on_position(OUT), None);
    assert_eq!(k.on_position((Some(32), Some(g))), Some(KeyWhy::Song)); // same key, new song
}

#[test]
fn a_new_song_in_the_same_key_is_a_song_start() {
    let f = major(5);
    let mut k = KeyFollow::new();
    k.on_position((Some(31), Some(f)));
    assert_eq!(k.on_position((Some(32), Some(f))), Some(KeyWhy::Song));
    assert_eq!(k.now(), (Some(32), Some(f)));
}

#[test]
fn a_jump_back_before_a_change_is_a_change_not_a_song_start() {
    let (f, g) = (major(5), major(7));
    let mut k = KeyFollow::new();
    k.on_position((Some(31), Some(g)));
    assert_eq!(k.on_position((Some(31), Some(f))), Some(KeyWhy::Change));
}

#[test]
fn leaving_a_song_and_coming_back_is_a_start_again() {
    let f = major(5);
    let mut k = KeyFollow::new();
    k.on_position((Some(31), Some(f)));
    assert_eq!(k.on_position(OUT), Some(KeyWhy::Span));
    assert_eq!(k.on_position((Some(31), Some(f))), Some(KeyWhy::Song));
}

#[test]
fn a_keyless_song_is_told_apart_from_out_of_every_song() {
    let e = major(4);
    let mut k = KeyFollow::new();
    // From out of every song into a keyless song: no key in force, its
    // song known.
    assert_eq!(k.on_position((Some(88), None)), Some(KeyWhy::NoKey));
    assert_eq!(k.now(), (Some(88), None));
    assert_eq!(k.on_position((Some(88), None)), None, "staying in it");
    // Out of it: out of every song.
    assert_eq!(k.on_position(OUT), Some(KeyWhy::Span));
    assert_eq!(k.on_position(OUT), None);
    // From a keyed song into a keyless one: a song start with no key.
    k.on_position((Some(87), Some(e)));
    assert_eq!(k.on_position((Some(88), None)), Some(KeyWhy::Song));
    // And back into the keyed one: a song start in its key.
    assert_eq!(k.on_position((Some(87), Some(e))), Some(KeyWhy::Song));
    // Out of every song into a keyed one: a song start.
    k.on_position(OUT);
    assert_eq!(k.on_position((Some(87), Some(e))), Some(KeyWhy::Song));
}

#[test]
fn a_reset_makes_the_same_song_a_start_again() {
    let f = major(5);
    let mut k = KeyFollow::new();
    k.on_position((Some(31), Some(f)));
    k.reset();
    assert_eq!(k.now(), OUT);
    assert_eq!(k.on_position((Some(31), Some(f))), Some(KeyWhy::Song));
}
