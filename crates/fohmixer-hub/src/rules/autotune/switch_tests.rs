use super::*;

const F: Key = Key {
    root: 5,
    minor: false,
};
const G: Key = Key {
    root: 7,
    minor: false,
};

/// Every condition met but the key's: AUT on, playing in song 31 in F, the
/// singer on, no problem, no other writer, the fall back armed.
fn tuning(key: KeyCheck, selected: bool) -> TrackInput {
    TrackInput {
        aut: true,
        held: false,
        playing: Some(true),
        in_force: (Some(31), Some(F)),
        singer_off: false,
        problem: false,
        another_writer: false,
        armed: true,
        key,
        selected,
    }
}

fn off(why: SwitchWhy) -> Decision {
    Decision { on: false, why }
}

const ON: Decision = Decision {
    on: true,
    why: SwitchWhy::Play,
};

/// Every key check and both `selected`.
fn every_key() -> Vec<(KeyCheck, bool)> {
    [KeyCheck::Confirmed, KeyCheck::Pending, KeyCheck::Bad]
        .into_iter()
        .flat_map(|key| [(key, false), (key, true)])
        .collect()
}

#[test]
fn aut_off_is_dry_always() {
    for (key, selected) in every_key() {
        for playing in [None, Some(false), Some(true)] {
            let input = TrackInput {
                aut: false,
                playing,
                ..tuning(key, selected)
            };
            assert_eq!(decide(&input), off(SwitchWhy::Aut), "{input:?}");
        }
    }
    // AUT off comes first, a hold or a stop too.
    let all_off = TrackInput {
        aut: false,
        held: true,
        playing: None,
        in_force: (None, None),
        singer_off: true,
        problem: true,
        another_writer: true,
        armed: false,
        key: KeyCheck::Bad,
        selected: false,
    };
    assert_eq!(decide(&all_off), off(SwitchWhy::Aut));
}

#[test]
fn a_held_set_is_dry() {
    for (key, selected) in every_key() {
        let input = TrackInput {
            held: true,
            ..tuning(key, selected)
        };
        assert_eq!(decide(&input), off(SwitchWhy::Held), "{input:?}");
    }
}

#[test]
fn stop_is_dry_at_once_whatever_else() {
    for (key, selected) in every_key() {
        for playing in [None, Some(false)] {
            let input = TrackInput {
                playing,
                ..tuning(key, selected)
            };
            assert_eq!(decide(&input), off(SwitchWhy::Stop), "{input:?}");
        }
    }
}

#[test]
fn out_of_every_song_is_span_and_a_keyless_song_no_key() {
    // PR #143 review M2: the two are told apart.
    for (key, selected) in every_key() {
        let out = TrackInput {
            in_force: (None, None),
            ..tuning(key, selected)
        };
        assert_eq!(decide(&out), off(SwitchWhy::Span), "{out:?}");
        let keyless = TrackInput {
            in_force: (Some(88), None),
            ..tuning(key, selected)
        };
        assert_eq!(decide(&keyless), off(SwitchWhy::NoKey), "{keyless:?}");
    }
}

#[test]
fn a_singer_off_by_hand_is_dry() {
    for (key, selected) in every_key() {
        let input = TrackInput {
            singer_off: true,
            ..tuning(key, selected)
        };
        assert_eq!(decide(&input), off(SwitchWhy::SingerOff), "{input:?}");
    }
}

#[test]
fn a_problem_is_dry() {
    for (key, selected) in every_key() {
        let input = TrackInput {
            problem: true,
            ..tuning(key, selected)
        };
        assert_eq!(decide(&input), off(SwitchWhy::Problem), "{input:?}");
        let writer = TrackInput {
            another_writer: true,
            ..tuning(key, selected)
        };
        assert_eq!(decide(&writer), off(SwitchWhy::AnotherWriter), "{writer:?}");
    }
    // The singer's off comes before a problem, a problem before another
    // writer.
    let both = TrackInput {
        singer_off: true,
        problem: true,
        another_writer: true,
        ..tuning(KeyCheck::Confirmed, true)
    };
    assert_eq!(decide(&both), off(SwitchWhy::SingerOff));
    let both = TrackInput {
        problem: true,
        another_writer: true,
        ..tuning(KeyCheck::Confirmed, true)
    };
    assert_eq!(decide(&both), off(SwitchWhy::Problem));
}

#[test]
fn on_needs_the_fall_back_armed() {
    // PR #146 review I2: never on without the script's DRY writes kept,
    // a selected track included.
    for (key, selected) in every_key() {
        let input = TrackInput {
            armed: false,
            ..tuning(key, selected)
        };
        assert_eq!(decide(&input), off(SwitchWhy::Unarmed), "{input:?}");
    }
    let writer = TrackInput {
        armed: false,
        another_writer: true,
        ..tuning(KeyCheck::Confirmed, true)
    };
    assert_eq!(decide(&writer), off(SwitchWhy::AnotherWriter));
}

#[test]
fn on_needs_the_key_confirmed_for_the_key_in_force() {
    assert_eq!(
        decide(&tuning(KeyCheck::Pending, false)),
        off(SwitchWhy::Mismatch)
    );
    assert_eq!(decide(&tuning(KeyCheck::Confirmed, false)), ON);
    assert_eq!(decide(&tuning(KeyCheck::Confirmed, true)), ON);
    assert_eq!(
        decide(&tuning(KeyCheck::Bad, false)),
        off(SwitchWhy::Mismatch)
    );
}

#[test]
fn a_tracking_track_stays_on_through_a_key_change_until_its_read_back_disagrees() {
    assert_eq!(decide(&tuning(KeyCheck::Pending, true)), ON);
    assert_eq!(
        decide(&tuning(KeyCheck::Bad, true)),
        off(SwitchWhy::Mismatch)
    );
}

#[test]
fn the_key_check_compares_want_with_the_key_in_force() {
    use KeyState::{AnotherWriter, Confirmed, Mismatch, NoKey, Refused, Writing};
    // PR #144 review M2: `state` judges `want`.
    assert_eq!(
        key_check(true, Some(F), Confirmed, Some(F)),
        KeyCheck::Confirmed
    );
    assert_eq!(
        key_check(true, Some(F), Confirmed, Some(G)),
        KeyCheck::Pending,
        "the old key confirmed: the new one's write to come"
    );
    assert_eq!(key_check(true, None, NoKey, Some(F)), KeyCheck::Pending);
    assert_eq!(
        key_check(true, Some(F), Writing, Some(F)),
        KeyCheck::Pending
    );
    assert_eq!(key_check(true, Some(F), NoKey, Some(F)), KeyCheck::Pending);
    for bad in [Mismatch, Refused, AnotherWriter] {
        assert_eq!(
            key_check(true, Some(F), bad, Some(F)),
            KeyCheck::Bad,
            "{bad:?}"
        );
        assert_eq!(
            key_check(true, Some(F), bad, Some(G)),
            KeyCheck::Pending,
            "{bad:?} of another key"
        );
        assert_eq!(
            key_check(false, Some(F), bad, Some(F)),
            KeyCheck::Pending,
            "{bad:?} without the device"
        );
    }
    assert_eq!(
        key_check(false, Some(F), Confirmed, Some(F)),
        KeyCheck::Pending,
        "no key device followed now"
    );
    assert_eq!(
        key_check(true, None, Confirmed, None),
        KeyCheck::Pending,
        "no key"
    );
    assert_eq!(key_check(true, Some(F), Confirmed, None), KeyCheck::Pending);
}

#[test]
fn every_why_has_its_word() {
    let words: Vec<&str> = [
        SwitchWhy::Aut,
        SwitchWhy::Held,
        SwitchWhy::Stop,
        SwitchWhy::Span,
        SwitchWhy::NoKey,
        SwitchWhy::SingerOff,
        SwitchWhy::Problem,
        SwitchWhy::AnotherWriter,
        SwitchWhy::Unarmed,
        SwitchWhy::Mismatch,
        SwitchWhy::Play,
    ]
    .into_iter()
    .map(why_word)
    .collect();
    assert_eq!(
        words,
        [
            "aut",
            "held",
            "stop",
            "span",
            "no_key",
            "singer_off",
            "problem",
            "another_writer",
            "unarmed",
            "mismatch",
            "play"
        ]
    );
}

#[test]
fn a_selector_is_written_once_per_change() {
    // Live shows otherwise: written.
    assert!(needs_write(true, false, None, false));
    assert!(needs_write(false, true, None, false));
    // Live shows it: nothing.
    assert!(!needs_write(true, true, None, false));
    assert!(!needs_write(false, false, None, false));
    // Its write on its way: nothing again; another write on its way: this.
    assert!(!needs_write(true, false, Some(true), false));
    assert!(!needs_write(false, true, Some(false), false));
    assert!(needs_write(true, false, Some(false), false));
    assert!(needs_write(false, true, Some(true), false));
    // The guard stopped the hub: never.
    assert!(!needs_write(false, true, None, true));
    assert!(!needs_write(true, false, None, true));
}

#[test]
fn another_writers_move_is_one_away_from_the_decision_that_is_no_echo() {
    // To the chain while the hub wants DRY.
    assert_eq!(another_move(false, true, false, None), Some(true));
    assert_eq!(another_move(false, true, false, Some(false)), Some(true));
    // To DRY while the hub wants the chain.
    assert_eq!(another_move(true, false, true, None), Some(false));
    assert_eq!(another_move(true, false, true, Some(true)), Some(false));
    // The echo of the hub's own write.
    assert_eq!(another_move(false, true, false, Some(true)), None);
    assert_eq!(another_move(true, false, true, Some(false)), None);
    // Toward the decision, or no move.
    assert_eq!(another_move(false, true, true, None), None);
    assert_eq!(another_move(true, false, false, None), None);
    assert_eq!(another_move(true, true, false, None), None);
    assert_eq!(another_move(false, false, true, None), None);
}

#[test]
fn another_writer_turns_dry_once_and_gives_up_on_a_second_move_within_10_s() {
    assert_eq!(SELECTOR_GUARD_MS, 10_000.0);
    let mut guard = SelectorGuard::new();
    assert!(!guard.given_up() && !guard.another());
    assert_eq!(guard.moved(0.0), GuardAct::WriteDryOnce);
    assert!(!guard.given_up() && !guard.another());
    assert_eq!(guard.moved(9_999.0), GuardAct::GiveUp);
    assert!(guard.given_up() && guard.another());
    // Given up, it stays so.
    assert_eq!(guard.moved(60_000.0), GuardAct::GiveUp);
    // The next song start resets it.
    guard.reset();
    assert!(!guard.given_up() && !guard.another());
    assert_eq!(guard.moved(20_000.0), GuardAct::WriteDryOnce);
    // A second move after 10 s is a first one again: its own window.
    let mut fresh = SelectorGuard::new();
    assert_eq!(fresh.moved(0.0), GuardAct::WriteDryOnce);
    assert_eq!(fresh.moved(10_001.0), GuardAct::WriteDryOnce);
    assert_eq!(fresh.moved(15_000.0), GuardAct::GiveUp);
}

#[test]
fn the_guards_window_is_10_s_after_the_first_move() {
    // At 10 s exactly: within; the next float after: a first move again.
    let mut at = SelectorGuard::new();
    at.moved(5_000.0);
    assert_eq!(at.moved(15_000.0), GuardAct::GiveUp);
    let mut after = SelectorGuard::new();
    after.moved(5_000.0);
    assert_eq!(after.moved(15_000.0_f64.next_up()), GuardAct::WriteDryOnce);
    let mut just = SelectorGuard::new();
    just.moved(5_000.0);
    assert_eq!(just.moved(14_999.0), GuardAct::GiveUp);
}

#[test]
fn a_selector_left_dry_is_another_writers_until_the_reset() {
    let mut guard = SelectorGuard::new();
    guard.left();
    assert!(guard.another());
    assert!(!guard.given_up(), "nothing to stop writing: it is DRY");
    assert_eq!(guard.moved(0.0), GuardAct::WriteDryOnce);
    guard.reset();
    assert!(!guard.another());
}
