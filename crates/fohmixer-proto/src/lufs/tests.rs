use super::*;

const TARGET: Option<i8> = Some(-25);

#[test]
fn judge_on_every_boundary() {
    let cases = [
        (-220, LufsState::Fine),
        (-210, LufsState::Warn),
        (-190, LufsState::Warn),
        (-180, LufsState::Far),
        (-280, LufsState::Fine),
        (-290, LufsState::Warn),
        (-310, LufsState::Warn),
        (-320, LufsState::Far),
        (-250, LufsState::Fine),
        (-699, LufsState::Far),
        (-700, LufsState::Silent),
        (-701, LufsState::Silent),
        (i16::MIN, LufsState::Silent),
        (0, LufsState::Far),
    ];
    for (tenths, state) in cases {
        assert_eq!(judge(Some(tenths), TARGET), state, "{tenths}");
    }
}

#[test]
fn the_tint_follows_the_shown_integer() {
    for (tenths, shows, state) in [
        (-215, -22, LufsState::Fine),
        (-224, -22, LufsState::Fine),
        (-216, -22, LufsState::Fine),
        (-185, -19, LufsState::Warn),
        (-214, -21, LufsState::Warn),
        (-285, -29, LufsState::Warn),
        (-284, -28, LufsState::Fine),
    ] {
        assert_eq!(shown(tenths), shows, "{tenths}");
        assert_eq!(judge(Some(tenths), TARGET), state, "{tenths}");
    }
}

#[test]
fn shown_rounds_half_away_from_zero() {
    for (tenths, shows) in [
        (-215, -22),
        (-214, -21),
        (-216, -22),
        (-210, -21),
        (5, 1),
        (4, 0),
        (-5, -1),
        (-4, 0),
        (0, 0),
        (i16::MAX, 3277),
        (i16::MIN, -3277),
    ] {
        assert_eq!(shown(tenths), shows, "{tenths}");
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
        (LufsState::Fine, "fine"),
        (LufsState::Warn, "warn"),
        (LufsState::Far, "far"),
        (LufsState::Plain, "plain"),
        (LufsState::Silent, "silent"),
        (LufsState::None, "none"),
    ] {
        assert_eq!(state.as_str(), word);
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!(word)
        );
    }
}

#[test]
fn the_owners_constants() {
    assert_eq!((FINE_LU, FAR_LU, SILENT_TENTHS), (3, 6, -700));
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
