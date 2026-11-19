use super::*;

/// A column with the set's transport `p` and song `s` (no reading).
fn col(p: Option<bool>, s: Option<u16>) -> Column {
    Column {
        v: None,
        f: None,
        p,
        s,
    }
}

const PLAY: Option<bool> = Some(true);
const STOP: Option<bool> = Some(false);

fn names() -> BTreeMap<u16, String> {
    BTreeMap::from([
        (31, "Morning Light".to_string()),
        (57, "Quiet Hill".to_string()),
        (88, String::new()),
    ])
}

#[test]
fn the_constants_are_the_mockups() {
    assert_eq!(LABEL_MIN_PX, 40.0);
    assert_eq!(LABEL_PAD_PX, 4.0);
    assert_eq!(LABEL_FONT_PX, 11.0);
    assert_eq!(LABEL_SPACING_EM, 0.04);
    assert_eq!(TIME_LEFT_PX, 10.0);
    assert_eq!(TIME_RIGHT_PX, 32.0);
}

#[test]
fn a_texts_width_is_the_faces_advances_and_spacing() {
    // Space 200, `0` 453, `A` 482, `~` 490 units; 11 px; 0.44 px spacing.
    let px = |units: f64| units * 11.0 / 1000.0 + 0.44;
    assert!((char_px(' ') - px(200.0)).abs() < 1e-9);
    assert!((char_px('0') - px(453.0)).abs() < 1e-9);
    assert!((char_px('A') - px(482.0)).abs() < 1e-9);
    assert!((char_px('~') - px(490.0)).abs() < 1e-9);
    assert!((char_px('\u{2026}') - px(709.0)).abs() < 1e-9);
    // Anything else: the widest letter's.
    assert!((char_px('é') - px(689.0)).abs() < 1e-9);
    assert!((char_px('\u{7f}') - px(689.0)).abs() < 1e-9);
    assert!((text_px("A0") - px(482.0) - px(453.0)).abs() < 1e-9);
    assert_eq!(text_px(""), 0.0);
    // `031 Morning Light`, as the mockup's label measured about 80 px.
    let w = text_px("031 Morning Light");
    assert!((75.0..90.0).contains(&w), "{w}");
}

#[test]
fn a_label_is_cut_with_an_ellipsis_to_its_room_or_left_out() {
    let text = "031 Morning Light";
    let whole = text_px(text);
    assert_eq!(cut_text(text, whole).as_deref(), Some(text));
    assert_eq!(cut_text(text, 500.0).as_deref(), Some(text));
    // A little short: the longest head that fits with its ellipsis, its
    // trailing space dropped.
    let cut = cut_text(text, whole - 0.01).unwrap();
    assert_eq!(cut, "031 Morning Lig\u{2026}");
    assert!(text_px(&cut) <= whole - 0.01);
    let room = text_px("031 \u{2026}");
    assert_eq!(cut_text(text, room).as_deref(), Some("031\u{2026}"));
    assert_eq!(cut_text(text, room - 0.01).as_deref(), Some("031\u{2026}"));
    // Exactly one character and the ellipsis.
    let one = text_px("0\u{2026}");
    assert_eq!(cut_text(text, one).as_deref(), Some("0\u{2026}"));
    assert_eq!(cut_text(text, one - 0.01), None);
    assert_eq!(cut_text(text, 0.0), None);
    assert_eq!(cut_text("", 0.0).as_deref(), Some(""));
}

#[test]
fn a_song_is_named_by_its_three_digit_number_and_its_name() {
    let names = names();
    assert_eq!(song_text(31, &names), "031 Morning Light");
    assert_eq!(song_text(57, &names), "057 Quiet Hill");
    assert_eq!(song_text(88, &names), "088", "an empty name");
    assert_eq!(song_text(7, &names), "007", "no name known");
    assert_eq!(song_text(204, &BTreeMap::new()), "204");
}

#[test]
fn the_lane_is_the_known_and_the_playing_runs() {
    let cols = [
        col(None, None),
        col(STOP, None),
        col(PLAY, Some(31)),
        col(PLAY, Some(31)),
        col(None, None),
        col(PLAY, None),
        col(STOP, None),
    ];
    assert_eq!(known_runs(&cols), vec![(1, 4), (5, 7)]);
    assert_eq!(play_runs(&cols), vec![(2, 4), (5, 6)]);
    // A run to the window's end; nothing at all.
    let end = [col(STOP, None), col(PLAY, Some(31))];
    assert_eq!(known_runs(&end), vec![(0, 2)]);
    assert_eq!(play_runs(&end), vec![(1, 2)]);
    assert!(play_runs(&[col(None, None)]).is_empty());
    assert!(known_runs(&[]).is_empty());
    assert_eq!(runs(&cols, |c| c.p.is_none()), vec![(0, 1), (4, 5)]);
}

#[test]
fn a_songs_span_is_its_playing_columns_of_one_song() {
    let cols = [
        col(PLAY, Some(31)),
        col(PLAY, Some(31)),
        col(PLAY, Some(57)),
        col(STOP, Some(57)),
        col(PLAY, Some(57)),
        col(PLAY, None),
        col(PLAY, Some(57)),
        col(None, Some(57)),
    ];
    assert_eq!(
        song_spans(&cols),
        vec![(0, 2, 31), (2, 3, 57), (4, 5, 57), (6, 7, 57)]
    );
    assert!(song_spans(&[]).is_empty());
}

#[test]
fn a_songs_mark_has_its_line_and_its_name_cut_to_its_span() {
    let names = names();
    // 120 columns over 600 px: 5 px a column.
    let mut cols = vec![col(STOP, None); 120];
    for c in &mut cols[0..30] {
        *c = col(PLAY, Some(31));
    }
    for c in &mut cols[40..47] {
        *c = col(PLAY, Some(57));
    }
    for c in &mut cols[50..58] {
        *c = col(PLAY, Some(88));
    }
    for c in &mut cols[60..80] {
        *c = col(PLAY, Some(57));
    }
    let marks = song_marks(&cols, &names, 600.0);
    assert_eq!(marks.len(), 4);
    // Begun before the window: named at the left edge, no line.
    assert_eq!(marks[0].from, 0);
    assert_eq!(marks[0].to, 30);
    assert_eq!(marks[0].song, 31);
    assert_eq!(marks[0].line, Line::None);
    assert_eq!(marks[0].label.as_deref(), Some("031 Morning Light"));
    assert_eq!(marks[0].room, 142.0);
    // 35 px: too narrow, a faint line and no name.
    assert_eq!(marks[1].line, Line::Faint);
    assert_eq!(marks[1].label, None);
    assert_eq!(marks[1].room, 27.0);
    // 40 px exactly: wide enough; its number alone fits 32 px of room.
    assert_eq!(marks[2].line, Line::Full);
    assert_eq!(marks[2].label.as_deref(), Some("088"));
    assert_eq!(marks[2].room, 32.0);
    // 100 px: `057 Quiet Hill` whole.
    assert_eq!(marks[3].line, Line::Full);
    assert_eq!(marks[3].label.as_deref(), Some("057 Quiet Hill"));
    // A narrower plot cuts the long name with an ellipsis.
    let cut = song_marks(&cols, &names, 300.0);
    assert_eq!(cut[0].room, 67.0);
    let label = cut[0].label.clone().unwrap();
    assert!(
        label.ends_with('\u{2026}') && label.starts_with("031 Morn"),
        "{label}"
    );
    assert!(text_px(&label) <= 67.0);
    // 120 columns over no width at all (not measured): nothing is named.
    assert!(
        song_marks(&cols, &names, 0.0)
            .iter()
            .all(|m| m.label.is_none())
    );
    assert!(song_marks(&[], &names, 600.0).is_empty());
    assert_eq!(Line::None.name(), "none");
    assert_eq!(Line::Faint.name(), "faint");
    assert_eq!(Line::Full.name(), "full");
}

#[test]
fn the_narrowest_named_span_cuts_its_name_to_32_px() {
    let cols = vec![col(PLAY, Some(31)); 1];
    let marks = song_marks(&cols, &names(), 40.0);
    assert_eq!(marks[0].room, 32.0);
    assert_eq!(marks[0].label.as_deref(), Some("031 M\u{2026}"));
}

#[test]
fn a_run_is_placed_by_its_column_edges() {
    assert_eq!(column_edge(0, 120), 0.0);
    assert_eq!(column_edge(60, 120), 500.0);
    assert_eq!(column_edge(120, 120), 1000.0);
    assert_eq!(column_edge(3, 0), 3000.0);
    assert_eq!(run_style(30, 60, 120), "left:25.000%;width:25.000%");
    assert_eq!(run_style(0, 120, 120), "left:0.000%;width:100.000%");
}

#[test]
fn a_time_label_gives_way_to_the_corners_names() {
    // A 512 px plot: shown from past 10 px to under 480 px.
    assert!(!time_label_shows(0.0, 512.0));
    assert!(!time_label_shows(10.0 / 512.0, 512.0));
    assert!(time_label_shows(11.0 / 512.0, 512.0));
    assert!(time_label_shows(0.5, 512.0));
    assert!(time_label_shows(479.0 / 512.0, 512.0));
    assert!(!time_label_shows(480.0 / 512.0, 512.0));
    assert!(!time_label_shows(1.0, 512.0));
    // Not measured yet: all show.
    assert!(time_label_shows(0.0, 0.0));
    assert!(time_label_shows(1.0, -1.0));
}

#[test]
fn the_cursor_says_play_with_the_song_or_stop() {
    let names = names();
    assert_eq!(
        transport_text(col(PLAY, Some(31)), &names),
        "\u{b7} PLAY 031 Morning Light"
    );
    assert_eq!(
        transport_text(col(PLAY, Some(7)), &names),
        "\u{b7} PLAY 007"
    );
    assert_eq!(transport_text(col(PLAY, None), &names), "\u{b7} PLAY");
    assert_eq!(transport_text(col(STOP, Some(31)), &names), "\u{b7} STOP");
    assert_eq!(transport_text(col(STOP, None), &names), "\u{b7} STOP");
    assert_eq!(transport_text(col(None, Some(31)), &names), "");
}

/// AUTOTUNE's state with the set's transport and song.
fn autotune(playing: Option<bool>, song: Option<u16>) -> AutotuneState {
    serde_json::from_value(serde_json::json!({
        "aut": false,
        "playing": playing,
        "targets": [],
        "song": song,
    }))
    .unwrap()
}

/// The recording watch's state with the band's transport.
fn rec(playing: Option<bool>) -> RecState {
    RecState {
        playing,
        record_mode: None,
        tracks: Vec::new(),
    }
}

#[test]
fn the_live_set_comes_from_autotune_for_its_set_else_from_the_recording_watch() {
    let at = autotune(Some(true), Some(31));
    // AUTOTUNE's set: its transport and song.
    assert_eq!(
        live_set("band", Some(("band", &at)), None),
        LiveSet {
            playing: Some(true),
            song: Some(Some(31)),
        }
    );
    // Out of every song is a song known live.
    let out = autotune(Some(true), None);
    assert_eq!(
        live_set("band", Some(("band", &out)), None).song,
        Some(None)
    );
    // AUTOTUNE's transport not heard yet: the recording watch's.
    let unheard = autotune(None, None);
    assert_eq!(
        live_set("band", Some(("band", &unheard)), Some(&rec(Some(false)))),
        LiveSet {
            playing: Some(false),
            song: Some(None),
        }
    );
    // AUTOTUNE's wins over the watch's.
    assert_eq!(
        live_set("band", Some(("band", &at)), Some(&rec(Some(false)))).playing,
        Some(true)
    );
    // Another set's AUTOTUNE: the watch's transport only, no song.
    assert_eq!(
        live_set("band", Some(("mbc", &at)), Some(&rec(Some(false)))),
        LiveSet {
            playing: Some(false),
            song: None,
        }
    );
    // The watch is the band's only.
    assert_eq!(
        live_set("mbc", None, Some(&rec(Some(true)))),
        LiveSet::default()
    );
    assert_eq!(live_set("band", None, None), LiveSet::default());
    assert_eq!(live_set("band", None, Some(&rec(None))), LiveSet::default());
}

#[test]
fn the_live_edge_takes_the_live_transport_and_song_else_the_series_last() {
    let heard = |playing, song| LiveSet { playing, song };
    // A start heard live while the series ended stopped.
    assert_eq!(
        edge_set(heard(Some(true), Some(Some(57))), (Some(false), None)),
        (Some(true), Some(57))
    );
    // A stop heard live: no song.
    assert_eq!(
        edge_set(heard(Some(false), Some(Some(57))), (Some(true), Some(31))),
        (Some(false), None)
    );
    // Playing out of every song, heard live.
    assert_eq!(
        edge_set(heard(Some(true), Some(None)), (Some(true), Some(31))),
        (Some(true), None)
    );
    // The transport live, the song not: the series' last song while it played.
    assert_eq!(
        edge_set(heard(Some(true), None), (Some(true), Some(31))),
        (Some(true), Some(31))
    );
    assert_eq!(
        edge_set(heard(Some(true), None), (Some(false), Some(31))),
        (Some(true), None)
    );
    // Nothing heard live: the series' last bucket, as before.
    assert_eq!(
        edge_set(LiveSet::default(), (Some(true), Some(31))),
        (Some(true), Some(31))
    );
    assert_eq!(
        edge_set(LiveSet::default(), (Some(false), None)),
        (Some(false), None)
    );
    assert_eq!(edge_set(LiveSet::default(), (None, None)), (None, None));
    // A song with an unknown transport is no song.
    assert_eq!(
        edge_set(heard(None, Some(Some(31))), (None, None)),
        (None, None)
    );
}

#[test]
fn a_live_songs_name_is_asked_for_at_most_every_five_ticks() {
    assert_eq!(NAME_RETRY_TICKS, 5);
    let names = BTreeMap::from([(31, "Morning Light".to_string()), (88, String::new())]);
    assert!(name_due(Some(57), &names, None, 1));
    assert!(!name_due(Some(57), &names, Some(2), 4));
    assert!(!name_due(Some(57), &names, Some(2), 6));
    assert!(name_due(Some(57), &names, Some(2), 7));
    assert!(name_due(Some(57), &names, Some(2), 8));
    // A known name, even an empty one, and no song: no ask.
    assert!(!name_due(Some(31), &names, None, 9));
    assert!(!name_due(Some(88), &names, None, 9));
    assert!(!name_due(None, &names, None, 9));
}
