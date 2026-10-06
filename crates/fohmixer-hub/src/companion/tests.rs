use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::*;
use crate::config::CompanionCfg;

fn deck(columns: u32, rows: u32, bitmap_px: u32) -> CompanionCfg {
    CompanionCfg {
        host: "10.0.0.7".into(),
        port: 16622,
        columns,
        rows,
        bitmap_px,
        title: "Stream Deck".into(),
    }
}

fn params(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
        .collect()
}

#[test]
fn a_line_is_its_command_and_its_parameters() {
    let line = parse_line("CMD A=1 B=\"x y\" C FLAG D=\"\" E=\"a=b+c/d==\" ").unwrap();
    assert_eq!(line.cmd, "CMD");
    assert_eq!(
        line.params,
        params(&[
            ("A", Some("1")),
            ("B", Some("x y")),
            ("C", None),
            ("FLAG", None),
            ("D", Some("")),
            ("E", Some("a=b+c/d==")),
        ])
    );
    assert_eq!(line.get("A"), Some("1"));
    assert_eq!(line.get("C"), None, "a bare word has no value");
    assert!(line.has("C") && line.has("A") && !line.has("Z"));
    // A quoted value at the line's end, with or without its closing quote.
    assert_eq!(parse_line("X V=\"a b\"").unwrap().get("V"), Some("a b"));
    assert_eq!(parse_line("X V=\"a b").unwrap().get("V"), Some("a b"));
    // A command alone, and blank lines.
    assert_eq!(parse_line("KEYS-CLEAR").unwrap().params, BTreeMap::new());
    assert_eq!(parse_line(""), None);
    assert_eq!(parse_line("   \r"), None);
    // A token that starts with `=` has an empty name and is still consumed.
    assert_eq!(
        parse_line("X =1 Y=2").unwrap().params,
        params(&[("", Some("1")), ("Y", Some("2"))])
    );
}

#[test]
fn classify_reads_companion_5_0_7_lines() {
    // The lines as the probe captured them (#52 plan), base64 shortened:
    // every line ends with a space.
    assert_eq!(
        classify("BEGIN CompanionVersion=\"5.0.7+9763-stable-cec2f88e6f\" ApiVersion=\"1.12.0\" "),
        Inbound::Begin {
            companion: "5.0.7+9763-stable-cec2f88e6f".into(),
            api: "1.12.0".into()
        }
    );
    assert_eq!(
        classify("CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\" "),
        Inbound::Caps("SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\"".into())
    );
    assert_eq!(
        classify("ADD-DEVICE OK DEVICEID=\"fohmixer-1\" "),
        Inbound::Added(Ok(()))
    );
    assert_eq!(
        classify("ADD-DEVICE ERROR DEVICEID=\"fohmixer-1\" MESSAGE=\"Device exists elsewhere\" "),
        Inbound::Added(Err("Device exists elsewhere".into()))
    );
    assert_eq!(
        classify("KEY-PRESS OK DEVICEID=\"fohmixer-1\" "),
        Inbound::Pressed(Ok(()))
    );
    assert_eq!(
        classify("KEY-PRESS ERROR DEVICEID=\"fohmixer-1\" MESSAGE=\"Invalid KEY\" "),
        Inbound::Pressed(Err("Invalid KEY".into()))
    );
    assert_eq!(
        classify("KEY-PRESS ERROR DEVICEID=\"nope\" "),
        Inbound::Pressed(Err("error".into())),
        "an ERROR without MESSAGE"
    );
    assert_eq!(
        classify(
            "KEY-STATE DEVICEID=\"fohmixer-10\" KEY=0 LOCATION=\"1/0/0\" PRESSED=1 TYPE=\"BUTTON\" \
             BITMAP=\"data:image/webp;base64,UklGRsIcAABXRUJQ+/4y+M=\" COLOR=\"#00aa00\" TEXTCOLOR=\"#ffffff\" "
        ),
        Inbound::KeyState(KeyUpdate {
            key: 0,
            img: Some("data:image/webp;base64,UklGRsIcAABXRUJQ+/4y+M=".into()),
            color: Some("#00aa00".into()),
            pressed: Some(true),
        })
    );
    // A page key of a fresh install: no COLOR.
    assert_eq!(
        classify(
            "KEY-STATE DEVICEID=\"fohmixer-1\" KEY=8 LOCATION=\"1/1/0\" PRESSED=0 TYPE=\"PAGENUM\" \
             BITMAP=\"data:image/webp;base64,AAAA\" "
        ),
        Inbound::KeyState(KeyUpdate {
            key: 8,
            img: Some("data:image/webp;base64,AAAA".into()),
            color: None,
            pressed: Some(false),
        })
    );
    // A missing field keeps the cache's old value: none here.
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=3 COLOR=\"#ff0000\""),
        Inbound::KeyState(KeyUpdate {
            key: 3,
            img: None,
            color: Some("#ff0000".into()),
            pressed: None,
        })
    );
    // A COLOR that is not `#rrggbb` is dropped: it goes into the key's style
    // on the page, where a `;` would add declarations of its own.
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=3 COLOR=\"#ff0000;background:red\""),
        Inbound::KeyState(KeyUpdate {
            key: 3,
            img: None,
            color: None,
            pressed: None,
        })
    );
    // A raw rgb bitmap is no image the page can show.
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=1 BITMAP=\"AAAAAA==\" PRESSED=true"),
        Inbound::KeyState(KeyUpdate {
            key: 1,
            img: None,
            color: None,
            pressed: Some(true),
        })
    );
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=x"),
        Inbound::Other("KEY-STATE".into())
    );
    assert_eq!(
        classify("KEYS-CLEAR DEVICEID=\"fohmixer-1\" "),
        Inbound::KeysClear
    );
    assert_eq!(classify("PING abc"), Inbound::Ping("abc".into()));
    assert_eq!(classify("PING abc "), Inbound::Ping("abc".into()));
    assert_eq!(classify("PONG 7 "), Inbound::Pong);
    assert_eq!(
        classify("BRIGHTNESS DEVICEID=\"fohmixer-1\" VALUE=100 "),
        Inbound::Other("BRIGHTNESS".into())
    );
    // Companion's answer to an unknown command answers no press.
    assert_eq!(
        classify("ERROR MESSAGE=\"Unknown command: FOO\" "),
        Inbound::Other("ERROR".into())
    );
    assert_eq!(classify(""), Inbound::Other(String::new()));
}

#[test]
fn every_inbound_kind_has_its_command_name() {
    for (inbound, name) in [
        (
            Inbound::Begin {
                companion: String::new(),
                api: String::new(),
            },
            "BEGIN",
        ),
        (Inbound::Caps(String::new()), "CAPS"),
        (Inbound::Added(Ok(())), "ADD-DEVICE"),
        (Inbound::Pressed(Ok(())), "KEY-PRESS"),
        (
            Inbound::KeyState(KeyUpdate {
                key: 0,
                img: None,
                color: None,
                pressed: None,
            }),
            "KEY-STATE",
        ),
        (Inbound::KeysClear, "KEYS-CLEAR"),
        (Inbound::Ping(String::new()), "PING"),
        (Inbound::Pong, "PONG"),
        (Inbound::Other("LOCKED-STATE".into()), "LOCKED-STATE"),
    ] {
        assert_eq!(inbound.name(), name);
    }
}

#[test]
fn a_colour_is_a_hash_and_six_hex_digits() {
    assert!(hex_colour("#00aa00"));
    assert!(hex_colour("#FFFFFF"));
    assert!(hex_colour("#0a1B2c"));
    assert!(!hex_colour("#00aa0"), "five digits");
    assert!(!hex_colour("#00aa000"), "seven digits");
    assert!(!hex_colour("00aa00"), "no hash");
    assert!(!hex_colour("#00aa0g"), "not hex");
    assert!(!hex_colour("#abc"), "short form");
    assert!(!hex_colour("#ff0000;x"));
    assert!(!hex_colour("#ffééff"), "not ascii");
    assert!(!hex_colour(""));
}

#[test]
fn a_flag_is_1_or_true() {
    assert!(flag("1") && flag("true") && flag("TRUE"));
    assert!(!flag("0") && !flag("false") && !flag("") && !flag("yes"));
}

#[test]
fn a_bounded_read_is_a_line_too_long_or_the_end() {
    assert_eq!(read_outcome(0, b""), Read::Closed);
    assert_eq!(read_outcome(5, b"PONG\n"), Read::Line("PONG".into()));
    assert_eq!(read_outcome(6, b"PONG\r\n"), Read::Line("PONG".into()));
    // A line of exactly MAX_LINE bytes and its newline: taken.
    let mut longest = vec![b'A'; MAX_LINE];
    longest.push(b'\n');
    assert_eq!(
        read_outcome(longest.len(), &longest),
        Read::Line("A".repeat(MAX_LINE))
    );
    // One byte more and no newline yet: too long.
    let over = vec![b'A'; MAX_LINE + 1];
    assert_eq!(read_outcome(over.len(), &over), Read::TooLong);
    // MAX_LINE bytes and no newline: the connection ended mid-line.
    let cut = vec![b'A'; MAX_LINE];
    assert_eq!(read_outcome(cut.len(), &cut), Read::Closed);
    assert_eq!(MAX_LINE, 262_144);
}

#[test]
fn the_api_must_be_1_12_or_a_later_1_x() {
    for ok in ["1.12.0", "1.12", "1.14.0", "1.12.3-beta"] {
        assert!(api_ok(ok), "{ok}");
    }
    for refused in ["1.11.9", "1.0.0", "2.0.0", "0.12.0", "", "1", "x.12", "1.x"] {
        assert!(!api_ok(refused), "{refused}");
    }
}

#[test]
fn the_hubs_lines() {
    assert_eq!(device_id(1), "fohmixer-1");
    assert_eq!(device_id(17), "fohmixer-17");
    assert_eq!(
        add_device("fohmixer-3", &deck(8, 4, 144)),
        "ADD-DEVICE DEVICEID=\"fohmixer-3\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
         KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=144 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n"
    );
    assert_eq!(
        add_device("fohmixer-4", &deck(5, 3, 72)),
        "ADD-DEVICE DEVICEID=\"fohmixer-4\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
         KEYS_TOTAL=15 KEYS_PER_ROW=5 BITMAPS=72 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n"
    );
    assert_eq!(
        key_press("fohmixer-3", 7, true),
        "KEY-PRESS DEVICEID=\"fohmixer-3\" KEY=7 PRESSED=1\n"
    );
    assert_eq!(
        key_press("fohmixer-3", 0, false),
        "KEY-PRESS DEVICEID=\"fohmixer-3\" KEY=0 PRESSED=0\n"
    );
    assert_eq!(
        remove_device("fohmixer-3"),
        "REMOVE-DEVICE DEVICEID=\"fohmixer-3\"\n"
    );
    assert_eq!(ping(12), "PING 12\n");
    assert_eq!(pong("abc"), "PONG abc\n");
}

#[test]
fn a_session_is_overdue_at_its_deadlines() {
    let ms = Duration::from_millis;
    assert_eq!(overdue(Phase::Begin, ms(3000), ms(9000)), None);
    assert_eq!(
        overdue(Phase::Begin, ms(3001), ms(0)),
        Some("no BEGIN from Companion within 3 s")
    );
    assert_eq!(overdue(Phase::Adding, ms(3000), ms(9000)), None);
    assert_eq!(
        overdue(Phase::Adding, ms(3001), ms(0)),
        Some("no ADD-DEVICE answer within 3 s")
    );
    assert_eq!(overdue(Phase::Up, ms(60_000), ms(5000)), None);
    assert_eq!(
        overdue(Phase::Up, ms(0), ms(5001)),
        Some("nothing from Companion for 5 s")
    );
    assert!(!link_silent(ms(5000)));
    assert!(link_silent(ms(5001)));
    assert_eq!(
        (PING_EVERY, LINK_SILENCE, ADD_TIMEOUT, STOP_BOUND),
        (ms(2000), ms(5000), ms(3000), ms(500))
    );
}

#[test]
fn the_stop_waits_for_the_remove_device_answer_a_close_or_500_ms() {
    let ms = Duration::from_millis;
    let line = |text: &str| Read::Line(text.to_string());
    // Companion's answer ends the wait at once, OK or ERROR (as 5.0.7
    // writes it, a space before the line end).
    assert_eq!(
        stop_over(
            Some(&line("REMOVE-DEVICE OK DEVICEID=\"fohmixer-1\" ")),
            ms(0)
        ),
        Some(Left::Answered(Ok(())))
    );
    assert_eq!(
        stop_over(
            Some(&line(
                "REMOVE-DEVICE ERROR DEVICEID=\"fohmixer-1\" MESSAGE=\"Device not found\" "
            )),
            ms(0)
        ),
        Some(Left::Answered(Err("Device not found".into())))
    );
    // So does its close, a failed read or a line too long (the reader ends
    // there).
    assert_eq!(stop_over(Some(&Read::Closed), ms(0)), Some(Left::Closed));
    assert_eq!(stop_over(Some(&Read::TooLong), ms(0)), Some(Left::Closed));
    assert_eq!(
        stop_over(Some(&Read::Failed("reset".into())), ms(0)),
        Some(Left::Closed)
    );
    // Any other line is dropped: the wait goes on to the bound.
    let other = line("KEY-PRESS OK DEVICEID=\"fohmixer-1\" ");
    assert_eq!(stop_over(Some(&other), ms(500)), None);
    assert_eq!(stop_over(Some(&other), ms(501)), Some(Left::Bound));
    assert_eq!(stop_over(Some(&line("")), ms(0)), None);
    // The clock alone: 500 ms is not over, 501 ms is.
    assert_eq!(stop_over(None, ms(0)), None);
    assert_eq!(stop_over(None, ms(500)), None);
    assert_eq!(stop_over(None, ms(501)), Some(Left::Bound));
}

#[test]
fn an_ignored_command_is_logged_once_per_session() {
    let mut counts = BTreeMap::new();
    assert!(first_seen(&mut counts, "BRIGHTNESS"));
    assert!(!first_seen(&mut counts, "BRIGHTNESS"));
    assert!(first_seen(&mut counts, "LOCKED-STATE"));
    assert!(!first_seen(&mut counts, "BRIGHTNESS"));
    assert_eq!(counts["BRIGHTNESS"], 3);
    assert_eq!(counts["LOCKED-STATE"], 1);
}

#[test]
fn milliseconds_between_two_instants() {
    let t = Instant::now();
    assert!((ms_between(t, t + Duration::from_millis(12)) - 12.0).abs() < 1e-9);
    assert!((ms_between(t, t + Duration::from_micros(2500)) - 2.5).abs() < 1e-9);
    assert_eq!(ms_between(t + Duration::from_millis(5), t), 0.0);
}

#[test]
fn answers_go_to_the_presses_in_the_order_they_were_sent() {
    let t = Instant::now();
    let down = Press {
        key: 3,
        down: true,
        from: Some((7, 1)),
    };
    let up = Press {
        key: 3,
        down: false,
        from: Some((7, 2)),
    };
    let release = Press {
        key: 9,
        down: false,
        from: None,
    };
    let mut fifo = Fifo::default();
    fifo.sent(down, t);
    fifo.sent(up, t + Duration::from_millis(100));
    fifo.sent(release, t + Duration::from_millis(150));
    assert_eq!(fifo.waiting(), 3);
    let first = fifo.answer(Ok(()), t + Duration::from_millis(4)).unwrap();
    assert_eq!(
        (first.press, first.ok, first.error.clone()),
        (down, true, None)
    );
    assert!((first.rtt_ms.unwrap() - 4.0).abs() < 1e-9);
    let second = fifo
        .answer(Err("Invalid KEY".into()), t + Duration::from_millis(110))
        .unwrap();
    assert_eq!(
        (second.press, second.ok, second.error.as_deref()),
        (up, false, Some("Invalid KEY"))
    );
    assert!((second.rtt_ms.unwrap() - 10.0).abs() < 1e-9);
    // The link goes: the rest is answered offline, without a round trip.
    assert_eq!(
        fifo.offline(),
        vec![Answer {
            press: release,
            ok: false,
            error: Some("offline".into()),
            rtt_ms: None
        }]
    );
    assert_eq!(fifo.waiting(), 0);
    // An answer to nothing the hub sent.
    assert_eq!(fifo.answer(Ok(()), t), None);
    assert_eq!(Answer::offline(down).error.as_deref(), Some(OFFLINE));
}
