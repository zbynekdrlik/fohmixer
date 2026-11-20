use std::collections::BTreeMap;

use super::*;

const TARGET: Option<i8> = Some(-25);

#[test]
fn judge_on_every_boundary() {
    // ±1.0 LU around −25, both ends inside (the owner's trueLevel band).
    let cases = [
        (-250, LufsState::In),
        (-240, LufsState::In),
        (-239, LufsState::Off),
        (-260, LufsState::In),
        (-261, LufsState::Off),
        (-180, LufsState::Off),
        (-320, LufsState::Off),
        (-699, LufsState::Off),
        (-700, LufsState::Silent),
        (-701, LufsState::Silent),
        (i16::MIN, LufsState::Silent),
        (0, LufsState::Off),
        (i16::MAX, LufsState::Off),
    ];
    for (tenths, state) in cases {
        assert_eq!(judge(Some(tenths), TARGET), state, "{tenths}");
    }
}

#[test]
fn the_band_is_ten_tenths_each_side_of_any_target() {
    assert!(in_band(-150, -14));
    assert!(in_band(-130, -14));
    assert!(!in_band(-151, -14));
    assert!(!in_band(-129, -14));
    assert!(in_band(10, 0));
    assert!(in_band(-10, 0));
    assert!(!in_band(11, 0));
    assert!(!in_band(-11, 0));
    assert!(in_band(-1280, i8::MIN));
    assert!(!in_band(i16::MIN, i8::MIN));
    assert!(!in_band(i16::MAX, i8::MAX));
    assert!(in_band(1270, i8::MAX));
}

#[test]
fn text_has_one_decimal_and_a_typographic_minus() {
    for (tenths, shows) in [
        (-259, "\u{2212}25.9"),
        (-250, "\u{2212}25.0"),
        (-5, "\u{2212}0.5"),
        (-1, "\u{2212}0.1"),
        (0, "0.0"),
        (1, "0.1"),
        (15, "1.5"),
        (-699, "\u{2212}69.9"),
        (-123, "\u{2212}12.3"),
        (i16::MAX, "3276.7"),
    ] {
        assert_eq!(text(tenths), shows, "{tenths}");
    }
}

#[test]
fn silence_reads_minus_infinity() {
    for tenths in [-700, -701, -1000, i16::MIN] {
        assert_eq!(text(tenths), "\u{2212}\u{221e}", "{tenths}");
    }
}

#[test]
fn an_output_point_is_plain() {
    assert_eq!(judge(Some(-180), None), LufsState::Plain);
    assert_eq!(judge(Some(-699), None), LufsState::Plain);
    assert_eq!(judge(Some(0), None), LufsState::Plain);
    assert_eq!(judge(Some(-700), None), LufsState::Silent);
}

#[test]
fn no_value_is_none() {
    assert_eq!(judge(None, TARGET), LufsState::None);
    assert_eq!(judge(None, None), LufsState::None);
}

#[test]
fn captions_name_the_kind() {
    assert_eq!(caption(LufsKind::In), ["LUFS", "IN"]);
    assert_eq!(caption(LufsKind::Out), ["LUFS", "OUT"]);
}

#[test]
fn the_states_words() {
    for (state, word) in [
        (LufsState::In, "in"),
        (LufsState::Off, "off"),
        (LufsState::Plain, "plain"),
        (LufsState::Silent, "silent"),
        (LufsState::None, "none"),
    ] {
        assert_eq!(state.as_str(), word);
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!(word)
        );
        assert_eq!(
            serde_json::from_value::<LufsState>(serde_json::json!(word)).unwrap(),
            state
        );
    }
}

#[test]
fn the_owners_constants() {
    assert_eq!((BAND_TENTHS, SILENT_TENTHS), (10, -700));
}

#[test]
fn the_problems_words_and_wire_names() {
    for (problem, text, wire) in [
        (LufsProblem::NoLufsMeter, "no LUFS meter", "no_lufs_meter"),
        (
            LufsProblem::TwoLufsMeters,
            "two LUFS meters",
            "two_lufs_meters",
        ),
        (
            LufsProblem::NoMeterDevice,
            "no meter device",
            "no_meter_device",
        ),
    ] {
        assert_eq!(problem.text(), text);
        assert_eq!(
            serde_json::to_value(problem).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<LufsProblem>(serde_json::json!(wire)).unwrap(),
            problem
        );
    }
}

#[test]
fn the_history_answers_read_as_the_hub_writes_them() {
    let series = HistorySeries {
        from: 1_794_736_800_000,
        step_ms: 2000,
        values: vec![Some(-230), None, Some(-700)],
        fader: Vec::new(),
        play: Vec::new(),
        song: Vec::new(),
        songs: BTreeMap::new(),
    };
    let text = serde_json::to_string(&series).unwrap();
    assert_eq!(
        text,
        r#"{"from":1794736800000,"step_ms":2000,"values":[-230,null,-700]}"#
    );
    assert_eq!(
        serde_json::from_str::<HistorySeries>(&text).unwrap(),
        series
    );
    let days = HistoryDays {
        days: vec![HistoryDay {
            first_ms: 10,
            last_ms: 20,
        }],
    };
    let text = serde_json::to_string(&days).unwrap();
    assert_eq!(text, r#"{"days":[{"first_ms":10,"last_ms":20}]}"#);
    assert_eq!(serde_json::from_str::<HistoryDays>(&text).unwrap(), days);
}

#[test]
fn a_series_carries_the_fader_beside_the_readings_and_an_older_hubs_reads_as_none() {
    let series = HistorySeries {
        from: 1_794_736_800_000,
        step_ms: 2000,
        values: vec![Some(-230), None],
        fader: vec![Some(-45), None],
        play: Vec::new(),
        song: Vec::new(),
        songs: BTreeMap::new(),
    };
    let text = serde_json::to_string(&series).unwrap();
    assert_eq!(
        text,
        r#"{"from":1794736800000,"step_ms":2000,"values":[-230,null],"fader":[-45,null]}"#
    );
    assert_eq!(
        serde_json::from_str::<HistorySeries>(&text).unwrap(),
        series
    );
    // An older hub's answer has no `fader`.
    let old: HistorySeries =
        serde_json::from_str(r#"{"from":5,"step_ms":1000,"values":[-230]}"#).unwrap();
    assert_eq!(old.fader, Vec::<Option<i16>>::new());
    assert_eq!(old.values, vec![Some(-230)]);
}

#[test]
fn a_faders_volume_reads_as_db_tenths_minus_infinity_at_the_floor() {
    assert_eq!(FADER_FLOOR_TENTHS, -700);
    assert_eq!(FADER_TOP_TENTHS, 60);
    assert_eq!(fader_tenths(None), None);
    // 0 dB at Live's 0.85, +6 dB at 1.0, -10 dB at 0.6.
    assert_eq!(fader_tenths(Some(0.85)), Some(0));
    assert_eq!(fader_tenths(Some(1.0)), Some(60));
    assert_eq!(fader_tenths(Some(0.6)), Some(-100));
    // Rounded to the nearest tenth: 0.84 is -0.4 dB, 0.8425 is -0.3 dB.
    assert_eq!(fader_tenths(Some(0.84)), Some(-4));
    assert_eq!(fader_tenths(Some(0.8425)), Some(-3));
    // -inf at 0 and at the floor.
    assert_eq!(fader_tenths(Some(0.0)), Some(-700));
    // 0.97 is +4.8 dB.
    assert_eq!(fader_tenths(Some(0.97)), Some(48));
    // Just above the floor: -69.9 dB stays a number.
    let near = crate::volume::volume_of(-69.9);
    assert_eq!(fader_tenths(Some(near)), Some(-699));
}

#[test]
fn a_median_is_the_upper_middle_and_none_of_nothing() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[-40]), Some(-40));
    assert_eq!(median(&[-10, -40]), Some(-10));
    assert_eq!(median(&[-10, -40, -20]), Some(-20));
    assert_eq!(median(&[5, -10, -40, -20]), Some(-10));
}

#[test]
fn a_bucket_is_silence_when_half_or_more_are_silent() {
    assert!(mostly_silent(1, 2));
    assert!(!mostly_silent(1, 3));
    assert!(mostly_silent(2, 3));
    assert!(mostly_silent(0, 0));
    assert!(!mostly_silent(0, 1));
}

#[test]
fn a_bucket_is_the_gated_loudness_above_silence() {
    assert_eq!(bucket_of(&[]), None);
    assert_eq!(bucket_of(&[-700]), Some(SILENT_TENTHS));
    assert_eq!(bucket_of(&[-701, -200]), Some(SILENT_TENTHS));
    // Exactly −70 is silence too: half of two.
    assert_eq!(bucket_of(&[-700, -200]), Some(SILENT_TENTHS));
    // One silent of three: the gated loudness of the others.
    assert_eq!(bucket_of(&[-700, -200, -210]), Some(-205));
    assert_eq!(bucket_of(&[-250, -200, -210]), Some(-215));
    assert_eq!(bucket_of(&[-699]), Some(-699));
    assert_eq!(bucket_of(&[-200, -250]), Some(-218));
    assert_eq!(bucket_of(&[-200, -240, -220, -210]), Some(-215));
    // A breath more than 10 LU under the sung level is gated out.
    assert_eq!(bucket_of(&[-200, -200, -200, -340]), Some(-200));
}

#[test]
fn an_energy_mean_weighs_the_loud_readings_as_loudness_does() {
    assert_eq!(energy_mean(&[]), None);
    let one = energy_mean(&[-230]).unwrap();
    assert!((one + 230.0).abs() < 1e-9, "{one}");
    // −20 and −30 LU: 10·log10((0.01 + 0.001) / 2) = −22.596 LU.
    let two = energy_mean(&[-200, -300]).unwrap();
    assert!((two + 225.96).abs() < 0.01, "{two}");
    assert_eq!(GATE_TENTHS, 100);
}

#[test]
fn the_gate_leaves_out_what_lies_more_than_10_lu_under_the_mean() {
    // Sung at −20, one breath at −34: gated out, −20.0.
    assert_eq!(gated(&[-200, -200, -200, -340]), Some(-200));
    // The mean before the gate: 10·log10((3·0.01 + 0.00078) / 4) = −21.14,
    // −21.1 rounded; a reading at −31.1, exactly 10 LU under it, stays.
    assert_eq!(gated(&[-200, -200, -200, -311]), Some(-211));
    // One a tenth further under it is gated out.
    assert_eq!(gated(&[-200, -200, -200, -312]), Some(-200));
    // A quiet song stays quiet.
    assert_eq!(gated(&[-300, -300]), Some(-300));
    // Two loud and two soft 8 LU apart: no gate, the energy mean
    // (10·log10((2·0.01 + 2·0.00158) / 4) = −22.37).
    assert_eq!(gated(&[-200, -200, -280, -280]), Some(-224));
    assert_eq!(gated(&[]), None);
}

#[test]
fn a_series_carries_the_sets_transport_and_song_and_an_older_hubs_reads_as_unknown() {
    let series = HistorySeries {
        from: 1_794_736_800_000,
        step_ms: 1000,
        values: vec![Some(-230), None, Some(-700)],
        fader: Vec::new(),
        play: vec![Some(true), Some(false), None],
        song: vec![Some(31), None, None],
        songs: BTreeMap::from([(31, "Morning Light".to_string())]),
    };
    let text = serde_json::to_string(&series).unwrap();
    assert_eq!(
        text,
        r#"{"from":1794736800000,"step_ms":1000,"values":[-230,null,-700],"play":[true,false,null],"song":[31,null,null],"songs":{"31":"Morning Light"}}"#
    );
    assert_eq!(
        serde_json::from_str::<HistorySeries>(&text).unwrap(),
        series
    );
    let old: HistorySeries =
        serde_json::from_str(r#"{"from":5,"step_ms":1000,"values":[-230]}"#).unwrap();
    assert!(old.play.is_empty());
    assert!(old.song.is_empty());
    assert!(old.songs.is_empty());
}

#[test]
fn a_bucket_plays_when_half_or_more_of_its_known_seconds_play() {
    assert_eq!(play_of(&[]), None);
    assert_eq!(play_of(&[true]), Some(true));
    assert_eq!(play_of(&[false]), Some(false));
    // Half is enough; under half is a stop.
    assert_eq!(play_of(&[true, false]), Some(true));
    assert_eq!(play_of(&[false, true, false]), Some(false));
    assert_eq!(play_of(&[true, true, false]), Some(true));
    assert_eq!(play_of(&[false, false, false, true]), Some(false));
}

#[test]
fn a_buckets_song_is_the_one_most_of_its_playing_seconds_hold() {
    assert_eq!(song_of(&[]), None);
    assert_eq!(song_of(&[31]), Some(31));
    assert_eq!(song_of(&[31, 57, 57]), Some(57));
    assert_eq!(song_of(&[57, 31, 31, 57, 31]), Some(31));
    // A tie: the song heard first (a song ending, the next starting).
    assert_eq!(song_of(&[31, 31, 57, 57]), Some(31));
    assert_eq!(song_of(&[57, 31]), Some(57));
}
