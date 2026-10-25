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
    let (on, off, between) = (Some(true), Some(false), None);
    // Nothing on its way: Live shows otherwise, written.
    assert!(needs_write(true, off, None, false));
    assert!(needs_write(false, on, None, false));
    // Between the values (PR #147 review N4: a stream selector in its
    // chain's zone): neither decision is shown, both are written.
    assert!(needs_write(true, between, None, false));
    assert!(needs_write(false, between, None, false));
    // Live shows it: nothing.
    assert!(!needs_write(true, on, None, false));
    assert!(!needs_write(false, off, None, false));
    // Its write on its way: nothing again, whatever Live shows yet.
    assert!(!needs_write(true, off, Some(true), false));
    assert!(!needs_write(false, on, Some(false), false));
    // Another write on its way (PR #147 review I1): this one, also when
    // Live still shows the decision (a stop right after a switch-on, a
    // start right after a stop).
    assert!(needs_write(true, off, Some(false), false));
    assert!(needs_write(false, on, Some(true), false));
    assert!(needs_write(false, off, Some(true), false));
    assert!(needs_write(true, on, Some(false), false));
    // The hub stopped writing it: never.
    assert!(!needs_write(false, on, None, true));
    assert!(!needs_write(true, off, None, true));
    assert!(!needs_write(false, on, Some(true), true));
}

#[test]
fn another_writers_move_is_one_away_from_the_decision_that_is_no_echo() {
    let (on, off, between) = (Some(true), Some(false), None);
    // Off DRY while the hub wants DRY: to the chain, or between (N4).
    assert_eq!(another_move(off, on, false, &[]), Some(true));
    assert_eq!(another_move(off, between, false, &[]), Some(true));
    assert_eq!(another_move(between, on, false, &[false]), Some(true));
    // Off the chain while the hub wants the chain.
    assert_eq!(another_move(on, off, true, &[]), Some(false));
    assert_eq!(another_move(on, between, true, &[true]), Some(false));
    // The echo of a hub write on its way: of either of two (I1).
    assert_eq!(another_move(off, on, false, &[true]), None);
    assert_eq!(another_move(off, on, false, &[true, false]), None);
    assert_eq!(another_move(on, off, true, &[false, true]), None);
    // To the decision, or no move.
    assert_eq!(another_move(off, on, true, &[]), None);
    assert_eq!(another_move(on, off, false, &[]), None);
    assert_eq!(another_move(on, on, false, &[]), None);
    assert_eq!(another_move(between, between, false, &[]), None);
}

fn fly(write: u64, on: bool, answered: bool) -> Flying {
    Flying {
        write,
        on,
        answered,
    }
}

#[test]
fn a_value_at_a_write_on_its_way_lands_it_and_every_one_before() {
    // PR #147 review I1: Live takes the hub's writes in order. A value at
    // the chain lands the first write at it (PR #147 re-review R5): the
    // DRY and the chain after it stay on their way, so their echoes are
    // still the hub's own.
    let flying = [
        fly(1, true, false),
        fly(2, false, false),
        fly(3, true, false),
    ];
    assert_eq!(
        still_flying(&flying, Some(true)),
        vec![fly(2, false, false), fly(3, true, false)]
    );
    assert_eq!(
        still_flying(&flying, Some(false)),
        vec![fly(3, true, false)]
    );
    assert_eq!(
        still_flying(&flying[..2], Some(true)),
        vec![fly(2, false, false)]
    );
    assert_eq!(still_flying(&flying[..2], Some(false)), vec![]);
    // A value at none of them (between, or none on its way) lands none.
    assert_eq!(still_flying(&flying, None), flying.to_vec());
    assert_eq!(
        still_flying(&[fly(4, true, false)], Some(false)),
        vec![fly(4, true, false)]
    );
    assert_eq!(still_flying(&[], Some(true)), vec![]);
    // An answered write is taken by any value heard after its answer (its
    // echo or not); the unanswered ones after it stay.
    let answered = [fly(1, true, true), fly(2, false, false)];
    assert_eq!(
        still_flying(&answered, Some(true)),
        vec![fly(2, false, false)]
    );
    assert_eq!(still_flying(&answered, None), vec![fly(2, false, false)]);
    assert_eq!(still_flying(&answered, Some(false)), vec![]);
    assert_eq!(
        still_flying(&[fly(1, true, true), fly(2, false, true)], Some(true)),
        vec![]
    );
}

#[test]
fn a_write_answered_is_taken_and_a_failed_one_never_lands() {
    let flying = [fly(1, true, false), fly(2, false, false)];
    // Failed: it never lands; the others stay as they were.
    assert_eq!(
        answered_flying(&flying, 1, false),
        vec![fly(2, false, false)]
    );
    assert_eq!(
        answered_flying(&flying, 2, false),
        vec![fly(1, true, false)]
    );
    // Went: answered, its echo (if any) the next value.
    assert_eq!(
        answered_flying(&flying, 2, true),
        vec![fly(1, true, false), fly(2, false, true)]
    );
    assert_eq!(
        answered_flying(&flying, 1, true),
        vec![fly(1, true, true), fly(2, false, false)]
    );
    // An answer of a write no longer on its way changes nothing.
    assert_eq!(answered_flying(&flying, 7, false), flying.to_vec());
    assert_eq!(answered_flying(&flying, 7, true), flying.to_vec());
    let both = [fly(1, true, true), fly(2, false, false)];
    assert_eq!(
        answered_flying(&both, 2, true),
        vec![fly(1, true, true), fly(2, false, true)]
    );
}

#[test]
fn a_refused_selector_write_gets_one_retry_then_the_hub_stops() {
    // PR #147 review I5, ruled.
    assert_eq!(SELECTOR_RETRIES, 1);
    let first = Refusal::after(None, "p7", true, "held");
    assert_eq!(
        first,
        Refusal {
            id: "p7".into(),
            on: true,
            tries: 1,
            why: "held".into(),
        }
    );
    assert!(first.holds("p7", true));
    assert!(!first.stops("p7", true), "its one retry goes");
    let second = Refusal::after(Some(&first), "p7", true, "held again");
    assert_eq!((second.tries, second.why.as_str()), (2, "held again"));
    assert!(second.stops("p7", true));
    // Another `$ref` or the other side starts over, and the refusal does
    // not hold for them.
    assert_eq!(Refusal::after(Some(&second), "p8", true, "x").tries, 1);
    assert_eq!(Refusal::after(Some(&second), "p7", false, "x").tries, 1);
    assert!(!second.holds("p8", true) && !second.stops("p8", true));
    assert!(!second.holds("p7", false) && !second.stops("p7", false));
    // Live refused the chain for good: DRY with a problem; DRY refused
    // stays DRY (its write stops).
    let play = Decision {
        on: true,
        why: SwitchWhy::Play,
    };
    assert_eq!(
        refused_decision(play, Some(&second), "p7"),
        off(SwitchWhy::Problem)
    );
    assert_eq!(refused_decision(play, Some(&first), "p7"), play);
    assert_eq!(refused_decision(play, Some(&second), "p8"), play);
    assert_eq!(refused_decision(play, None, "p7"), play);
    let dry = Refusal::after(
        Some(&Refusal::after(None, "p7", false, "x")),
        "p7",
        false,
        "x",
    );
    assert!(dry.stops("p7", false));
    assert_eq!(refused_decision(play, Some(&dry), "p7"), play);
    assert_eq!(
        refused_decision(off(SwitchWhy::Stop), Some(&second), "p7"),
        off(SwitchWhy::Stop)
    );
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
