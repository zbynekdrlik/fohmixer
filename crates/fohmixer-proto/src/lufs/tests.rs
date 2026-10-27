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
