use super::*;
use fohmixer_proto::layout::{Binding, Scale};

fn target(anchor: Anchor, path: Option<&str>, prop: &str) -> ParamTarget {
    ParamTarget {
        binding: Binding {
            instance: "band".to_string(),
            anchor,
            path: path.map(str::to_string),
        },
        prop: prop.to_string(),
        on: None,
        off: None,
        scale: Some(Scale::CcLinear),
    }
}

fn track(name: &str) -> Anchor {
    Anchor::Track {
        name: name.to_string(),
    }
}

/// A track's volume, as the fixture's Podklady All writes them.
fn volume(name: &str) -> ParamTarget {
    target(track(name), Some("mixer_device volume"), "value")
}

#[track_caller]
fn assert_db(volume: f64, want: f64, tolerance: f64) {
    let got = value2db(volume);
    assert!(
        (got - want).abs() <= tolerance,
        "{volume} reads {got} dB, not {want}"
    );
}

#[test]
fn a_track_volume_is_a_tracks_mixer_device_volume_value() {
    assert!(is_track_volume(&volume("Drums #")));
    assert!(is_track_volume(&target(
        Anchor::TrackAt { index: 3 },
        Some("mixer_device volume"),
        "value"
    )));
    assert!(is_track_volume(&target(
        track("Bass #"),
        Some("  mixer_device   volume "),
        "value"
    )));
    // Each part on its own makes it something else.
    let ret = Anchor::Return {
        name: "A-Reverb #".to_string(),
    };
    assert!(!is_track_volume(&target(
        ret,
        Some("mixer_device volume"),
        "value"
    )));
    assert!(!is_track_volume(&target(
        Anchor::Master,
        Some("mixer_device volume"),
        "value"
    )));
    assert!(!is_track_volume(&target(
        track("Drums #"),
        Some("mixer_device panning"),
        "value"
    )));
    assert!(!is_track_volume(&target(
        track("Drums #"),
        Some("mixer_device sends 1"),
        "value"
    )));
    assert!(!is_track_volume(&target(
        track("Drums #"),
        Some("mixer_device volume extra"),
        "value"
    )));
    assert!(!is_track_volume(&target(track("Drums #"), None, "value")));
    assert!(!is_track_volume(&target(
        track("Drums #"),
        Some("mixer_device volume"),
        "min"
    )));
}

#[test]
fn only_a_fader_of_two_or_more_track_volumes_is_the_backing_tracks_fader() {
    // The fixture's Podklady All: two volumes; the site's has five.
    assert!(is_vca(&[volume("Drums #"), volume("Bass #")]));
    let five: Vec<ParamTarget> = (1..=5).map(|i| volume(&format!("Stem {i}"))).collect();
    assert!(is_vca(&five));
    // A single volume (the markers spec's OWN) keeps today's fader.
    assert!(!is_vca(&[volume("Drums #")]));
    assert!(!is_vca(&[]));
    // A mix, or no volume at all.
    let send = |name: &str| target(track(name), Some("mixer_device sends 1"), "value");
    assert!(!is_vca(&[volume("Drums #"), send("Bass #")]));
    assert!(!is_vca(&[
        send("Drums #"),
        volume("Bass #"),
        volume("Vox 1")
    ]));
    assert!(!is_vca(&[send("Drums #"), send("Bass #")]));
}

#[test]
fn the_loudest_is_the_largest_known_volume() {
    assert_eq!(loudest(&[Some(0.5), None, Some(0.55)]), Some(0.55));
    assert_eq!(loudest(&[Some(0.7), Some(0.2)]), Some(0.7));
    assert_eq!(loudest(&[Some(0.0), None]), Some(0.0));
    assert_eq!(loudest(&[None, None]), None);
    assert_eq!(loudest(&[]), None);
}

#[test]
fn the_fader_thresholds_are_its_own() {
    // The law's ends are `fohmixer_proto::volume`'s (#130), tested there.
    assert_eq!(SILENT_DB, -69.95);
    assert_eq!(MOVED_DB, 0.05);
    assert_eq!(SAME_DB, 0.000_001);
}

#[test]
fn a_track_shows_moved_from_0_05_db_and_silent_from_minus_69_95_db() {
    assert!(is_moved(0.05));
    assert!(is_moved(-0.05));
    assert!(!is_moved(0.05_f64.next_down()));
    assert!(!is_moved((-0.05_f64).next_up()));
    assert!(!is_moved(0.0));
    assert!(is_moved(-14.0));
    assert!(is_moved(2.0));
    assert!(is_moved(f64::NEG_INFINITY));
    assert!(is_silent(-69.95));
    assert!(!is_silent((-69.95_f64).next_up()));
    assert!(is_silent(-70.0));
    assert!(is_silent(f64::NEG_INFINITY));
    assert!(!is_silent(-14.0));
}

#[test]
fn a_change_under_a_millionth_of_a_db_is_no_change() {
    assert!(is_same(0.0));
    assert!(is_same(0.000_001_f64.next_down()));
    assert!(is_same(-0.000_001_f64.next_down()));
    assert!(!is_same(0.000_001));
    assert!(!is_same(-0.000_001));
    assert!(!is_same(f64::NAN));
    assert!(!is_same(f64::NEG_INFINITY));
}

#[test]
fn a_drag_needs_a_target() {
    assert_eq!(Drag::new(Vec::new()), None);
    let one = Drag::new(vec![0.3]).expect("one target");
    assert_eq!(one.volumes(0.4), vec![0.4]);
}

#[test]
fn the_loudest_follows_the_finger_and_every_other_track_moves_by_the_same_db() {
    // −14 and −12 dB; the finger takes the loudest to −18 dB: 6 dB down.
    let drag = Drag::new(vec![0.5, 0.55]).expect("targets");
    assert_eq!(drag.volumes(0.4), vec![0.360015869140625, 0.4]);
    assert_db(drag.volumes(0.4)[0], -20.0, 0.002);
    // The mockup's prayer mode (−14, −14, −12, −15, −14) and the loudest
    // up to −6 dB: 6 dB up, the balance kept.
    let prayer = Drag::new(vec![0.5, 0.5, 0.55, 0.475, 0.5]).expect("targets");
    let up = prayer.volumes(0.7);
    assert_eq!(
        up,
        vec![
            0.649993896484375,
            0.649993896484375,
            0.7,
            0.624969482421875,
            0.649993896484375
        ]
    );
    for (got, want) in up.iter().zip([-8.0, -8.0, -6.0, -9.0, -8.0]) {
        assert_db(*got, want, 0.002);
    }
}

#[test]
fn a_drag_back_to_its_start_restores_the_exact_values() {
    let drag = Drag::new(vec![0.5, 0.55]).expect("targets");
    // The bisection alone would give 0.499969482421875 for −14 dB.
    assert_eq!(drag.volumes(0.55), vec![0.5, 0.55]);
    // The finger's summed moves come back a few ulps off: still no move.
    assert_eq!(drag.volumes(0.55_f64.next_up()), vec![0.5, 0.55]);
    assert_eq!(drag.volumes(0.55_f64.next_down()), vec![0.5, 0.55]);
}

#[test]
fn the_balance_comes_back_after_the_floor() {
    // −14, −12 and 0 dB pulled down to −∞, then part way up, then back.
    let drag = Drag::new(vec![0.5, 0.55, 0.85]).expect("targets");
    assert_eq!(drag.volumes(0.0), vec![0.0, 0.0, 0.0]);
    let part = drag.volumes(0.2);
    let delta = value2db(0.2);
    assert_db(part[0], -14.0 + delta, 0.01);
    assert_db(part[1], -12.0 + delta, 0.01);
    assert_eq!(part[2], 0.2);
    assert_eq!(drag.volumes(0.85), vec![0.5, 0.55, 0.85]);
    // A quiet track that a move pushes below the floor comes back to its
    // own level: every move is computed from the touch's start.
    let quiet = Drag::new(vec![0.85, 0.1]).expect("targets");
    let first = quiet.volumes(0.4);
    assert_eq!(first[0], 0.4);
    assert_db(first[1], value2db(0.1) - 18.0, 0.02);
    assert_eq!(quiet.volumes(0.25), vec![0.25, 0.0]);
    assert_eq!(quiet.volumes(0.4), first);
}

#[test]
fn a_drag_stays_within_the_law_at_both_ends() {
    // 0 and −6 dB with the finger at the top: the loudest at +6 dB (volume
    // 1), the other 6 dB up at 0 dB; a finger past either end is that end.
    let drag = Drag::new(vec![0.85, 0.7]).expect("targets");
    assert_eq!(drag.volumes(1.0), vec![1.0, 0.850006103515625]);
    assert_eq!(drag.volumes(1.5), vec![1.0, 0.850006103515625]);
    assert_eq!(drag.volumes(-0.5), vec![0.0, 0.0]);
    assert_eq!(drag.volumes(0.0), vec![0.0, 0.0]);
}

#[test]
fn silent_tracks_follow_the_finger_only_when_every_one_is_silent() {
    // All at −∞: they are all the loudest and all take the finger.
    let silent = Drag::new(vec![0.0, 0.0]).expect("targets");
    assert_eq!(silent.volumes(0.3), vec![0.3, 0.3]);
    // One at −∞ beside a sounding one stays at −∞.
    let one = Drag::new(vec![0.0, 0.5]).expect("targets");
    assert_eq!(one.volumes(0.6), vec![0.0, 0.6]);
}

#[test]
fn the_loudest_reads_one_decimal_with_its_sign() {
    assert_eq!(loudest_text(0.0), "0.0 dB");
    assert_eq!(loudest_text(-0.04), "0.0 dB");
    assert_eq!(loudest_text(0.049), "0.0 dB");
    assert_eq!(loudest_text(0.05), "+0.1 dB");
    assert_eq!(loudest_text(-0.05), "-0.1 dB");
    assert_eq!(loudest_text(-14.0), "-14.0 dB");
    assert_eq!(loudest_text(-11.999999999999996), "-12.0 dB");
    assert_eq!(loudest_text(2.0), "+2.0 dB");
    assert_eq!(loudest_text(6.0), "+6.0 dB");
    assert_eq!(loudest_text((-69.95_f64).next_up()), "-69.9 dB");
    assert_eq!(loudest_text(-69.95), "-∞ dB");
    assert_eq!(loudest_text(f64::NEG_INFINITY), "-∞ dB");
}

#[test]
fn every_track_reads_whole_db_rounded_half_up() {
    assert_eq!(value_text(Some(-14.0)), "-14");
    assert_eq!(value_text(Some(-14.5)), "-14");
    assert_eq!(value_text(Some(-14.51)), "-15");
    assert_eq!(value_text(Some(2.5)), "3");
    assert_eq!(value_text(Some(1.9)), "2");
    assert_eq!(value_text(Some(6.0)), "6");
    assert_eq!(value_text(Some(0.0)), "0");
    assert_eq!(value_text(Some(-0.4)), "0");
    assert_eq!(value_text(Some(-0.5)), "0");
    assert_eq!(value_text(Some(-69.95)), "-∞");
    assert_eq!(value_text(Some(f64::NEG_INFINITY)), "-∞");
    assert_eq!(value_text(None), "–");
}

#[test]
fn the_values_go_three_to_a_line() {
    let five = [-14.0, -14.0, -12.0, -15.0, -14.0].map(Some);
    assert_eq!(values_text(&five), "-14 -14 -12\n-15 -14");
    assert_eq!(values_text(&[Some(-14.0), Some(-12.0)]), "-14 -12");
    assert_eq!(values_text(&[Some(0.0), None, Some(0.0)]), "0 – 0");
    assert_eq!(values_text(&[Some(0.0); 4]), "0 0 0\n0");
    assert_eq!(values_text(&[]), "");
}

#[test]
fn at_0_db_the_fader_looks_like_any_strip() {
    let law = VolumeLaw::Live;
    let groove = Groove {
        pos: 0.85,
        db: "0.000".to_string(),
        moved: false,
    };
    assert_eq!(
        look(law, &[Some(0.85), Some(0.85)]),
        Look {
            moved: false,
            loudest: "0.0 dB".to_string(),
            values: "0 0".to_string(),
            grooves: vec![groove.clone(), groove],
        }
    );
}

#[test]
fn a_track_off_0_db_turns_the_fader_amber() {
    let law = VolumeLaw::Live;
    assert_eq!(
        look(law, &[Some(0.5), Some(0.55)]),
        Look {
            moved: true,
            loudest: "-12.0 dB".to_string(),
            values: "-14 -12".to_string(),
            grooves: vec![
                Groove {
                    pos: 0.5,
                    db: "-14.000".to_string(),
                    moved: true,
                },
                Groove {
                    pos: 0.55,
                    db: "-12.000".to_string(),
                    moved: true,
                },
            ],
        }
    );
    // One track off is enough; silence is off too.
    let one = look(law, &[Some(0.85), Some(0.0)]);
    assert!(one.moved);
    assert_eq!(one.loudest, "0.0 dB");
    assert_eq!(one.values, "0 -∞");
    assert_eq!(
        one.grooves[1],
        Groove {
            pos: 0.0,
            db: "-inf".to_string(),
            moved: true,
        }
    );
    assert!(!one.grooves[0].moved);
}

#[test]
fn a_groove_sits_on_the_strips_law_and_waits_for_its_value() {
    let tosc = VolumeLaw::TouchOsc;
    let shown = look(tosc, &[None, Some(0.85)]);
    assert!(!shown.moved);
    assert_eq!(shown.loudest, "0.0 dB");
    assert_eq!(shown.values, "– 0");
    assert_eq!(
        shown.grooves,
        vec![
            Groove {
                pos: 0.0,
                db: String::new(),
                moved: false,
            },
            Groove {
                pos: tosc.to_pos(0.85),
                db: "0.000".to_string(),
                moved: false,
            },
        ]
    );
    let none = look(tosc, &[None, None]);
    assert_eq!(none.loudest, "");
    assert_eq!(none.values, "– –");
    assert!(!none.moved);
}

/// The double tap's glide from 0 dB, −14 dB, −∞ and +4 dB on Live's law
/// (the position is Live's volume; 0 dB at 0.85).
fn home() -> Home {
    Home::new(
        VolumeLaw::Live,
        &[Some(0.85), Some(0.5), Some(0.0), Some(0.95)],
    )
    .expect("targets")
}

#[test]
fn a_double_taps_glide_needs_every_targets_value() {
    let law = VolumeLaw::Live;
    assert_eq!(Home::new(law, &[]), None);
    assert_eq!(Home::new(law, &[Some(0.5), None]), None);
    assert_eq!(Home::new(law, &[None, None]), None);
    let one = Home::new(law, &[Some(0.5)]).expect("one target");
    assert_eq!(one.volumes(1.0), vec![0.85]);
}

#[test]
fn the_glide_starts_at_the_loudest_and_lasts_the_furthest_tracks_way() {
    let h = home();
    // The cap from the loudest (+4 dB); the furthest way is −∞'s, 0.85.
    assert_eq!(h.cap_from(), 0.95);
    assert_eq!(h.span(), 0.85);
    // Prayer-mode levels: from −14 dB, the furthest −29.8 dB's 0.61.
    let prayer = Home::new(VolumeLaw::Live, &[Some(0.5), Some(0.24)]).expect("targets");
    assert_eq!(prayer.cap_from(), 0.5);
    assert_eq!(prayer.span(), 0.85 - 0.24);
    // The loudest at 0 dB and one moved up on its own: its way down.
    let up = Home::new(VolumeLaw::Live, &[Some(0.85), Some(1.0)]).expect("targets");
    assert_eq!(up.cap_from(), 1.0);
    assert_eq!(up.span(), 1.0 - 0.85);
    // Every track at 0 dB already: nowhere to go.
    let still = Home::new(VolumeLaw::Live, &[Some(0.85), Some(0.85)]).expect("targets");
    assert_eq!(still.cap_from(), 0.85);
    assert_eq!(still.span(), 0.0);
}

#[test]
fn the_glide_takes_every_track_from_its_start_to_exactly_0_db() {
    let h = home();
    // The start: every track where it was.
    assert_eq!(h.volumes(0.0), vec![0.85, 0.5, 0.0, 0.95]);
    // A quarter and half of the way, each on its own way on the law.
    let quarter = h.volumes(0.25);
    assert_eq!(quarter[0], 0.85, "the track at 0 dB stays exactly there");
    for (got, want) in quarter.iter().zip([0.85, 0.5875, 0.2125, 0.925]) {
        assert!((got - want).abs() < 1e-12, "{quarter:?}");
    }
    assert_eq!(quarter.len(), 4);
    let half = h.volumes(0.5);
    assert_eq!(half, vec![0.85, 0.675, 0.425, 0.8999999999999999]);
    for (got, want) in half.iter().zip([0.0, -7.0, -17.0, 2.0]) {
        assert_db(*got, want, 1e-9);
    }
    // The end: every track at Live's 0 dB volume, exactly (a strip's
    // double tap's), the one above 0 dB brought down too.
    assert_eq!(h.volumes(1.0), vec![UNITY; 4]);
    for volume in h.volumes(1.0) {
        assert_eq!(value2db(volume), 0.0);
        assert!(!is_moved(value2db(volume)));
    }
    // Before the start and past the end: the ends.
    assert_eq!(h.volumes(-0.5), vec![0.85, 0.5, 0.0, 0.95]);
    assert_eq!(h.volumes(1.5), vec![UNITY; 4]);
}

#[test]
fn every_track_moves_toward_0_db_and_the_loudest_stays_the_loudest() {
    let h = home();
    let steps: Vec<Vec<f64>> = (0..=20_u32)
        .map(|i| h.volumes(f64::from(i) / 20.0))
        .collect();
    for pair in steps.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        for (track, (b, a)) in before.iter().zip(after).enumerate() {
            let (b, a) = (value2db(*b), value2db(*a));
            // Never away from 0 dB, never past it.
            assert!(a.abs() <= b.abs(), "track {track}: {b} dB to {a} dB");
            assert!(a * b >= 0.0, "track {track} crossed 0 dB: {b} to {a}");
        }
        // Every one that was off 0 dB moves on every step until the end.
        assert!(
            before[1..].iter().zip(&after[1..]).all(|(b, a)| a != b),
            "{before:?} to {after:?}"
        );
        // The cap's track (+4 dB) is never below another one, and the one
        // at 0 dB stays exactly there (no float wobble around it).
        assert!(after.iter().all(|v| *v <= after[3]), "{after:?}");
        assert_eq!(after[0], UNITY);
    }
}

#[test]
fn on_touchoscs_law_the_glide_ends_where_a_strips_double_tap_does() {
    let law = VolumeLaw::TouchOsc;
    let h = Home::new(law, &[Some(0.85), Some(0.24)]).expect("targets");
    assert_eq!(h.cap_from(), law.to_pos(0.85));
    assert_eq!(h.span(), law.to_pos(UNITY) - law.to_pos(0.24));
    // A strip's glide ends at `to_pos(UNITY)` and writes that position's
    // volume.
    let strip = law.to_live(law.to_pos(UNITY));
    assert_eq!(h.volumes(1.0), vec![strip, strip]);
    let start = h.volumes(0.0);
    assert!((start[1] - 0.24).abs() < 1e-12, "{start:?}");
    // Half way on that law's positions.
    let half = h.volumes(0.5);
    let pos = (law.to_pos(0.24) + law.to_pos(UNITY)) / 2.0;
    assert!((half[1] - law.to_live(pos)).abs() < 1e-12, "{half:?}");
}
