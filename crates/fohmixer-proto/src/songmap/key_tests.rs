use super::*;

#[test]
fn every_accepted_root_parses_to_its_index() {
    let cases = [
        ("C", 0),
        ("C#", 1),
        ("Db", 1),
        ("D", 2),
        ("D#", 3),
        ("Eb", 3),
        ("E", 4),
        ("F", 5),
        ("F#", 6),
        ("Gb", 6),
        ("G", 7),
        ("G#", 8),
        ("Ab", 8),
        ("A", 9),
        ("A#", 10),
        ("Bb", 10),
        ("B", 11),
    ];
    for (t, k) in cases {
        assert_eq!(
            parse_token(t),
            Ok(Key {
                root: k,
                minor: false
            }),
            "{t}"
        );
    }
}

#[test]
fn a_trailing_m_is_minor() {
    assert_eq!(
        parse_token("F#m"),
        Ok(Key {
            root: 6,
            minor: true
        })
    );
    assert_eq!(
        parse_token("Am"),
        Ok(Key {
            root: 9,
            minor: true
        })
    );
    assert_eq!(
        parse_token("Bbm"),
        Ok(Key {
            root: 10,
            minor: true
        })
    );
}

#[test]
fn a_double_spelling_of_one_key_is_that_key() {
    assert_eq!(
        parse_token("F#/Gb"),
        Ok(Key {
            root: 6,
            minor: false
        })
    );
    assert_eq!(
        parse_token("Gb/F#"),
        Ok(Key {
            root: 6,
            minor: false
        })
    );
    assert_eq!(
        parse_token("C#m/Dbm"),
        Ok(Key {
            root: 1,
            minor: true
        })
    );
}

#[test]
fn a_double_spelling_of_two_keys_is_refused() {
    assert_eq!(parse_token("F/G"), Err(KeyError::TwoKeys));
    assert_eq!(parse_token("Am/A"), Err(KeyError::TwoKeys));
    assert_eq!(parse_token("C/C/C"), Err(KeyError::TwoKeys));
    // A spelling that does not read is its own problem.
    assert_eq!(parse_token("F#/H"), Err(KeyError::Letter));
    assert_eq!(parse_token("F#/"), Err(KeyError::Empty));
}

#[test]
fn rejected_spellings_are_problems() {
    for t in [
        "", "c", "H", "Fis", "Es", "Amin", "Amol", "Amoll", "Amaj", "Adur", "AM", "A m", "F♯",
        "B♭", "E#", "B#", "Cb", "Fb", "Cx", "C##",
    ] {
        assert!(parse_token(t).is_err(), "{t}");
    }
    assert_eq!(parse_token(""), Err(KeyError::Empty));
    assert_eq!(parse_token("H"), Err(KeyError::Letter));
    assert_eq!(parse_token("c"), Err(KeyError::Letter));
    assert_eq!(parse_token("E#"), Err(KeyError::Accidental));
    assert_eq!(parse_token("B#"), Err(KeyError::Accidental));
    assert_eq!(parse_token("Cb"), Err(KeyError::Accidental));
    assert_eq!(parse_token("Fbm"), Err(KeyError::Accidental));
    assert_eq!(parse_token("Amin"), Err(KeyError::Suffix));
    assert_eq!(parse_token("C##"), Err(KeyError::Suffix));
    assert_eq!(parse_token("F♯"), Err(KeyError::Suffix));
    assert_eq!(parse_token("Fis"), Err(KeyError::Suffix));
}

#[test]
fn every_problem_has_its_words() {
    assert_eq!(KeyError::Empty.text(), "no key");
    assert_eq!(
        KeyError::Letter.text(),
        "a key starts with C, D, E, F, G, A or B"
    );
    assert_eq!(
        KeyError::Accidental.text(),
        "E#, B#, Cb and Fb are not keys"
    );
    assert_eq!(KeyError::Suffix.text(), "only m may follow the key");
    assert_eq!(
        KeyError::TwoKeys.text(),
        "the two spellings name different keys"
    );
    assert_eq!(
        serde_json::to_value(KeyError::TwoKeys).unwrap(),
        serde_json::json!("twokeys")
    );
}

#[test]
fn names_are_autotunes_spelling() {
    let names: Vec<String> = (0..12)
        .map(|r| {
            Key {
                root: r,
                minor: false,
            }
            .name()
        })
        .collect();
    assert_eq!(
        names,
        [
            "C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B"
        ]
    );
    assert_eq!(
        Key {
            root: 6,
            minor: true
        }
        .name(),
        "F#m"
    );
    assert_eq!(
        Key {
            root: 12,
            minor: false
        }
        .name(),
        "?"
    );
}

#[test]
fn autotune_values_are_k_over_11_and_s_over_28() {
    assert_eq!(
        Key {
            root: 0,
            minor: false
        }
        .autotune_key(),
        0.0
    );
    assert_eq!(
        Key {
            root: 11,
            minor: false
        }
        .autotune_key(),
        1.0
    );
    assert!(
        (Key {
            root: 5,
            minor: false
        }
        .autotune_key()
            - 5.0 / 11.0)
            .abs()
            < 1e-12
    );
    assert_eq!(
        Key {
            root: 5,
            minor: false
        }
        .autotune_scale(),
        0.0
    );
    assert!(
        (Key {
            root: 5,
            minor: true
        }
        .autotune_scale()
            - 1.0 / 28.0)
            .abs()
            < 1e-12
    );
    assert_eq!(
        Key {
            root: 5,
            minor: true
        }
        .live_scale_name(),
        "Minor"
    );
    assert_eq!(
        Key {
            root: 5,
            minor: false
        }
        .live_scale_name(),
        "Major"
    );
}

#[test]
fn read_back_indexes_round_half_up_at_the_boundaries() {
    assert_eq!(key_index_of(0.5), 6); // (5 + 0.5) / 11 = 0.5 shows F#
    assert_eq!(key_index_of(0.499886), 5);
    assert_eq!(key_index_of(1.0), 11);
    assert_eq!(key_index_of(0.0), 0);
    assert_eq!(key_index_of(5.0 / 11.0), 5);
    assert_eq!(key_index_of(1.5), 11, "clamped");
    assert_eq!(key_index_of(-0.5), 0, "clamped");
    assert_eq!(key_index_of(f64::NAN), 0);
    assert_eq!(scale_index_of(0.0), 0);
    assert_eq!(scale_index_of(0.035714), 1);
    assert_eq!(scale_index_of(0.071429), 2); // Chromatic: a mismatch for the hub
    assert_eq!(scale_index_of(0.5 / 28.0), 1); // the boundary (0 + 0.5)/28 shows Minor
    assert_eq!(scale_index_of(0.017857), 0, "just under the boundary");
    assert_eq!(scale_index_of(1.0), 28);
    assert_eq!(scale_index_of(2.0), 28, "clamped");
    assert_eq!(KEY_STEPS, 11);
    assert_eq!(SCALE_STEPS, 28);
}
