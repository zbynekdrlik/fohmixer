use super::*;

#[test]
fn every_word_reads_back_as_itself() {
    for word in [
        Word::Notes(vec![50, 57]),
        Word::Notes(vec![0, 127]),
        Word::Pattern([false, true, false, true]),
        Word::Pattern([true, false, true, false]),
        Word::Silent(true),
        Word::Silent(false),
        Word::On(true),
        Word::On(false),
    ] {
        assert_eq!(parse(&encode(&word)), Ok(word));
    }
}

#[test]
fn a_pattern_is_its_address_four_int_tags_and_four_big_endian_ints() {
    let bytes = encode(&Word::Pattern([false, true, false, true]));
    let mut want = b"/fohmixer/claps/pattern\0,iiii\0\0\0".to_vec();
    for v in [0i32, 1, 0, 1] {
        want.extend_from_slice(&v.to_be_bytes());
    }
    assert_eq!(bytes, want);
}

#[test]
fn a_switch_is_its_address_and_one_int() {
    let mut want = b"/fohmixer/claps/on\0\0,i\0\0".to_vec();
    want.extend_from_slice(&1i32.to_be_bytes());
    assert_eq!(encode(&Word::On(true)), want);
}

#[test]
fn a_word_out_of_range_or_shape_is_refused() {
    let mut two = b"/fohmixer/claps/on\0\0,i\0\0".to_vec();
    two.extend_from_slice(&2i32.to_be_bytes());
    assert_eq!(parse(&two), Err(WireError::Range));
    let mut three = b"/fohmixer/claps/pattern\0,iii\0\0\0\0".to_vec();
    for v in [0i32, 1, 0] {
        three.extend_from_slice(&v.to_be_bytes());
    }
    assert_eq!(parse(&three), Err(WireError::Tags));
    let mut pitch = b"/fohmixer/claps/notes\0\0\0,i\0\0".to_vec();
    pitch.extend_from_slice(&128i32.to_be_bytes());
    assert_eq!(parse(&pitch), Err(WireError::Range));
    let mut other = b"/fohmixer/other\0,i\0\0".to_vec();
    other.extend_from_slice(&1i32.to_be_bytes());
    assert_eq!(parse(&other), Err(WireError::Address));
    let mut more = encode(&Word::On(true));
    more.extend_from_slice(&[0, 0, 0, 1]);
    assert_eq!(parse(&more), Err(WireError::Trailing));
    assert_eq!(
        parse(b"/fohmixer/claps/on\0\0,f\0\0\0\0\0\0"),
        Err(WireError::Tags)
    );
    assert_eq!(
        parse(&vec![b'/'; MAX_DATAGRAM + 1]),
        Err(WireError::TooLong)
    );
}

#[test]
fn no_notes_and_too_many_notes_are_refused() {
    assert_eq!(
        parse(&encode(&Word::Notes(Vec::new()))),
        Err(WireError::Tags)
    );
    assert_eq!(
        parse(&encode(&Word::Notes(vec![50; MAX_NOTES]))),
        Ok(Word::Notes(vec![50; MAX_NOTES]))
    );
    assert_eq!(
        parse(&encode(&Word::Notes(vec![50; MAX_NOTES + 1]))),
        Err(WireError::Tags)
    );
}

#[test]
fn a_hello_reads_back_and_its_bytes_are_max_udpsends() {
    let hello = Hello {
        version: "v1.52.0".into(),
        running: true,
    };
    let bytes = encode_hello(&hello);
    let mut want = b"/fohmixer/claps\0,isi\0\0\0\0".to_vec();
    want.extend_from_slice(&1i32.to_be_bytes());
    want.extend_from_slice(b"v1.52.0\0");
    want.extend_from_slice(&1i32.to_be_bytes());
    assert_eq!(bytes, want);
    assert_eq!(parse_hello(&bytes), Ok(hello));
}

#[test]
fn a_hello_of_another_shape_is_refused() {
    let good = Hello {
        version: "v1".into(),
        running: false,
    };
    let mut wire2 = encode_hello(&good);
    wire2[24..28].copy_from_slice(&2i32.to_be_bytes());
    assert_eq!(parse_hello(&wire2), Err(WireError::Version));
    assert_eq!(
        parse_hello(&encode(&Word::On(true))),
        Err(WireError::Address)
    );
    let mut tags = b"/fohmixer/claps\0,is\0".to_vec();
    tags.extend_from_slice(&1i32.to_be_bytes());
    tags.extend_from_slice(b"v1\0\0");
    assert_eq!(parse_hello(&tags), Err(WireError::Tags));
    let mut running = encode_hello(&good);
    let at = running.len() - 4;
    running[at..].copy_from_slice(&3i32.to_be_bytes());
    assert_eq!(parse_hello(&running), Err(WireError::Range));
    let mut more = encode_hello(&good);
    more.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse_hello(&more), Err(WireError::Trailing));
    assert_eq!(
        parse_hello(&vec![b'/'; MAX_DATAGRAM + 1]),
        Err(WireError::TooLong)
    );
}

#[test]
fn a_whole_state_sends_the_pattern_before_the_switch() {
    let words = ClapsWords {
        pattern: [false, true, false, true],
        on: true,
        silent: false,
        notes: vec![50],
    };
    assert_eq!(
        words_of(&words),
        vec![
            Word::Notes(vec![50]),
            Word::Pattern([false, true, false, true]),
            Word::Silent(false),
            Word::On(true),
        ]
    );
}

#[test]
fn the_state_leaves_out_an_unknown_playhead() {
    let state = ClapsState {
        state: ClapsPhase::NextBar,
        pattern: [false, true, false, true],
        beat: None,
        tempo: None,
    };
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::json!({"state": "next_bar", "pattern": [false, true, false, true]})
    );
    let read: ClapsState = serde_json::from_value(serde_json::json!({
        "state": "signature",
        "pattern": [true, true, true, true],
        "beat": 3.5,
        "tempo": 120.0
    }))
    .unwrap();
    assert_eq!(read.state, ClapsPhase::Signature);
    assert_eq!(read.beat, Some(3.5));
}
