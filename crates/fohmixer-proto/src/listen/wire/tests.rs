use super::*;
use crate::lufs::wire::NO_PARENT;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// Device 9, v1.50.0, in the chain `Listen` of the rack `Listen` on
/// `Vox 1` in the set `FOH Band`, 48 kHz, streaming. `sim/listendev.py`
/// pins the same bytes.
pub const GOLDEN_ANNOUNCE_HEX: &str = "2f666f686d697865722f6c697374656e000000002c69697373737373696900000000000100000009\
76312e35302e30004c697374656e00004c697374656e0000566f782031000000464f482042616e64000000000000bb8000000001";

fn golden() -> Announce {
    Announce {
        device: 9,
        version: "v1.50.0".into(),
        parents: parents_in(Some(("Listen", "Listen")), "Vox 1", "FOH Band"),
        sample_rate: 48_000,
        sending: true,
    }
}

#[test]
fn the_golden_announce_encodes_and_parses() {
    let bytes = encode_announce(&golden());
    assert_eq!(bytes, hex(GOLDEN_ANNOUNCE_HEX));
    assert_eq!(parse_announce(&bytes), Ok(golden()));
    assert_eq!(golden().track_and_set(), Some(("Vox 1", "FOH Band")));
}

#[test]
fn an_announce_on_a_track_names_its_track_and_set() {
    let a = Announce {
        parents: parents_in(None, "Vox 2", "FOH Band"),
        sending: false,
        sample_rate: 44_100,
        ..golden()
    };
    let back = parse_announce(&encode_announce(&a)).unwrap();
    assert_eq!(back.track_and_set(), Some(("Vox 2", "FOH Band")));
    assert!(!back.sending);
    assert_eq!(back.sample_rate, 44_100);
    assert_eq!(back.parents[2], NO_PARENT);
}

#[test]
fn a_bad_announce_is_refused_with_its_reason() {
    let good = encode_announce(&golden());
    let mut long = good.clone();
    long.resize(MAX_DATAGRAM + 1, 0);
    assert_eq!(parse_announce(&long), Err(WireError::TooLong));
    let mut at_limit = encode_announce(&Announce {
        version: "v".repeat(400),
        ..golden()
    });
    assert!(at_limit.len() <= MAX_DATAGRAM);
    assert!(parse_announce(&at_limit).is_ok());
    at_limit.push(0);
    assert_eq!(parse_announce(&at_limit), Err(WireError::Trailing));
    let mut lufs = good.clone();
    lufs[10] = b'u';
    assert_eq!(parse_announce(&lufs), Err(WireError::Address));
    let tags_at = 20 + 1;
    let mut tags = good.clone();
    tags[tags_at + 1] = b's';
    assert_eq!(parse_announce(&tags), Err(WireError::Tags));
    let wire_at = 20 + 12;
    let mut wire = good.clone();
    wire[wire_at + 3] = 2;
    assert_eq!(parse_announce(&wire), Err(WireError::Version));
    assert_eq!(
        parse_announce(&good[..good.len() - 2]),
        Err(WireError::Truncated)
    );
    let n = good.len();
    let mut sending = good.clone();
    sending[n - 1] = 2;
    assert_eq!(parse_announce(&sending), Err(WireError::Range));
    let mut negative = good.clone();
    negative[n - 1] = 0;
    assert!(parse_announce(&negative).is_ok_and(|a| !a.sending));
    negative[n - 8] = 0xff;
    assert_eq!(parse_announce(&negative), Err(WireError::Range));
    let mut rate_zero = good;
    rate_zero[n - 8..n - 4].copy_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse_announce(&rate_zero).map(|a| a.sample_rate), Ok(0));
    let lonely = Announce {
        parents: ["Vox 1", NO_PARENT, NO_PARENT, NO_PARENT].map(str::to_string),
        ..golden()
    };
    assert_eq!(
        parse_announce(&encode_announce(&lonely)),
        Err(WireError::Identity)
    );
}

#[test]
fn a_select_round_trips_and_is_checked() {
    let s = Select { device: 9, turn: 3 };
    let bytes = encode_select(s);
    assert_eq!(&bytes[..24], b"/fohmixer/listen/select\0");
    assert_eq!(&bytes[24..28], b",ii\0");
    assert_eq!(&bytes[28..], &[0, 0, 0, 9, 0, 0, 0, 3]);
    assert_eq!(parse_select(&bytes), Ok(s));
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse_select(&trailing), Err(WireError::Trailing));
    assert_eq!(
        parse_select(&encode_announce(&golden())),
        Err(WireError::Address)
    );
    let mut tags = bytes.clone();
    tags[25] = b's';
    assert_eq!(parse_select(&tags), Err(WireError::Tags));
    let mut long = bytes;
    long.resize(MAX_DATAGRAM + 1, 0);
    assert_eq!(parse_select(&long), Err(WireError::TooLong));
}

#[test]
fn a_hello_reads_longs_or_whole_floats() {
    let h = Hello {
        device: 9,
        version: "v1.50.0".into(),
        sample_rate: 48_000,
    };
    assert_eq!(parse_hello(&hello_atoms(&h)), Some(h.clone()));
    let mut floats = hello_atoms(&h);
    floats[4] = Atom::Float(48_000.0);
    floats[2] = Atom::Float(9.0);
    assert_eq!(parse_hello(&floats), Some(h.clone()));
    let mut fraction = hello_atoms(&h);
    fraction[4] = Atom::Float(47_999.5);
    assert_eq!(parse_hello(&fraction), None);
    let mut huge = hello_atoms(&h);
    huge[4] = Atom::Float(1e10);
    assert_eq!(parse_hello(&huge), None);
    let mut word = hello_atoms(&h);
    word[0] = Atom::Sym("fohmixer_lufs".into());
    assert_eq!(parse_hello(&word), None);
    let mut wire = hello_atoms(&h);
    wire[1] = Atom::Long(2);
    assert_eq!(parse_hello(&wire), None);
    let mut version = hello_atoms(&h);
    version[3] = Atom::Long(1);
    assert_eq!(parse_hello(&version), None);
    let mut sym = hello_atoms(&h);
    sym[2] = Atom::Sym("9".into());
    assert_eq!(parse_hello(&sym), None);
    assert_eq!(parse_hello(&hello_atoms(&h)[..4]), None);
}

#[test]
fn the_timing_constants_hold() {
    assert_eq!(SELECT_EVERY_MS, 500);
    assert_eq!(SELECT_HOLD_MS, 1500);
    assert_eq!(ANNOUNCE_EVERY_MS, 1000);
    assert_eq!(FORGET_MS, 3000);
    assert_eq!(MAX_DATAGRAM, 512);
}
