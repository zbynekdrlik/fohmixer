use super::*;

use std::cell::RefCell;

use bytes::Bytes;

fn reply(status: u16, body: &[u8]) -> anyhow::Result<Reply> {
    Ok(Reply {
        status,
        body: Bytes::copy_from_slice(body),
    })
}

/// A value: what the outcome says (scene, refusal, link).
fn value(f: &mut Follow, raw: &str, now: u64) -> Outcome {
    f.value(raw, now)
}

#[test]
fn the_constants_are_the_rulings() {
    assert_eq!(POLL, Duration::from_millis(500));
    assert_eq!(TIMEOUT, Duration::from_secs(2));
    assert_eq!(DOWN_AFTER, 2);
    assert_eq!(SOURCE, "companion");
    assert_eq!(REFUSED_LINES, 4);
}

#[test]
fn a_read_is_a_value_only_as_a_200_with_utf8_text() {
    assert_eq!(read_of(reply(200, b"ytslow")), Ok("ytslow".into()));
    assert_eq!(read_of(reply(200, b"")), Ok(String::new()));
    assert_eq!(
        read_of(reply(404, b"Not found")),
        Err("Companion answered 404: Not found".into())
    );
    assert_eq!(
        read_of(reply(201, b"x")),
        Err("Companion answered 201: x".into())
    );
    assert_eq!(
        read_of(reply(200, &[0xff, 0xfe])),
        Err("the value is no UTF-8 text".into())
    );
    let failed = read_of(Err(
        anyhow::anyhow!("no answer within 2000 ms").context("http://c/x")
    ));
    assert_eq!(failed, Err("http://c/x: no answer within 2000 ms".into()));
}

#[test]
fn the_first_value_only_becomes_known_and_a_change_is_a_scene() {
    let mut f = Follow::default();
    let first = value(&mut f, "intro", 1000);
    assert_eq!(
        first,
        Outcome {
            scene: None,
            refused: None,
            link: Some(Link::Up {
                failures: 0,
                down_ms: None
            }),
        }
    );
    // The same value, trimmed or not: nothing.
    assert_eq!(value(&mut f, "intro", 1500), Outcome::default());
    assert_eq!(value(&mut f, " intro\n", 2000), Outcome::default());
    // A change: a scene, trimmed.
    assert_eq!(
        value(&mut f, " talk ", 2500),
        Outcome {
            scene: Some("talk".into()),
            refused: None,
            link: None
        }
    );
    assert_eq!(value(&mut f, "talk", 3000), Outcome::default());
    // Back to the first one: a change again.
    assert_eq!(value(&mut f, "intro", 3500).scene.as_deref(), Some("intro"));
    let status = f.status();
    assert_eq!(
        status,
        SceneSourceStatus {
            on: true,
            last: Some("intro".into()),
            ok: Some(true),
            error: None,
            since: Some(1000),
        }
    );
}

#[test]
fn a_value_that_is_no_scene_name_is_refused_and_moves_nothing() {
    let mut f = Follow::default();
    value(&mut f, "intro", 0);
    assert_eq!(
        value(&mut f, "   ", 500),
        Outcome {
            scene: None,
            refused: Some((String::new(), "the scene name is empty")),
            link: None
        }
    );
    // Its repeat is no change: no second refusal.
    assert_eq!(value(&mut f, "", 1000), Outcome::default());
    let long = "x".repeat(65);
    assert_eq!(
        value(&mut f, &long, 1500).refused,
        Some((long.clone(), "the scene name is longer than 64 characters"))
    );
    assert_eq!(
        value(&mut f, "a\u{1}b", 2000).refused,
        Some(("a\u{1}b".into(), "the scene name holds a control character"))
    );
    // A good name after a refused value is a change: a scene.
    assert_eq!(value(&mut f, "intro", 2500).scene.as_deref(), Some("intro"));
    // A refused first value is said too, and the next good one is a scene.
    let mut g = Follow::default();
    let first = value(&mut g, "", 0);
    assert_eq!(
        first.refused,
        Some((String::new(), "the scene name is empty"))
    );
    assert_eq!(first.scene, None);
    assert_eq!(value(&mut g, "talk", 500).scene.as_deref(), Some("talk"));
}

#[test]
fn one_failed_read_is_no_outage_and_keeps_the_known_value() {
    let mut f = Follow::default();
    value(&mut f, "intro", 0);
    assert_eq!(f.failed("reset", 500), Outcome::default());
    // Not down: the error is not shown while Companion answers.
    assert_eq!(f.status().ok, Some(true));
    assert_eq!(f.status().error, None);
    // The next value is judged against the known one, no link change.
    assert_eq!(
        value(&mut f, "talk", 1000),
        Outcome {
            scene: Some("talk".into()),
            refused: None,
            link: None
        }
    );
    // The count started over: one more failure is still no outage.
    assert_eq!(f.failed("reset", 1500), Outcome::default());
}

#[test]
fn an_outage_is_said_once_forgets_the_known_value_and_its_end_moves_nothing() {
    let mut f = Follow::default();
    value(&mut f, "intro", 0);
    assert_eq!(f.failed("refused", 1000), Outcome::default());
    assert_eq!(
        f.failed("refused", 1500),
        Outcome {
            scene: None,
            refused: None,
            link: Some(Link::Down {
                error: "refused".into(),
                failures: 2
            }),
        }
    );
    // Once per outage.
    assert_eq!(f.failed("timeout", 2000), Outcome::default());
    assert_eq!(f.failed("timeout", 2500), Outcome::default());
    assert_eq!(
        f.status(),
        SceneSourceStatus {
            on: true,
            last: Some("intro".into()),
            ok: Some(false),
            error: Some("timeout".into()),
            since: Some(1500),
        }
    );
    // Back with the same value: up, nothing moves.
    assert_eq!(
        value(&mut f, "intro", 4000),
        Outcome {
            scene: None,
            refused: None,
            link: Some(Link::Up {
                failures: 4,
                down_ms: Some(2500)
            }),
        }
    );
    // A new value after it moves.
    assert_eq!(value(&mut f, "talk", 4500).scene.as_deref(), Some("talk"));
    // Back with ANOTHER value: still only known, nothing moves.
    f.failed("refused", 5000);
    f.failed("refused", 5500);
    let back = value(&mut f, "song", 6000);
    assert_eq!(back.scene, None);
    assert_eq!(
        back.link,
        Some(Link::Up {
            failures: 2,
            down_ms: Some(500)
        })
    );
    assert_eq!(value(&mut f, "intro", 6500).scene.as_deref(), Some("intro"));
}

#[test]
fn a_companion_down_from_the_start_is_said_and_its_first_answer_moves_nothing() {
    let mut f = Follow::default();
    assert_eq!(
        f.status(),
        SceneSourceStatus {
            on: true,
            last: None,
            ok: None,
            error: None,
            since: None,
        }
    );
    f.failed("refused", 0);
    // Not known yet: the error shows, `ok` stays none.
    assert_eq!(f.status().ok, None);
    assert_eq!(f.status().error.as_deref(), Some("refused"));
    assert!(f.failed("refused", 500).link.is_some());
    let up = value(&mut f, "intro", 1000);
    assert_eq!(up.scene, None);
    assert_eq!(
        up.link,
        Some(Link::Up {
            failures: 2,
            down_ms: Some(500)
        })
    );
}

#[test]
fn the_status_clips_a_long_value() {
    let mut f = Follow::default();
    let long = "y".repeat(300);
    value(&mut f, &long, 0);
    let last = f.status().last.unwrap();
    assert!(last.starts_with(&"y".repeat(256)), "{last}");
    assert!(last.ends_with("… (300 bytes)"), "{last}");
}

#[test]
fn the_lines_and_records_name_the_change() {
    let peer = "10.0.0.7:8000";
    let up_again = Link::Up {
        failures: 3,
        down_ms: Some(1500),
    };
    let first = Link::Up {
        failures: 0,
        down_ms: None,
    };
    let down = Link::Down {
        error: "refused".into(),
        failures: 2,
    };
    assert_eq!(
        link_line(&up_again, peer),
        "scenes: Companion at 10.0.0.7:8000 answers again after 1500 ms, 3 failed reads"
    );
    assert_eq!(
        link_line(&first, peer),
        "scenes: Companion at 10.0.0.7:8000 answers, 0 failed reads before"
    );
    assert_eq!(
        link_line(&down, peer),
        "scenes: Companion at 10.0.0.7:8000 is DOWN after 2 failed reads: refused"
    );
    assert_eq!(
        link_fields(&up_again, peer),
        json!({"up": true, "peer": peer, "failures": 3, "down_ms": 1500})
    );
    assert_eq!(
        link_fields(&first, peer),
        json!({"up": true, "peer": peer, "failures": 0, "down_ms": null})
    );
    assert_eq!(
        link_fields(&down, peer),
        json!({"up": false, "peer": peer, "failures": 2, "error": "refused"})
    );
    assert_eq!(
        refused_line("a b", "why", 2),
        "scenes: Companion's value \"a b\" is no scene (why); 2 left out before"
    );
}

#[test]
fn an_outcome_is_recorded_rationed_and_its_scene_handed_on() {
    let (events, records) = EventLog::channel(64);
    let source = SceneSource::new("http://c/x".into(), "c:8000".into());
    let mut refusals = Ration::default();
    let scenes = RefCell::new(Vec::new());
    let take = |name: String| scenes.borrow_mut().push(name);
    // The first answer: up, no scene.
    let outcome = source.heard(Ok("intro".into()), 0);
    act(&source, &events, &mut refusals, outcome, &take);
    // A change: the scene goes on.
    let outcome = source.heard(Ok("talk".into()), 500);
    act(&source, &events, &mut refusals, outcome, &take);
    assert_eq!(*scenes.borrow(), vec!["talk".to_string()]);
    // Refusals: at most REFUSED_LINES per gap, the rest counted.
    for (i, bad) in ["", "a\u{1}", " ", "b\u{2}", "c\u{3}", "d\u{4}"]
        .iter()
        .enumerate()
    {
        let outcome = source.heard(Ok((*bad).into()), 1000 + i as u64);
        act(&source, &events, &mut refusals, outcome, &take);
    }
    // Down after two failures, one record.
    for at in [2000, 2500, 3000] {
        let outcome = source.heard(Err("refused".into()), at);
        act(&source, &events, &mut refusals, outcome, &take);
    }
    let got: Vec<Value> = records.try_iter().collect();
    let kinds: Vec<&Value> = got.iter().map(|r| &r["ev"]).collect();
    assert!(kinds.iter().all(|k| *k == "scene_source"), "{got:?}");
    assert_eq!(got[0]["up"], true);
    assert_eq!(got[0]["peer"], "c:8000");
    let refused: Vec<&Value> = got.iter().filter(|r| r.get("refused").is_some()).collect();
    assert_eq!(refused.len(), REFUSED_LINES as usize, "{got:?}");
    assert_eq!(refused[0]["refused"], "the scene name is empty");
    assert_eq!(refused[0]["value"], "");
    assert_eq!(refused[0]["skipped"], 0);
    assert_eq!(refused[1]["value"], "a\u{1}");
    assert_eq!(got.last().unwrap()["up"], false);
    assert_eq!(got.last().unwrap()["failures"], 2);
    assert_eq!(got.len(), 1 + REFUSED_LINES as usize + 1, "{got:?}");
    assert_eq!(*scenes.borrow(), vec!["talk".to_string()]);
    assert_eq!(source.status().ok, Some(false));
    // A refused value's ration counts what it left out on the next line.
    refusals.expire();
    let outcome = source.heard(Ok("e\u{5}".into()), 4000);
    act(&source, &events, &mut refusals, outcome, &take);
    let next: Vec<Value> = records.try_iter().collect();
    let line = next
        .iter()
        .find(|r| r.get("refused").is_some())
        .expect("a refusal record");
    assert_eq!(line["skipped"], 2);
}
