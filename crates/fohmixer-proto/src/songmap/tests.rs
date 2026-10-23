use super::*;

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

fn kinds(m: &SongMap) -> Vec<(ProblemKind, Option<u16>, Option<f64>)> {
    m.problems
        .iter()
        .map(|p| (p.kind, p.song, p.time))
        .collect()
}

#[test]
fn a_song_runs_from_its_locator_to_its_song_end() {
    let m = build(
        &[
            cue("001 Test {C 120 4/4}", 0.0),
            cue("SONG END", 100.0),
            cue("002 Next {Am 90}", 120.0),
            cue("SONG END", 200.0),
        ],
        &[],
    );
    assert_eq!(m.spans.len(), 2);
    assert_eq!(
        (m.spans[0].number, m.spans[0].start, m.spans[0].end),
        (1, 0.0, Some(100.0))
    );
    assert_eq!(m.spans[0].key, Some(major(0)));
    assert_eq!(
        m.spans[1].key,
        Some(Key {
            root: 9,
            minor: true
        })
    );
    assert_eq!(
        (m.spans[1].number, m.spans[1].start, m.spans[1].end),
        (2, 120.0, Some(200.0))
    );
    assert!(m.problems.is_empty());
}

#[test]
fn locators_are_taken_in_time_order() {
    let m = build(
        &[
            cue("SONG END", 200.0),
            cue("002 Next {Am 90}", 120.0),
            cue("SONG END", 100.0),
            cue("001 Test {C 120 4/4}", 0.0),
        ],
        &[],
    );
    let spans: Vec<(u16, f64, Option<f64>)> =
        m.spans.iter().map(|s| (s.number, s.start, s.end)).collect();
    assert_eq!(spans, [(1, 0.0, Some(100.0)), (2, 120.0, Some(200.0))]);
    assert!(m.problems.is_empty());
}

#[test]
fn sections_and_ignored_cues_do_not_end_a_song() {
    let m = build(
        &[
            cue("001 T {D 100}", 0.0),
            cue("> Ending", 50.0),
            cue("* TS", 60.0),
            cue("*MS", 61.0),
            cue("[x] clyphx", 62.0),
            cue("SONG END", 90.0),
        ],
        &[],
    );
    assert_eq!(m.spans[0].end, Some(90.0));
    assert_eq!(m.spans[0].key, Some(major(2)));
    assert!(m.problems.is_empty());
}

#[test]
fn a_name_is_trimmed_before_it_is_read() {
    let m = build(
        &[cue("  058 T {F#/Gb 70 4/4}  ", 0.0), cue(" SONG END", 10.0)],
        &[],
    );
    assert_eq!(m.spans[0].number, 58);
    assert_eq!(m.spans[0].key, Some(major(6)));
    assert_eq!(m.spans[0].end, Some(10.0));
}

#[test]
fn stop_autostop_and_a_jump_flag_end_a_song() {
    for end in ["STOP", "AUTOSTOP", "X +JUMP", "SONGEND", "Y +STAY"] {
        let m = build(&[cue("001 T {E 90}", 0.0), cue(end, 40.0)], &[]);
        assert_eq!(m.spans[0].end, Some(40.0), "{end}");
        assert_eq!(m.spans[0].key, Some(major(4)), "{end}");
        assert!(m.problems.is_empty(), "{end}");
    }
}

#[test]
fn a_song_ended_by_the_next_song_is_a_problem_and_keyless() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("002 B {D 100}", 50.0),
            cue("SONG END", 90.0),
        ],
        &[],
    );
    assert_eq!(m.spans[0].key, None);
    assert_eq!(m.spans[0].end, None);
    assert_eq!(m.spans[1].key, Some(major(2)));
    assert!(
        m.problems
            .iter()
            .any(|p| p.kind == ProblemKind::NotEnded && p.song == Some(1))
    );
    assert_eq!(
        kinds(&m),
        [(ProblemKind::NotEnded, Some(1), Some(0.0))],
        "song 002 is fine"
    );
}

#[test]
fn a_song_ended_by_another_entry_or_by_nothing_is_not_ended() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("START", 50.0),
            cue("002 B {D 100}", 60.0),
        ],
        &[],
    );
    assert_eq!(
        kinds(&m),
        [
            (ProblemKind::NotEnded, Some(1), Some(0.0)),
            (ProblemKind::NotEnded, Some(2), Some(60.0))
        ]
    );
    assert!(m.spans.iter().all(|s| s.key.is_none() && s.end.is_none()));
}

#[test]
fn free_text_after_the_key_is_refused() {
    // Review M5: its own problem, which says the word must be the tempo.
    for name in [
        "001 T {A cappella}",
        "001 T {C 120bpm}",
        "001 T {F#m slow 4/4}",
    ] {
        let m = build(&[cue(name, 0.0), cue("SONG END", 9.0)], &[]);
        assert_eq!(m.spans[0].key, None, "{name}");
        assert_eq!(
            kinds(&m),
            [(ProblemKind::BadTempo, Some(1), Some(0.0))],
            "{name}"
        );
        assert_eq!(
            m.problems[0].text(),
            "song 001: the word after the key must be the tempo (beat 0)",
            "{name}"
        );
    }
    // A key that does not read stays its own problem, tempo or not.
    let m = build(
        &[cue("001 T {Amin cappella}", 0.0), cue("SONG END", 9.0)],
        &[],
    );
    assert_eq!(
        kinds(&m),
        [(ProblemKind::BadKey(KeyError::Suffix), Some(1), Some(0.0))]
    );
    assert_eq!(
        serde_json::to_value(ProblemKind::BadTempo).unwrap(),
        serde_json::json!("bad_tempo")
    );
}

#[test]
fn a_key_that_does_not_read_is_a_problem() {
    let m = build(&[cue("001 T {Amin 100}", 0.0), cue("SONG END", 9.0)], &[]);
    assert_eq!(m.spans[0].key, None);
    assert_eq!(
        kinds(&m),
        [(ProblemKind::BadKey(KeyError::Suffix), Some(1), Some(0.0))]
    );
    let m = build(&[cue("001 T {H 100}", 0.0), cue("SONG END", 9.0)], &[]);
    assert_eq!(
        kinds(&m),
        [(ProblemKind::BadKey(KeyError::Letter), Some(1), Some(0.0))]
    );
}

#[test]
fn a_key_alone_or_with_a_tempo_reads() {
    for (name, key) in [
        ("001 T {G}", major(7)),
        ("001 T {G 95.5 4/4 anything}", major(7)),
        (
            "001 T {Bbm 70}",
            Key {
                root: 10,
                minor: true,
            },
        ),
    ] {
        let m = build(&[cue(name, 0.0), cue("SONG END", 9.0)], &[]);
        assert_eq!(m.spans[0].key, Some(key), "{name}");
        assert!(m.problems.is_empty(), "{name}");
    }
    for name in ["001 T {G 95.}", "001 T {G .5}", "001 T {G 9x}"] {
        let m = build(&[cue(name, 0.0), cue("SONG END", 9.0)], &[]);
        assert_eq!(m.spans[0].key, None, "{name}");
        assert_eq!(
            kinds(&m),
            [(ProblemKind::BadTempo, Some(1), Some(0.0))],
            "{name}"
        );
    }
}

#[test]
fn a_song_without_a_description_has_no_key_and_no_problem() {
    let m = build(&[cue("088 Dance", 0.0), cue("SONG END", 30.0)], &[]);
    assert_eq!(m.spans.len(), 1);
    assert_eq!(m.spans[0].number, 88);
    assert_eq!(m.spans[0].key, None);
    assert_eq!(m.spans[0].end, Some(30.0));
    assert!(m.problems.is_empty());
}

#[test]
fn an_empty_or_unclosed_description_is_a_problem() {
    // Review M6: a song locator holding a `{` that opens no description is
    // a tag that does not read (spec §4), not a song without a key.
    // Round 2's M6b: its own problem, whose words say the braces are at
    // fault: "no key" was the wrong hint, the key may be there (`{C 120`).
    for name in [
        "001 T {}",
        "001 T { }",
        "001 T {C 120",
        "001 T {",
        "001 T } {C 120",
    ] {
        let m = build(&[cue(name, 0.0), cue("SONG END", 9.0)], &[]);
        assert_eq!(m.spans[0].key, None, "{name}");
        assert_eq!(m.problems.len(), 1, "{name}");
        let problem = &m.problems[0];
        assert_eq!(
            serde_json::to_value(problem.kind).unwrap(),
            serde_json::json!("bad_braces"),
            "{name}"
        );
        assert_eq!((problem.song, problem.time), (Some(1), Some(0.0)), "{name}");
        assert_eq!(
            problem.text(),
            "song 001: its {…} is empty or never closed (beat 0)",
            "{name}"
        );
    }
    // A `}` alone opens nothing: no tag, no problem.
    let m = build(&[cue("001 T C 120}", 0.0), cue("SONG END", 9.0)], &[]);
    assert_eq!(m.spans[0].key, None);
    assert!(m.problems.is_empty());
}

#[test]
fn two_descriptions_make_the_song_keyless() {
    let m = build(&[cue("001 T {C 100} {D}", 0.0), cue("SONG END", 9.0)], &[]);
    assert_eq!(m.spans[0].key, None);
    assert_eq!(
        kinds(&m),
        [(ProblemKind::TwoDescriptions, Some(1), Some(0.0))]
    );
}

#[test]
fn a_duplicate_number_makes_both_keyless() {
    let m = build(
        &[
            cue("005 One {C 100}", 0.0),
            cue("SONG END", 10.0),
            cue("006 Fine {D 100}", 20.0),
            cue("SONG END", 30.0),
            cue("005 Two {E 100}", 40.0),
            cue("SONG END", 50.0),
        ],
        &[],
    );
    let keys: Vec<(u16, Option<Key>)> = m.spans.iter().map(|s| (s.number, s.key)).collect();
    assert_eq!(keys, [(5, None), (6, Some(major(2))), (5, None)]);
    assert_eq!(
        kinds(&m),
        [
            (ProblemKind::DuplicateNumber, Some(5), Some(0.0)),
            (ProblemKind::DuplicateNumber, Some(5), Some(40.0))
        ]
    );
}

#[test]
fn two_locators_at_one_time_make_the_song_keyless() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("SONG END", 10.0),
            cue("002 B {D 100}", 10.0),
        ],
        &[],
    );
    assert_eq!(m.spans[0].key, None, "001's end is not certain");
    assert_eq!(m.spans[0].end, Some(10.0));
    assert!(
        m.problems
            .iter()
            .any(|p| p.kind == ProblemKind::SameTime && p.song == Some(1) && p.time == Some(10.0))
    );
    // 002 starts at that time: its entry's bounds are not certain either.
    assert!(
        m.problems
            .iter()
            .any(|p| p.kind == ProblemKind::SameTime && p.song == Some(2))
    );
    assert_eq!(m.spans[1].key, None);
}

#[test]
fn two_locators_at_one_time_reach_only_the_songs_there() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("SONG END", 10.0),
            cue("START", 15.0),
            cue("MODE 1", 15.0),
            cue("002 B {D 100}", 20.0),
            cue("SONG END", 30.0),
            cue("003 C {E 100}", 30.5),
            cue("SONG END", 40.0),
        ],
        &[],
    );
    assert_eq!(
        kinds(&m),
        [(ProblemKind::SameTime, None, Some(15.0))],
        "no song starts at or reaches beat 15"
    );
    let keys: Vec<Option<Key>> = m.spans.iter().map(|s| s.key).collect();
    assert_eq!(keys, [Some(major(0)), Some(major(2)), Some(major(4))]);
}

#[test]
fn a_tie_inside_a_song_makes_it_keyless_and_the_next_song_keeps_its_key() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("SONG END", 10.0),
            cue("002 B {D 100}", 20.0),
            cue("SONG END", 30.0),
            cue("START", 30.0),
            cue("003 C {E 100}", 40.0),
            cue("SONG END", 50.0),
        ],
        &[],
    );
    assert_eq!(
        kinds(&m),
        [(ProblemKind::SameTime, Some(2), Some(30.0))],
        "only 002 reaches beat 30"
    );
    let keys: Vec<Option<Key>> = m.spans.iter().map(|s| s.key).collect();
    assert_eq!(keys, [Some(major(0)), None, Some(major(4))]);
}

#[test]
fn other_entries_are_no_songs() {
    let m = build(
        &[
            cue("START", 0.0),
            cue("MODE 1", 5.0),
            cue("001 T {E 90}", 10.0),
            cue("SONG END", 20.0),
        ],
        &[],
    );
    assert_eq!(m.spans.len(), 1);
    assert_eq!(m.spans[0].number, 1);
    assert_eq!(m.spans[0].key, Some(major(4)));
    assert!(m.problems.is_empty());
}

#[test]
fn a_cue_without_a_time_is_left_out() {
    let m = build(
        &[
            cue("001 T {E 90}", 10.0),
            Cue {
                name: "002 U {F 90}".into(),
                time: None,
            },
            cue("SONG END", 20.0),
            Cue {
                name: "> Chorus".into(),
                time: None,
            },
        ],
        &[],
    );
    assert_eq!(m.spans.len(), 1);
    assert_eq!(m.spans[0].end, Some(20.0));
    assert!(m.problems.is_empty());
}

#[test]
fn key_clips_change_the_key_from_their_start() {
    let m = build(
        &[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
        &[clip("Verse", 22906.0), clip("Key {G}", 22897.0)],
    );
    assert_eq!(
        m.spans[0].changes,
        vec![KeyChange {
            time: 22897.0,
            key: major(7)
        }]
    );
    assert_eq!(key_at(&m, 22896.9), Some((31, major(5))));
    assert_eq!(key_at(&m, 22897.0), Some((31, major(7))));
    assert_eq!(key_at(&m, 23500.9), Some((31, major(7))));
    assert_eq!(key_at(&m, 23501.0), None);
    assert_eq!(key_at(&m, 22765.9), None);
    assert_eq!(key_at(&m, 22766.0), Some((31, major(5))));
    assert!(m.problems.is_empty());
}

#[test]
fn changes_are_in_time_order_and_each_holds_until_the_next() {
    let m = build(
        &[cue("133 T {G 120}", 75000.0), cue("SONG END", 76000.0)],
        &[clip("Key {A}", 75169.5), clip("Key {G#}", 75136.5)],
    );
    let times: Vec<f64> = m.spans[0].changes.iter().map(|c| c.time).collect();
    assert_eq!(times, [75136.5, 75169.5]);
    assert_eq!(key_at(&m, 75136.4), Some((133, major(7))));
    assert_eq!(key_at(&m, 75150.0), Some((133, major(8))));
    assert_eq!(key_at(&m, 75169.5), Some((133, major(9))));
}

#[test]
fn a_key_clip_at_the_span_start_applies_from_it() {
    let m = build(
        &[cue("001 T {C 100}", 100.0), cue("SONG END", 200.0)],
        &[clip("Key {D}", 100.0)],
    );
    assert_eq!(key_at(&m, 100.0), Some((1, major(2))));
    assert_eq!(
        m.spans[0].changes,
        [KeyChange {
            time: 100.0,
            key: major(2)
        }]
    );
    assert!(m.problems.is_empty());
}

#[test]
fn a_key_clip_at_a_song_end_belongs_to_no_span() {
    let m = build(
        &[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
        &[clip("Key {G}", 23501.0)],
    );
    assert!(m.spans[0].changes.is_empty());
    assert_eq!(
        kinds(&m),
        [(ProblemKind::ClipOutsideSong, None, Some(23501.0))]
    );
}

#[test]
fn a_key_clip_before_a_song_or_in_an_unended_one_belongs_to_no_span() {
    let m = build(
        &[
            cue("001 T {C 100}", 100.0),
            cue("SONG END", 200.0),
            cue("002 U {D 100}", 300.0),
        ],
        &[clip("Key {E}", 50.0), clip("Key {F}", 350.0)],
    );
    assert_eq!(
        kinds(&m),
        [
            (ProblemKind::NotEnded, Some(2), Some(300.0)),
            (ProblemKind::ClipOutsideSong, None, Some(50.0)),
            (ProblemKind::ClipOutsideSong, None, Some(350.0))
        ]
    );
    assert!(m.spans.iter().all(|s| s.changes.is_empty()));
}

#[test]
fn a_clip_with_more_words_after_the_key_is_refused() {
    let m = build(
        &[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
        &[
            clip("Chorus {G loud}", 22900.0),
            clip("Late {G loud}", 30000.0),
        ],
    );
    assert!(m.spans[0].changes.is_empty());
    assert_eq!(key_at(&m, 22950.0), Some((31, major(5))));
    assert_eq!(
        kinds(&m),
        [
            (ProblemKind::BadClipKey, Some(31), Some(22900.0)),
            (ProblemKind::BadClipKey, None, Some(30000.0))
        ]
    );
    // The words name the clip and its song.
    let texts: Vec<String> = m.problems.iter().map(Problem::text).collect();
    assert_eq!(
        texts,
        [
            "song 031: the Sections clip \"Chorus {G loud}\" does not read as one key (beat 22900)",
            "the Sections clip \"Late {G loud}\" does not read as one key (beat 30000)"
        ]
    );
}

#[test]
fn a_clips_problem_carries_its_trimmed_name_and_a_locators_none() {
    let m = build(
        &[
            cue("001 T {C 100}", 0.0),
            cue("SONG END", 100.0),
            cue("002 U", 200.0),
            cue("SONG END", 300.0),
            cue("003 V {D 100}", 400.0),
        ],
        &[clip("  Key {E}  ", 150.0), clip("Key {F}", 250.0)],
    );
    let clips: Vec<(ProblemKind, Option<&str>)> = m
        .problems
        .iter()
        .map(|p| (p.kind, p.clip.as_deref()))
        .collect();
    assert_eq!(
        clips,
        [
            (ProblemKind::NotEnded, None),
            (ProblemKind::ClipOutsideSong, Some("Key {E}")),
            (ProblemKind::ClipInKeylessSong, Some("Key {F}"))
        ]
    );
    let texts: Vec<String> = m.problems.iter().map(Problem::text).collect();
    assert_eq!(
        texts,
        [
            "song 003: not ended by SONG END, STOP or AUTOSTOP (beat 400)",
            "the key clip \"Key {E}\" lies outside every ended song (beat 150)",
            "song 002: the key clip \"Key {F}\" lies in a song without a key (beat 250)"
        ]
    );
}

#[test]
fn a_clip_whose_description_is_no_key_is_no_key_clip() {
    let m = build(
        &[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
        &[
            clip("Verse {soft}", 22800.0),
            clip("Bridge {Amin}", 22810.0),
            clip("Plain", 22820.0),
            clip("Empty {}", 22830.0),
            ClipName {
                name: "Key {D}".into(),
                start: None,
            },
        ],
    );
    assert!(m.spans[0].changes.is_empty());
    // `Amin` is no word but a mistyped key: spec Appendix A.3 lists `min`
    // under "Rejected, as a problem", so it is never a silent no-key clip
    // (review I2). The others stay no key clip, with no problem.
    assert_eq!(
        kinds(&m),
        [(ProblemKind::BadClipKey, Some(31), Some(22810.0))]
    );
}

/// One song (031 in F, beats 22766 to 23501) with one clip at beat 22800.
fn one_clip(name: &str) -> SongMap {
    build(
        &[cue("031 T {F 95 4/4}", 22766.0), cue("SONG END", 23501.0)],
        &[clip(name, 22800.0)],
    )
}

#[test]
fn a_clip_named_key_whose_key_does_not_read_is_a_problem() {
    // The owner names every key clip `Key {X}` (spec §2.1): one whose key
    // does not read is a mistyped key, never a silent no-key clip (review
    // I2 a). The song keeps its own key.
    for name in [
        "Key {Amin}",
        "Key {Gmaj}",
        "Key {H}",
        "Key {g}",
        "Key {E#}",
        "Key {F♯}",
        "Key {Am/A}",
        "Key {Fis}",
        "Key {soft}",
        "Key {Amin loud}",
        "Key {}",
        "Key { }",
        "Key {G",
        "  Key{Cmoll}  ",
        "{soft} Key",
    ] {
        let m = one_clip(name);
        assert!(m.spans[0].changes.is_empty(), "{name}");
        assert_eq!(
            kinds(&m),
            [(ProblemKind::BadClipKey, Some(31), Some(22800.0))],
            "{name}"
        );
        assert_eq!(key_at(&m, 22900.0), Some((31, major(5))), "{name}");
    }
    assert_eq!(
        one_clip("Key {Amin}").problems[0].text(),
        "song 031: the Sections clip \"Key {Amin}\" does not read as one key (beat 22800)"
    );
}

#[test]
fn a_one_word_description_spelled_as_a_wrong_key_is_a_problem() {
    // Review I2 b: an accidental, two keys, a known wrong suffix, `H` or a
    // lower-case key letter first, whatever the clip's name.
    for name in [
        "Bridge {Amin}",
        "Bridge {Gmaj}",
        "Bridge {Amol}",
        "Bridge {Amoll}",
        "Bridge {Adur}",
        "Bridge {AM}",
        "Bridge {F♯}",
        "Bridge {B♭}",
        "Bridge {F♯m}",
        "Bridge {Ebmaj}",
        "Intro {E#}",
        "Intro {Cb}",
        "Intro {Fbm}",
        "Tag {Am/A}",
        "Tag {F/G}",
        "Chorus {H}",
        "Chorus {Hm}",
        "Chorus {Hmoll}",
        "Verse {g}",
        "Verse {f#m}",
        "Verse {am}",
        "Verse {cis}",
        "Verse {es}",
        "Verse {h}",
        "key {Amin}",
    ] {
        let m = one_clip(name);
        assert!(m.spans[0].changes.is_empty(), "{name}");
        assert_eq!(
            kinds(&m),
            [(ProblemKind::BadClipKey, Some(31), Some(22800.0))],
            "{name}"
        );
    }
    // Round 2's C1: a lower-case or `H` first letter makes a problem only
    // of a key spelled so; a plain word is no attempt at a key (spec A.3
    // rejects lower case inside a key token).
    assert!(one_clip("Pad {bridge}").problems.is_empty());
}

/// Whether a clip's description makes it a bad key clip of song 031.
fn bad_clip(name: &str) -> bool {
    kinds(&one_clip(name)) == [(ProblemKind::BadClipKey, Some(31), Some(22800.0))]
}

#[test]
fn a_key_spelled_in_lower_case_or_with_h_is_a_problem_a_word_is_not() {
    // Round 2's C1: a word whose first letter, upper-cased (`h`/`H` read as
    // `B`), gives a key or a mistyped key is a problem; any other word is
    // none, whatever its first letter.
    for word in ["g", "am", "f#m", "bb", "ebm", "amin", "h", "hm", "Hmoll"] {
        assert!(bad_clip(&format!("Pad {{{word}}}")), "{word}");
    }
    for word in ["bridge", "build", "drums", "fade", "hold", "Hook", "Hymn"] {
        let m = one_clip(&format!("Pad {{{word}}}"));
        assert!(m.problems.is_empty(), "{word}: {:?}", m.problems);
    }
}

#[test]
fn a_german_spelling_is_a_mistyped_key_on_any_clip() {
    // Round 2's C2: spec A.3 rejects Fis and Es, as a problem.
    for word in ["Fis", "Es", "Cis", "As", "Gism", "fis", "es"] {
        assert!(bad_clip(&format!("Pad {{{word}}}")), "{word}");
        assert!(bad_clip(&format!("Key {{{word}}}")), "{word}");
    }
}

#[test]
fn a_mistyped_first_word_or_spelling_half_is_a_problem() {
    // Round 2's C2b: more words after a mistyped key, or a double spelling
    // with a mistyped half, is a problem on any clip, as `{G loud}` is.
    for description in [
        "Amin loud",
        "Amin/C",
        "F♯m/A",
        "C/Amin",
        "g loud",
        "Fis 2",
        "F#/H",
    ] {
        assert!(bad_clip(&format!("Pad {{{description}}}")), "{description}");
    }
    // Words stay words, alone or with more after them.
    for description in [
        "Bridge 2",
        "Chorus 2",
        "soft and slow",
        "hold on",
        "Hook/Verse",
    ] {
        let m = one_clip(&format!("Pad {{{description}}}"));
        assert!(m.problems.is_empty(), "{description}: {:?}", m.problems);
    }
}

#[test]
fn words_and_longer_descriptions_stay_no_key_clips() {
    // {Verse}, {soft}, {Bridge} and {Chorus 2} are words (review I2's
    // ruling); a name in another case than `Key` is no key clip's name, and
    // a description of two words is no mistyped key.
    for name in [
        "Verse",
        "x {Verse}",
        "Break {soft}",
        "x {Bridge}",
        "y {Chorus 2}",
        "KEY {soft}",
        "key {soft}",
        "Keys {soft}",
        "Pad {Bridge 2}",
        "Empty {}",
        "Open {G",
    ] {
        let m = one_clip(name);
        assert!(m.spans[0].changes.is_empty(), "{name}");
        assert!(m.problems.is_empty(), "{name}: {:?}", m.problems);
    }
    // Round 2's C2b: a first word that is a mistyped key is a problem with
    // more words after it, as `{G loud}` is (spec A.3).
    assert_eq!(
        kinds(&one_clip("Pad {Amin loud}")),
        [(ProblemKind::BadClipKey, Some(31), Some(22800.0))]
    );
}

#[test]
fn a_clip_is_named_key_by_its_words_outside_the_braces() {
    for name in [
        "Key {G}",
        "Key{G}",
        "Key  {G}",
        "{G} Key",
        "Key {}",
        "Key {G",
        "Key {x} {y}",
    ] {
        assert!(named_key(name), "{name}");
    }
    for name in [
        "Key",
        "key {G}",
        "KEY {G}",
        "Keys {G}",
        "Key 2 {G}",
        "Key {G} x",
        "x {G",
    ] {
        assert!(!named_key(name), "{name}");
    }
}

#[test]
fn a_mistyped_key_is_told_from_a_word_by_how_it_fails() {
    for (word, error) in [
        ("E#", KeyError::Accidental),
        ("Am/A", KeyError::TwoKeys),
        ("Amin", KeyError::Suffix),
        ("AM", KeyError::Suffix),
        ("F♯", KeyError::Suffix),
        ("F♯m", KeyError::Suffix),
        ("Amolm", KeyError::Suffix),
        // Round 2's C2: spec A.3 rejects Fis and Es, as a problem.
        ("Fis", KeyError::Suffix),
        ("H", KeyError::Letter),
        ("h", KeyError::Letter),
        ("am", KeyError::Letter),
    ] {
        assert_eq!(parse_token(word), Err(error), "{word}");
        assert!(mistyped(word, error), "{word}");
    }
    for (word, error) in [
        ("Bridge", KeyError::Suffix),
        ("Amm", KeyError::Suffix),
        ("AMEN", KeyError::Suffix),
        ("Verse", KeyError::Letter),
        ("soft", KeyError::Letter),
        ("Intro", KeyError::Letter),
        ("", KeyError::Empty),
    ] {
        assert_eq!(parse_token(word), Err(error), "{word}");
        assert!(!mistyped(word, error), "{word}");
    }
    // Round 2: the German `is`, `es` and `s` join them (C2); no list of
    // letters any more, the letter rule reads the word itself (C1).
    assert_eq!(
        WRONG_SUFFIXES[..],
        [
            "min", "maj", "mol", "moll", "dur", "M", "♯", "♭", "is", "es", "s"
        ]
    );
}

#[test]
fn a_first_letter_is_read_as_a_key_letter_only_for_h_and_lower_case() {
    assert_eq!(letter_fixed("h"), Some("B".to_string()));
    assert_eq!(letter_fixed("Hmoll"), Some("Bmoll".to_string()));
    assert_eq!(letter_fixed("am"), Some("Am".to_string()));
    assert_eq!(letter_fixed("f#m"), Some("F#m".to_string()));
    assert_eq!(letter_fixed("bridge"), Some("Bridge".to_string()));
    assert_eq!(letter_fixed("g"), Some("G".to_string()));
    for spelling in ["", "A", "G#", "Verse", "i", "x", "soft"] {
        assert_eq!(letter_fixed(spelling), None, "{spelling}");
    }
}

#[test]
fn a_spelling_is_wrong_by_its_root_or_by_its_fixed_letter() {
    // Its root: an accidental that is no key, or a wrong suffix.
    for spelling in [
        "E#", "Cb", "Fbm", "Amin", "AM", "F♯m", "Fis", "Es", "Gism", "Amolm",
    ] {
        assert!(wrong_root(spelling), "{spelling}");
        assert!(spelled_wrong(spelling), "{spelling}");
    }
    for spelling in [
        "", "C", "Cm", "F#", "Bridge", "Amm", "AMEN", "Verse", "Hook",
    ] {
        assert!(!wrong_root(spelling), "{spelling}");
    }
    // A fixed first letter: a key, or a wrong root, once it is fixed.
    for spelling in [
        "g", "am", "bb", "ebm", "h", "hm", "Hmoll", "amin", "fis", "e#",
    ] {
        assert!(spelled_wrong(spelling), "{spelling}");
    }
    for spelling in [
        "bridge", "build", "drums", "fade", "hold", "Hook", "Hymn", "C", "Verse", "x",
    ] {
        assert!(!spelled_wrong(spelling), "{spelling}");
    }
    // A double spelling: two keys, or a half spelled wrong.
    assert!(mistyped("F/G", KeyError::TwoKeys));
    assert!(mistyped("C/Amin", KeyError::Suffix));
    assert!(mistyped("F#/H", KeyError::Letter));
    assert!(!mistyped("Hook/Verse", KeyError::Letter));
}

#[test]
fn a_key_clip_in_a_keyless_song_stays_keyless() {
    let m = build(
        &[cue("088 Dance", 0.0), cue("SONG END", 30.0)],
        &[clip("Key {G}", 10.0)],
    );
    assert_eq!(key_at(&m, 10.0), None);
    assert!(m.spans[0].changes.is_empty());
    assert_eq!(
        kinds(&m),
        [(ProblemKind::ClipInKeylessSong, Some(88), Some(10.0))]
    );
}

#[test]
fn two_key_clips_at_one_time_take_the_later_in_clip_order() {
    let m = build(
        &[cue("001 T {C 100}", 0.0), cue("SONG END", 100.0)],
        &[
            clip("Key {D}", 50.0),
            clip("Key {E}", 50.0),
            clip("Key {F}", 70.0),
        ],
    );
    assert_eq!(
        m.spans[0].changes,
        [
            KeyChange {
                time: 50.0,
                key: major(4)
            },
            KeyChange {
                time: 70.0,
                key: major(5)
            }
        ]
    );
    assert_eq!(kinds(&m), [(ProblemKind::SameTime, Some(1), Some(50.0))]);
}

#[test]
fn half_beat_changes_are_kept_exactly() {
    let m = build(
        &[cue("153 T {Am 70}", 86900.0), cue("SONG END", 87100.0)],
        &[clip("Key {Fm}", 86963.5), clip("Key {Dm}", 86990.5)],
    );
    let times: Vec<f64> = m.spans[0].changes.iter().map(|c| c.time).collect();
    assert_eq!(times, [86963.5, 86990.5]);
    assert_eq!(
        key_at(&m, 86963.5),
        Some((
            153,
            Key {
                root: 5,
                minor: true
            }
        ))
    );
    assert_eq!(
        key_at(&m, 86990.5),
        Some((
            153,
            Key {
                root: 2,
                minor: true
            }
        ))
    );
}

#[test]
fn counts_sum_up_the_map() {
    let m = build(
        &[
            cue("001 A {C 100}", 0.0),
            cue("SONG END", 10.0),
            cue("002 B", 20.0),
            cue("SONG END", 30.0),
            cue("003 C {E 100}", 40.0),
            cue("SONG END", 50.0),
        ],
        &[clip("Key {G}", 5.0), clip("Key {D}", 25.0)],
    );
    assert_eq!(
        counts(&m),
        Counts {
            songs: 3,
            keyed: 2,
            changes: 1,
            problems: 1
        }
    );
    assert_eq!(counts(&SongMap::default()), Counts::default());
}

#[test]
fn classify_reads_each_kind() {
    assert_eq!(classify("> Chorus"), CueKind::Section);
    assert_eq!(classify("* TS"), CueKind::Ignored);
    assert_eq!(classify("() x"), CueKind::Ignored);
    assert_eq!(classify("[x] clyphx"), CueKind::Ignored);
    assert_eq!(classify("SONGEND"), CueKind::SongEnd);
    assert_eq!(classify("SONG END"), CueKind::SongEnd);
    assert_eq!(classify("AUTOSTOP"), CueKind::Stop);
    assert_eq!(classify("STOP"), CueKind::Stop);
    assert_eq!(
        classify(". 012 Late {G 80}"),
        CueKind::Song {
            number: 12,
            description: Some("G 80".into())
        }
    );
    assert_eq!(classify("START"), CueKind::Other);
    assert!(!needs_time(&CueKind::Section) && needs_time(&CueKind::Other));
}

#[test]
fn an_end_is_read_without_its_description_flags_tags_and_colours() {
    assert_eq!(classify("SONG END {note}"), CueKind::SongEnd);
    assert_eq!(classify("SONG END #outro [red]"), CueKind::SongEnd);
    assert_eq!(classify("  STOP +FADE  "), CueKind::Stop);
    assert_eq!(classify("{x} AUTOSTOP"), CueKind::Stop);
    assert_eq!(classify("SONG  END"), CueKind::SongEnd);
    assert_eq!(classify("song end"), CueKind::Other);
    assert_eq!(classify("END"), CueKind::Other);
    // A stop flag makes any locator a stop, a song's too.
    assert_eq!(classify("010 T {C 90} +JUMP"), CueKind::Stop);
    assert_eq!(classify("MODE +STAY"), CueKind::Stop);
    assert_eq!(classify("MODE +STAYS"), CueKind::Other);
}

#[test]
fn a_song_is_three_digits_and_a_space() {
    assert_eq!(
        classify("001 T"),
        CueKind::Song {
            number: 1,
            description: None
        }
    );
    assert_eq!(
        classify(".031 T {F}"),
        CueKind::Song {
            number: 31,
            description: Some("F".into())
        }
    );
    assert_eq!(
        classify("999 Z"),
        CueKind::Song {
            number: 999,
            description: None
        }
    );
    for other in [
        "001",
        "0012 Title",
        "01 Short",
        "AB1 x",
        "x001 T",
        "…",
        "",
        "1.5 T",
    ] {
        assert_eq!(classify(other), CueKind::Other, "{other:?}");
    }
}

#[test]
fn ignored_names_need_their_mark_first() {
    assert_eq!(classify("[]x"), CueKind::Other, "nothing in the brackets");
    assert_eq!(classify("x [y] 001 T"), CueKind::Other);
    assert_eq!(classify("(x) T"), CueKind::Other);
    assert_eq!(classify("  *001 T"), CueKind::Ignored);
    assert!(!needs_time(&CueKind::Ignored));
    for kind in [
        CueKind::SongEnd,
        CueKind::Stop,
        CueKind::Song {
            number: 1,
            description: None,
        },
    ] {
        assert!(needs_time(&kind), "{kind:?}");
    }
}

#[test]
fn a_description_is_between_the_first_brace_and_the_last() {
    assert_eq!(description("001 T {C 120}"), Some("C 120"));
    assert_eq!(description("001 T {C} {D}"), Some("C} {D"));
    assert_eq!(description("x {}"), None);
    assert_eq!(description("x {a"), None);
    assert_eq!(description("x } {"), None);
    assert_eq!(description("{a}"), Some("a"));
    assert!(clip_needs_time("Key {G}"));
    assert!(clip_needs_time("x {"));
    assert!(!clip_needs_time("Verse"));
}

#[test]
fn each_problem_reads_in_words() {
    let text = |kind, song, time| {
        Problem {
            kind,
            song,
            time,
            clip: None,
        }
        .text()
    };
    assert_eq!(
        text(ProblemKind::SameTime, Some(31), Some(22766.0)),
        "song 031: two locators or key clips at one time (beat 22766)"
    );
    assert_eq!(
        text(ProblemKind::DuplicateNumber, Some(5), Some(0.5)),
        "song 005: its number is on another song too (beat 0.5)"
    );
    assert_eq!(
        text(ProblemKind::TwoDescriptions, Some(1), None),
        "song 001: its locator holds two {…}"
    );
    assert_eq!(
        text(ProblemKind::BadKey(KeyError::Suffix), Some(12), Some(3.0)),
        "song 012: its key does not read: only m may follow the key (beat 3)"
    );
    assert_eq!(
        text(ProblemKind::BadBraces, Some(12), None),
        "song 012: its {…} is empty or never closed"
    );
    assert_eq!(
        serde_json::to_value(ProblemKind::BadBraces).unwrap(),
        serde_json::json!("bad_braces")
    );
    assert_eq!(
        text(ProblemKind::NotEnded, Some(158), Some(89000.0)),
        "song 158: not ended by SONG END, STOP or AUTOSTOP (beat 89000)"
    );
    // A clip's problem without the clip's name.
    assert_eq!(
        text(ProblemKind::ClipOutsideSong, None, Some(75136.5)),
        "a key clip lies outside every ended song (beat 75136.5)"
    );
    assert_eq!(
        text(ProblemKind::BadClipKey, Some(31), Some(1.0)),
        "song 031: a Sections clip does not read as one key (beat 1)"
    );
    assert_eq!(
        text(ProblemKind::ClipInKeylessSong, Some(88), None),
        "song 088: a key clip lies in a song without a key"
    );
    // With it: in quotes.
    let named = Problem {
        kind: ProblemKind::BadClipKey,
        song: Some(31),
        time: None,
        clip: Some("Key {G loud}".into()),
    };
    assert_eq!(
        named.text(),
        "song 031: the Sections clip \"Key {G loud}\" does not read as one key"
    );
}

#[test]
fn the_map_serializes_in_english_words() {
    let m = build(
        &[cue("001 T {Am 90}", 0.0), cue("SONG END", 10.0)],
        &[clip("Key {C}", 5.0), clip("Key {D}", 50.0)],
    );
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "spans": [{"number": 1, "start": 0.0, "end": 10.0,
                       "key": {"root": 9, "minor": true},
                       "changes": [{"time": 5.0, "key": {"root": 0, "minor": false}}]}],
            "problems": [{"kind": "clip_outside_song", "song": null, "time": 50.0,
                          "clip": "Key {D}"}]
        })
    );
    // A problem without its clip (an older hub's) reads as a locator's.
    let older: Problem =
        serde_json::from_value(serde_json::json!({"kind": "not_ended", "song": 1, "time": 0.0}))
            .unwrap();
    assert_eq!(older.clip, None);
    assert_eq!(
        serde_json::to_value(ProblemKind::BadKey(KeyError::Letter)).unwrap(),
        serde_json::json!({"bad_key": "letter"})
    );
    assert_eq!(
        serde_json::to_value(SectionsState::FirstOf(2)).unwrap(),
        serde_json::json!({"first_of": 2})
    );
    assert_eq!(
        serde_json::to_value(SectionsState::None).unwrap(),
        serde_json::json!("none")
    );
    // #140 review M9: a Sections track whose clips cannot be read.
    assert_eq!(
        serde_json::to_value(SectionsState::Unreadable).unwrap(),
        serde_json::json!("unreadable")
    );
    let unreadable = Problem {
        kind: ProblemKind::SectionsUnreadable,
        song: None,
        time: None,
        clip: None,
    };
    assert_eq!(
        serde_json::to_value(unreadable.kind).unwrap(),
        serde_json::json!("sections_unreadable")
    );
    assert_eq!(
        unreadable.text(),
        "the clips of the track named Sections cannot be read: no key clip applies"
    );
    assert_eq!(TIME_CHUNK, 200);
}
