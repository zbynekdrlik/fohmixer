use super::*;
use crate::lufs::wire::NO_PARENT;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// Device 9, v1.50.0, in the chain `Listen` of the rack `Listen` on
/// `Vox 1` in the set `FOH Band`, 48 kHz, streaming, polled on port 40007
/// (wire version 2, #264). `sim/listendev.py` pins the same bytes.
pub const GOLDEN_ANNOUNCE_HEX: &str = "2f666f686d697865722f6c697374656e000000002c69697373737373696969000000000200000009\
76312e35302e30004c697374656e00004c697374656e0000566f782031000000464f482042616e64\
000000000000bb800000000100009c47";
/// The hub's poll saying on (#264); `sim/listendev.py` pins the same bytes.
pub const GOLDEN_POLL_ON_HEX: &str =
    "2f666f686d697865722f6c697374656e2f706f6c6c0000002c69000000000001";
/// The hub's frame poll (#264); `sim/listendev.py` pins the same bytes.
pub const GOLDEN_FRAME_HEX: &str = "2f666f686d697865722f6c697374656e2f6672616d6500002c000000";
/// The hub's move (#264, the design comment 7228766032); `sim/listendev.py`
/// pins the same bytes.
pub const GOLDEN_MOVE_HEX: &str = "2f666f686d697865722f6c697374656e2f6d6f76650000002c000000";

fn golden() -> Announce {
    Announce {
        device: 9,
        version: "v1.50.0".into(),
        parents: parents_in(Some(("Listen", "Listen")), "Vox 1", "FOH Band"),
        sample_rate: 48_000,
        sending: true,
        port: 40_007,
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
    wire[wire_at + 3] = 3;
    assert_eq!(parse_announce(&wire), Err(WireError::Version));
    assert_eq!(
        parse_announce(&good[..good.len() - 2]),
        Err(WireError::Truncated)
    );
    let n = good.len();
    let mut sending = good.clone();
    sending[n - 5] = 2;
    assert_eq!(parse_announce(&sending), Err(WireError::Range));
    let mut negative = good.clone();
    negative[n - 5] = 0;
    assert!(parse_announce(&negative).is_ok_and(|a| !a.sending));
    negative[n - 12] = 0xff;
    assert_eq!(parse_announce(&negative), Err(WireError::Range));
    let mut rate_zero = good.clone();
    rate_zero[n - 12..n - 8].copy_from_slice(&[0, 0, 0, 0]);
    assert_eq!(parse_announce(&rate_zero).map(|a| a.sample_rate), Ok(0));
    // The poll port (#264): within the LUFS devices' range, else Range.
    for (port, ok) in [
        (39_999i32, false),
        (40_000, true),
        (49_999, true),
        (50_000, false),
    ] {
        let mut ported = good.clone();
        ported[n - 4..].copy_from_slice(&port.to_be_bytes());
        let want = if ok {
            Ok(u16::try_from(port).unwrap())
        } else {
            Err(WireError::Range)
        };
        assert_eq!(parse_announce(&ported).map(|a| a.port), want, "{port}");
    }
    // The wire before #264 (version 1, no port) is refused.
    let mut old = good;
    old[wire_at + 3] = 1;
    assert_eq!(parse_announce(&old), Err(WireError::Version));
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
fn the_hubs_polls_are_the_patchs_osc_messages() {
    // #264: the select pushed in the poll (`route` hands the patch the
    // int), the frame poll with no argument (`route` hands it a bang).
    assert_eq!(poll_datagram(true), hex(GOLDEN_POLL_ON_HEX));
    let mut off = hex(GOLDEN_POLL_ON_HEX);
    let last = off.len() - 1;
    off[last] = 0;
    assert_eq!(poll_datagram(false), off);
    assert_eq!(frame_datagram(), hex(GOLDEN_FRAME_HEX));
    assert_eq!(POLL_ADDRESS, "/fohmixer/listen/poll");
    assert_eq!(FRAME_ADDRESS, "/fohmixer/listen/frame");
    // The move (#264): no argument, its own address in the same `route`.
    assert_eq!(move_datagram(), hex(GOLDEN_MOVE_HEX));
    assert_eq!(MOVE_ADDRESS, "/fohmixer/listen/move");
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
    assert_eq!(hello_atoms(&h)[1], Atom::Long(2), "the wire of #264");
    let mut wire = hello_atoms(&h);
    wire[1] = Atom::Long(1);
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
    assert_eq!(POLL_MS, 250);
    assert_eq!(FRAME_POLL_MS, 20);
    assert_eq!(REREAD_POLLS, 40);
    assert_eq!(REREAD_POLLS * POLL_MS, 10_000, "the parents every 10 s");
    assert_eq!(FORGET_MS, 3000);
    assert_eq!(WIRE_VERSION, 2);
    assert_eq!(ANNOUNCE_TAGS, ",iisssssiii");
    assert_eq!(MAX_DATAGRAM, 512);
}

#[test]
fn a_datagram_of_exactly_the_limit_is_read() {
    let exact = (1..500)
        .map(|n| {
            encode_announce(&Announce {
                version: "v".repeat(n),
                ..golden()
            })
        })
        .find(|d| d.len() == MAX_DATAGRAM)
        .expect("an announce of exactly the limit");
    assert!(parse_announce(&exact).is_ok());
}

#[test]
fn a_float_of_a_billion_or_more_is_no_whole_number() {
    let h = Hello {
        device: 9,
        version: "v1.50.0".into(),
        sample_rate: 48_000,
    };
    let mut billion = hello_atoms(&h);
    billion[2] = Atom::Float(1e9);
    assert_eq!(parse_hello(&billion), None);
    billion[2] = Atom::Float(999_999_936.0);
    assert_eq!(parse_hello(&billion).map(|h| h.device), Some(999_999_936));
}
