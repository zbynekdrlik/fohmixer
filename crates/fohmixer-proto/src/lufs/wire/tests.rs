use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// The hub's move of a LUFS device (#264); `sim/maxdev.py` pins the same
/// bytes.
const GOLDEN_LUFS_MOVE_HEX: &str = "2f666f686d697865722f6c7566732f6d6f7665002c000000";
/// The hub's move of a meter device (#264).
const GOLDEN_METER_MOVE_HEX: &str = "2f666f686d697865722f6d657465722f6d6f7665000000002c000000";

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
        port: None,
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
        port: None,
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
    for tags in [
        ",iisssssff",
        ",iissss",
        ",iisssssiiii",
        ",iisssssiif",
        "iisssssii",
    ] {
        let datagram = raw(LUFS_ADDRESS, tags, [1, 7], &PARTS, [-253, -248]);
        assert_eq!(parse(&datagram), Err(WireError::Tags), "{tags}");
    }
}

#[test]
fn each_form_takes_its_own_wire_version_only() {
    // The pushed form (nine fields) is version 1 only.
    for version in [2, 0, -1, 3] {
        let datagram = raw(LUFS_ADDRESS, TYPE_TAGS, [version, 7], &PARTS, [-253, -248]);
        assert_eq!(parse(&datagram), Err(WireError::Version), "{version}");
    }
    // The polled form (#264, the port as a tenth field) is version 2 only.
    for version in [1, 0, -1, 3] {
        let mut datagram = raw(
            LUFS_ADDRESS,
            POLLED_TYPE_TAGS,
            [version, 7],
            &PARTS,
            [-253, -248],
        );
        datagram.extend_from_slice(&41_234_i32.to_be_bytes());
        assert_eq!(parse(&datagram), Err(WireError::Version), "{version}");
    }
}

fn polled() -> Reading {
    Reading {
        kind: DeviceKind::Lufs,
        device: 7,
        version: "v1.19.0".into(),
        parents: names(["LUFS IN", "CORE Effects", "Vox 1", "FOH Band"]),
        first: -253,
        second: -248,
        port: Some(41_234),
    }
}

#[test]
fn the_polled_form_carries_the_devices_port() {
    // #264: the hand-built polled datagram, the golden one plus its port.
    let mut datagram = raw(LUFS_ADDRESS, POLLED_TYPE_TAGS, [2, 7], &PARTS, [-253, -248]);
    datagram.extend_from_slice(&41_234_i32.to_be_bytes());
    assert_eq!(datagram.len(), 100);
    assert_eq!(parse(&datagram), Ok(polled()));
    assert_eq!(encode(&polled()), datagram);
    // The pushed form still parses, with no port (a device of before
    // 1.58.0 during the rollout, FohMixer Meter).
    assert_eq!(parse(&hex(GOLDEN_HEX)).map(|r| r.port), Ok(None));
    assert_eq!(parse(&encode(&meter())), Ok(meter()));
}

#[test]
fn a_poll_port_outside_the_drawn_range_is_out_of_range() {
    for port in [0, 39_999, 50_000, 65_535, 65_536, -1, 39_110] {
        let mut datagram = encode(&polled());
        let at = datagram.len() - 4;
        datagram[at..].copy_from_slice(&i32::to_be_bytes(port));
        assert_eq!(parse(&datagram), Err(WireError::Range), "{port}");
    }
    for port in [40_000, 49_999, 45_000] {
        let mut datagram = encode(&polled());
        let at = datagram.len() - 4;
        datagram[at..].copy_from_slice(&i32::to_be_bytes(port));
        let want = u16::try_from(port).ok();
        assert_eq!(parse(&datagram).map(|r| r.port), Ok(want), "{port}");
    }
    assert_eq!(poll_port(40_000), Ok(40_000));
    assert_eq!(poll_port(49_999), Ok(49_999));
    assert_eq!(poll_port(39_999), Err(WireError::Range));
    assert_eq!(poll_port(50_000), Err(WireError::Range));
}

#[test]
fn a_polled_datagram_cut_or_with_more_is_refused() {
    let datagram = encode(&polled());
    for end in 0..datagram.len() {
        assert!(parse(&datagram[..end]).is_err(), "a prefix of {end} bytes");
    }
    let mut longer = datagram.clone();
    longer.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse(&longer), Err(WireError::Trailing));
}

#[test]
fn the_poll_is_an_osc_message_with_no_argument() {
    // #264: each kind its own address (the meter's since its pull).
    let lufs = poll_datagram(DeviceKind::Lufs);
    assert_eq!(lufs, b"/fohmixer/lufs/poll\0,\0\0\0".to_vec());
    let meter = poll_datagram(DeviceKind::Meter);
    assert_eq!(meter, b"/fohmixer/meter/poll\0\0\0\0,\0\0\0".to_vec());
    for (kind, poll) in [(DeviceKind::Lufs, &lufs), (DeviceKind::Meter, &meter)] {
        assert_eq!(poll.len() % 4, 0);
        let (address, rest) = osc_string(poll).unwrap();
        assert_eq!(address, kind.poll_address());
        assert_eq!(osc_string(rest), Ok((",", &[][..])));
    }
}

#[test]
fn the_move_is_an_osc_message_with_no_argument() {
    // #264 (the design comment 7228766032): each kind its own address, so
    // a device of the other kind on the port drops it in its `route`.
    // `sim/maxdev.py` pins the same bytes.
    let lufs = move_datagram(DeviceKind::Lufs);
    assert_eq!(lufs, hex(GOLDEN_LUFS_MOVE_HEX));
    assert_eq!(lufs, b"/fohmixer/lufs/move\0,\0\0\0".to_vec());
    let meter = move_datagram(DeviceKind::Meter);
    assert_eq!(meter, hex(GOLDEN_METER_MOVE_HEX));
    for (kind, out) in [(DeviceKind::Lufs, &lufs), (DeviceKind::Meter, &meter)] {
        let (address, rest) = osc_string(out).unwrap();
        assert_eq!(address, kind.move_address());
        assert_eq!(osc_string(rest), Ok((",", &[][..])));
        assert_ne!(*out, poll_datagram(kind));
    }
    assert_eq!(LUFS_MOVE_ADDRESS, "/fohmixer/lufs/move");
    assert_eq!(METER_MOVE_ADDRESS, "/fohmixer/meter/move");
    // One move per port per 5 s.
    assert_eq!(MOVE_EVERY_MS, 5000);
}

#[test]
fn a_meters_polled_form_carries_its_port() {
    // #264: the meter answers the hub's poll in the polled form too.
    let reading = Reading {
        kind: DeviceKind::Meter,
        device: 12,
        version: "v1.61.0".into(),
        parents: names(["Meter", "LUFS", "Vox 2", "FOH Band"]),
        first: 120,
        second: -700,
        port: Some(45_678),
    };
    let datagram = encode(&reading);
    assert_eq!(&datagram[..16], b"/fohmixer/meter\0");
    assert_eq!(parse(&datagram), Ok(reading));
    // The pushed form of a meter from before its pull still parses.
    assert_eq!(parse(&encode(&meter())).map(|r| r.port), Ok(None));
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
    assert_eq!(POLLED_WIRE_VERSION, 2);
    assert_eq!(POLLED_TYPE_TAGS, ",iisssssiii");
    assert_eq!((POLL_PORT_MIN, POLL_PORT_SPAN), (40_000, 10_000));
    assert_eq!(LUFS_POLL_ADDRESS, "/fohmixer/lufs/poll");
    assert_eq!(METER_POLL_ADDRESS, "/fohmixer/meter/poll");
    assert_eq!(DeviceKind::Lufs.poll_address(), LUFS_POLL_ADDRESS);
    assert_eq!(DeviceKind::Meter.poll_address(), METER_POLL_ADDRESS);
    // #264: LUFS 5 a second, the meter 30 a second (its faces' old rate).
    assert_eq!((LUFS_POLL_EVERY_MS, METER_POLL_EVERY_MS), (200, 33));
    assert_eq!(DeviceKind::Lufs.poll_every_ms(), 200);
    assert_eq!(DeviceKind::Meter.poll_every_ms(), 33);
    assert_eq!(MAX_DATAGRAM, 512);
    assert_eq!(DeviceKind::Lufs.address(), "/fohmixer/lufs");
    assert_eq!(DeviceKind::Meter.address(), "/fohmixer/meter");
    assert_eq!((DeviceKind::Lufs.top(), DeviceKind::Meter.top()), (0, 120));
}
