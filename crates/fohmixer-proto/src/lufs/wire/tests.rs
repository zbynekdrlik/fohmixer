use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

fn names(parents: [&str; PARENTS]) -> [String; PARENTS] {
    parents.map(str::to_string)
}

fn golden() -> Reading {
    Reading {
        kind: DeviceKind::Lufs,
        device: 7,
        version: "v1.19.0".into(),
        parents: names(["LUFS IN", "CORE Effects", "Vox 1", "FOH Band"]),
        first: -253,
        second: -248,
    }
}

fn meter() -> Reading {
    Reading {
        kind: DeviceKind::Meter,
        device: 12,
        version: "v1.19.0".into(),
        parents: names(["Vox 2", "FOH Band", NO_PARENT, NO_PARENT]),
        first: 120,
        second: -700,
    }
}

/// The OSC message of these raw parts, built by hand (not by `encode`).
fn raw(address: &str, tags: &str, ints: [i32; 2], strings: &[&str], values: [i32; 2]) -> Vec<u8> {
    let mut out = Vec::new();
    let string = |out: &mut Vec<u8>, s: &[u8]| {
        out.extend_from_slice(s);
        out.extend(std::iter::repeat_n(0, 4 - s.len() % 4));
    };
    string(&mut out, address.as_bytes());
    string(&mut out, tags.as_bytes());
    for i in ints {
        out.extend_from_slice(&i.to_be_bytes());
    }
    for s in strings {
        string(&mut out, s.as_bytes());
    }
    for v in values {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out
}

const PARTS: [&str; 5] = ["v1.19.0", "LUFS IN", "CORE Effects", "Vox 1", "FOH Band"];

#[test]
fn parses_the_golden_datagram() {
    let datagram = hex(GOLDEN_HEX);
    assert_eq!(datagram.len(), 96);
    let reading = parse(&datagram).expect("the golden datagram parses");
    assert_eq!(reading, golden());
    assert_eq!(reading.track_and_set(), Some(("Vox 1", "FOH Band")));
    // The hand-built message is the same bytes.
    assert_eq!(
        raw(LUFS_ADDRESS, TYPE_TAGS, [1, 7], &PARTS, [-253, -248]),
        datagram
    );
}

#[test]
fn encode_gives_the_golden_bytes() {
    assert_eq!(encode(&golden()), hex(GOLDEN_HEX));
}

#[test]
fn a_meter_datagram_parses() {
    let reading = parse(&encode(&meter())).expect("a meter datagram parses");
    assert_eq!(reading, meter());
    assert_eq!(reading.kind, DeviceKind::Meter);
    assert_eq!((reading.first, reading.second), (120, -700));
    assert_eq!(reading.track_and_set(), Some(("Vox 2", "FOH Band")));
}

#[test]
fn over_512_bytes_is_too_long() {
    let mut long = golden();
    // 96 bytes, then the name grows the datagram to 516.
    long.parents[0] = "x".repeat(7 + 420);
    let datagram = encode(&long);
    assert_eq!(datagram.len(), 516);
    assert_eq!(parse(&datagram), Err(WireError::TooLong));
    long.parents[0] = "x".repeat(7 + 416);
    let datagram = encode(&long);
    assert_eq!(datagram.len(), MAX_DATAGRAM);
    assert_eq!(parse(&datagram).map(|r| r.parents[0].len()), Ok(423));
}

#[test]
fn a_cut_string_is_truncated() {
    // No NUL within the datagram.
    assert_eq!(parse(b"/fohmixer/lufs"), Err(WireError::Truncated));
    assert_eq!(parse(b""), Err(WireError::Truncated));
}

#[test]
fn padding_not_to_four_is_truncated() {
    let mut datagram = hex(GOLDEN_HEX);
    // The address's padding: two NULs, the second made a byte.
    assert_eq!(&datagram[14..16], &[0, 0]);
    datagram[15] = b'x';
    assert_eq!(parse(&datagram), Err(WireError::Truncated));
    // One NUL only: the tags start one byte early.
    let mut short = hex(GOLDEN_HEX);
    short.remove(15);
    assert_eq!(parse(&short), Err(WireError::Truncated));
}

#[test]
fn osc_string_reads_to_its_padding() {
    assert_eq!(osc_string(b"ab\0\0"), Ok(("ab", &b""[..])));
    assert_eq!(osc_string(b"abc\0xyz"), Ok(("abc", &b"xyz"[..])));
    assert_eq!(osc_string(b"abcd\0\0\0\0"), Ok(("abcd", &b""[..])));
    assert_eq!(osc_string(b"\0\0\0\0z"), Ok(("", &b"z"[..])));
    assert_eq!(osc_string(b"abcd\0\0\0"), Err(WireError::Truncated));
    assert_eq!(osc_string(b"ab\0"), Err(WireError::Truncated));
}

#[test]
fn osc_i32_reads_big_endian() {
    assert_eq!(osc_i32(&[0, 0, 1, 2, 9]), Ok((258, &[9u8][..])));
    assert_eq!(osc_i32(&[0xff, 0xff, 0xff, 0x03]), Ok((-253, &b""[..])));
    assert_eq!(osc_i32(&[0, 0, 1]), Err(WireError::Truncated));
}

#[test]
fn another_address_is_refused() {
    for address in ["/fohmixer/loud", "/fohmixer/lufs/x", "/fohmixer", ""] {
        let datagram = raw(address, TYPE_TAGS, [1, 7], &PARTS, [-253, -248]);
        assert_eq!(parse(&datagram), Err(WireError::Address), "{address}");
    }
}

#[test]
fn float_values_are_refused_by_tags() {
    for tags in [",iisssssff", ",iissss", ",iisssssiii", "iisssssii"] {
        let datagram = raw(LUFS_ADDRESS, tags, [1, 7], &PARTS, [-253, -248]);
        assert_eq!(parse(&datagram), Err(WireError::Tags), "{tags}");
    }
}

#[test]
fn wire_version_2_is_refused() {
    for version in [2, 0, -1] {
        let datagram = raw(LUFS_ADDRESS, TYPE_TAGS, [version, 7], &PARTS, [-253, -248]);
        assert_eq!(parse(&datagram), Err(WireError::Version), "{version}");
    }
}

#[test]
fn a_name_not_utf8_is_refused() {
    let mut datagram = hex(GOLDEN_HEX);
    // `Vox 1`'s V.
    let at = datagram
        .windows(5)
        .position(|w| w == b"Vox 1")
        .expect("the track's name");
    datagram[at] = 0xff;
    assert_eq!(parse(&datagram), Err(WireError::Utf8));
}

#[test]
fn a_lufs_value_above_0_is_out_of_range() {
    for values in [
        [1, -248],
        [-253, 1],
        [-701, -248],
        [-253, -701],
        [i32::MAX, 0],
        [70_000, 0],
    ] {
        let datagram = raw(LUFS_ADDRESS, TYPE_TAGS, [1, 7], &PARTS, values);
        assert_eq!(parse(&datagram), Err(WireError::Range), "{values:?}");
    }
    let edges = raw(LUFS_ADDRESS, TYPE_TAGS, [1, 7], &PARTS, [0, -700]);
    let reading = parse(&edges).expect("the edges are in range");
    assert_eq!((reading.first, reading.second), (0, -700));
}

#[test]
fn a_meter_value_above_120_is_out_of_range() {
    for values in [[121, 0], [0, 121], [-701, 0], [0, -701]] {
        let datagram = raw(METER_ADDRESS, TYPE_TAGS, [1, 7], &PARTS, values);
        assert_eq!(parse(&datagram), Err(WireError::Range), "{values:?}");
    }
    let edges = raw(METER_ADDRESS, TYPE_TAGS, [1, 7], &PARTS, [120, -700]);
    let reading = parse(&edges).expect("the edges are in range");
    assert_eq!((reading.first, reading.second), (120, -700));
}

#[test]
fn bytes_after_the_last_value_are_refused() {
    let mut datagram = hex(GOLDEN_HEX);
    datagram.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse(&datagram), Err(WireError::Trailing));
}

#[test]
fn fewer_than_two_named_parents_is_refused() {
    for parents in [
        ["Vox 1", NO_PARENT, NO_PARENT, NO_PARENT],
        [NO_PARENT, NO_PARENT, NO_PARENT, NO_PARENT],
        [NO_PARENT, "Vox 1", "FOH Band", NO_PARENT],
    ] {
        let mut reading = golden();
        reading.parents = names(parents);
        assert_eq!(
            parse(&encode(&reading)),
            Err(WireError::Identity),
            "{parents:?}"
        );
    }
}

#[test]
fn the_track_and_set_are_the_last_two_named_parents() {
    let of = |parents: [&str; PARENTS]| {
        track_and_set(&names(parents)).map(|(t, s)| (t.to_string(), s.to_string()))
    };
    let pair = |t: &str, s: &str| Some((t.to_string(), s.to_string()));
    // On a track.
    assert_eq!(
        of(["Vox 1", "FOH Band", "-", "-"]),
        pair("Vox 1", "FOH Band")
    );
    // In a rack's chain.
    assert_eq!(
        of(["LUFS IN", "CORE Effects", "Vox 1", "FOH Band"]),
        pair("Vox 1", "FOH Band")
    );
    // Three named: the third is taken for the set.
    assert_eq!(of(["A", "B", "C", "-"]), pair("B", "C"));
    // A name after the first placeholder does not count.
    assert_eq!(
        of(["Vox 1", "FOH Band", "-", "X"]),
        pair("Vox 1", "FOH Band")
    );
    assert_eq!(of(["Vox 1", "-", "-", "-"]), None);
    assert_eq!(track_and_set(&[]), None);
}

#[test]
fn every_truncation_is_refused_never_a_panic() {
    let datagram = hex(GOLDEN_HEX);
    for end in 0..datagram.len() {
        assert!(parse(&datagram[..end]).is_err(), "a prefix of {end} bytes");
    }
}

#[test]
fn every_reason_has_its_word() {
    let words: Vec<&str> = [
        WireError::TooLong,
        WireError::Truncated,
        WireError::Trailing,
        WireError::Address,
        WireError::Tags,
        WireError::Version,
        WireError::Utf8,
        WireError::Range,
        WireError::Identity,
    ]
    .map(WireError::reason)
    .to_vec();
    assert_eq!(
        words,
        [
            "too_long",
            "truncated",
            "trailing",
            "address",
            "tags",
            "version",
            "utf8",
            "range",
            "identity"
        ]
    );
}

#[test]
fn the_constants() {
    assert_eq!(DEFAULT_PORT, 39110);
    assert_eq!(WIRE_VERSION, 1);
    assert_eq!(MAX_DATAGRAM, 512);
    assert_eq!(DeviceKind::Lufs.address(), "/fohmixer/lufs");
    assert_eq!(DeviceKind::Meter.address(), "/fohmixer/meter");
    assert_eq!((DeviceKind::Lufs.top(), DeviceKind::Meter.top()), (0, 120));
}
