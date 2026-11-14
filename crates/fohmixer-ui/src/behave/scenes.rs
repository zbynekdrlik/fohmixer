//! The scene automation on the tablet (#207; the owner's approved mockup
//! `docs/mockups/scenes-v3.html`): what the channel detail's SCENES shows
//! and what its keys ask the hub for, the column foot's line and the edge's
//! flash. Pure; `components/detail_scenes.rs` and the surface carry it out.

use fohmixer_proto::scenes::{DEFAULT_TIME_MS, SceneMute, SceneRule, ScenesDoc};
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::volume::{FLOOR_DB, TOP_DB, value2db};

/// The lowest dB a − step reaches before −∞ (the mockup's −60).
pub const DB_LOW: f64 = -60.0;
/// The glide times a − / + steps through, ms (0.5 to 10 s).
pub const TIMES_MS: [u32; 7] = [500, 1000, 2000, 3000, 5000, 8000, 10000];

/// Whether a scene flashes the mixer's edge: it came just now (never the
/// last one told at a connection) and moved a fader.
pub fn flashes(live: bool, moved: u32) -> bool {
    live && moved > 0
}

/// The column foot's line: `scene <name> · HH:MM` (local time).
pub fn foot_text(name: &str, hours: u32, minutes: u32) -> String {
    format!("scene {name} · {hours:02}:{minutes:02}")
}

/// `strip`'s rows: each scene it has a rule for, in the order the hub
/// first heard them (a name it never heard last, by name).
pub fn rows(doc: &ScenesDoc, strip: &StripRef) -> Vec<(String, SceneRule)> {
    let mut rows: Vec<(String, SceneRule)> = doc.of(strip).into_iter().collect();
    let place = |name: &str| {
        doc.known
            .iter()
            .position(|k| k == name)
            .unwrap_or(usize::MAX)
    };
    rows.sort_by(|(a, _), (b, _)| place(a).cmp(&place(b)).then_with(|| a.cmp(b)));
    rows
}

/// The scenes ADD SCENE offers: every scene heard that `strip` has no rule
/// for, in the order heard.
pub fn addable(doc: &ScenesDoc, strip: &StripRef) -> Vec<String> {
    let has = doc.of(strip);
    doc.known
        .iter()
        .filter(|k| !has.contains_key(*k))
        .cloned()
        .collect()
}

/// A fader's dB now as a rule takes it (`= FADER`, a new row): Live's
/// volume on the strips' law to 0.1 dB, −∞ as the floor; none without a
/// value.
pub fn fader_db(volume: Option<f64>) -> Option<f64> {
    let db = value2db(volume?);
    if db <= FLOOR_DB {
        return Some(FLOOR_DB);
    }
    Some(((db * 10.0).round() / 10.0).clamp(FLOOR_DB, TOP_DB))
}

/// A − step: down to the next whole dB, past [`DB_LOW`] −∞ (none counts as
/// 0 dB, as the mockup).
pub fn db_down(db: Option<f64>) -> f64 {
    let next = db.unwrap_or(0.0).ceil() - 1.0;
    if next < DB_LOW { FLOOR_DB } else { next }
}

/// A + step: up to the next whole dB, −∞ to [`DB_LOW`], at most +6 dB.
pub fn db_up(db: Option<f64>) -> f64 {
    let db = db.unwrap_or(0.0);
    if db < DB_LOW {
        return DB_LOW;
    }
    (db.floor() + 1.0).min(TOP_DB)
}

/// A dB as SCENES shows it: `—` for none, `−∞`, else one decimal with its
/// sign (a typographic minus).
pub fn db_text(db: Option<f64>) -> String {
    match db {
        None => "—".to_string(),
        Some(db) if db <= FLOOR_DB => "−∞ dB".to_string(),
        Some(db) => {
            // Its sign by the value shown (−0.04 reads 0.0, never −0.0).
            let shown = (db * 10.0).round() / 10.0;
            if shown > 0.0 {
                format!("+{shown:.1} dB")
            } else if shown < 0.0 {
                format!("{shown:.1} dB").replace('-', "−")
            } else {
                "0.0 dB".to_string()
            }
        }
    }
}

/// A − step of the time: the step below it (the first stays).
pub fn time_down(ms: u32) -> u32 {
    TIMES_MS
        .iter()
        .rev()
        .find(|t| **t < ms)
        .copied()
        .unwrap_or(TIMES_MS[0])
}

/// A + step of the time: the step above it (the last stays).
pub fn time_up(ms: u32) -> u32 {
    TIMES_MS
        .iter()
        .find(|t| **t > ms)
        .copied()
        .unwrap_or(TIMES_MS[TIMES_MS.len() - 1])
}

/// A time as SCENES shows it: `0.5 s`, `2 s`.
pub fn time_text(ms: u32) -> String {
    if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{} s", f64::from(ms) / 1000.0)
    }
}

/// The MUTE chip's next state: — → MUTE → UNMUTE → —; a row without a dB
/// never goes to — (it would do nothing, and the hub removes such a rule):
/// MUTE ↔ UNMUTE.
pub fn mute_next(mute: Option<SceneMute>, has_db: bool) -> Option<SceneMute> {
    match mute {
        None => Some(SceneMute::Mute),
        Some(SceneMute::Mute) => Some(SceneMute::Unmute),
        Some(SceneMute::Unmute) if has_db => None,
        Some(SceneMute::Unmute) => Some(SceneMute::Mute),
    }
}

/// The MUTE chip's word.
pub fn mute_word(mute: Option<SceneMute>) -> &'static str {
    mute.map_or("MUTE —", SceneMute::word)
}

/// A new row's rule: the fader's dB now (0 dB without one), no mute, 2 s.
pub fn new_rule(fader: Option<f64>) -> SceneRule {
    SceneRule {
        db: Some(fader.unwrap_or(0.0)),
        mute: None,
        time_ms: DEFAULT_TIME_MS,
    }
}

/// Whether the page keeps the last scene it was told after a new document:
/// only while the document knows its name (the hub forgets a last scene a
/// hand edit drops, so the foot's line goes too).
pub fn keeps_last(doc: &ScenesDoc, name: &str) -> bool {
    doc.knows(name)
}

/// Whether `scene` is the one that came last (its row is marked).
pub fn is_last(last: Option<&str>, scene: &str) -> bool {
    last == Some(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::scenes::heard;
    use fohmixer_proto::volume::volume_of;

    fn band(label: &str) -> StripRef {
        StripRef {
            instance: "band".into(),
            label: label.into(),
        }
    }

    fn rule(db: Option<f64>) -> SceneRule {
        SceneRule {
            db,
            mute: None,
            time_ms: DEFAULT_TIME_MS,
        }
    }

    /// `intro`, `talk`, `song` heard; Vox 1 has rules for song and intro.
    fn doc() -> ScenesDoc {
        let doc = ["intro", "talk", "song"]
            .iter()
            .fold(ScenesDoc::empty(), |d, s| heard(&d, s).unwrap());
        doc.with(&band("Vox 1"), "song", Some(rule(Some(-3.0))))
            .with(&band("Vox 1"), "intro", Some(rule(Some(0.0))))
            .with(&band("Vox 1"), "zzz", Some(rule(Some(1.0))))
            .with(&band("Vox 1"), "aaa", Some(rule(Some(1.0))))
    }

    #[test]
    fn only_a_live_scene_that_moved_flashes() {
        assert!(flashes(true, 1));
        assert!(!flashes(true, 0));
        assert!(!flashes(false, 3));
    }

    #[test]
    fn the_foot_names_the_scene_and_its_time() {
        assert_eq!(foot_text("intro", 9, 5), "scene intro · 09:05");
        assert_eq!(foot_text("talk", 19, 0), "scene talk · 19:00");
    }

    #[test]
    fn rows_follow_the_order_heard_then_names() {
        let names: Vec<String> = rows(&doc(), &band("Vox 1"))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["intro", "song", "aaa", "zzz"]);
        assert_eq!(rows(&doc(), &band("Bass")), Vec::new());
    }

    #[test]
    fn add_offers_the_scenes_without_a_row() {
        assert_eq!(addable(&doc(), &band("Vox 1")), vec!["talk"]);
        assert_eq!(
            addable(&doc(), &band("Bass")),
            vec!["intro", "talk", "song"]
        );
        assert_eq!(
            addable(&ScenesDoc::empty(), &band("Bass")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_faders_db_is_live_s_to_a_tenth_and_the_floor_for_silence() {
        assert_eq!(fader_db(None), None);
        assert_eq!(fader_db(Some(0.0)), Some(FLOOR_DB));
        assert_eq!(fader_db(Some(0.85)), Some(0.0));
        assert_eq!(fader_db(Some(1.0)), Some(TOP_DB));
        assert_eq!(fader_db(Some(volume_of(-10.04))), Some(-10.0));
        assert_eq!(fader_db(Some(volume_of(-10.06))), Some(-10.1));
        assert_eq!(fader_db(Some(1e-9)), Some(FLOOR_DB));
    }

    #[test]
    fn the_db_steps_are_whole_db_to_minus_60_then_minus_infinity() {
        assert_eq!(DB_LOW, -60.0);
        assert_eq!(db_down(None), -1.0);
        assert_eq!(db_down(Some(0.0)), -1.0);
        assert_eq!(db_down(Some(-10.4)), -11.0);
        assert_eq!(db_down(Some(-10.0)), -11.0);
        assert_eq!(db_down(Some(-59.0)), -60.0);
        assert_eq!(db_down(Some(-59.5)), -60.0);
        assert_eq!(db_down(Some(-60.0)), FLOOR_DB);
        assert_eq!(db_down(Some(FLOOR_DB)), FLOOR_DB);
        assert_eq!(db_up(None), 1.0);
        assert_eq!(db_up(Some(-10.4)), -10.0);
        assert_eq!(db_up(Some(FLOOR_DB)), -60.0);
        assert_eq!(db_up(Some(-60.0)), -59.0);
        assert_eq!(db_up(Some(5.5)), 6.0);
        assert_eq!(db_up(Some(6.0)), 6.0);
    }

    #[test]
    fn a_db_reads_with_its_sign() {
        assert_eq!(db_text(None), "—");
        assert_eq!(db_text(Some(FLOOR_DB)), "−∞ dB");
        assert_eq!(db_text(Some(0.0)), "0.0 dB");
        assert_eq!(db_text(Some(2.5)), "+2.5 dB");
        assert_eq!(db_text(Some(-10.0)), "−10.0 dB");
        assert_eq!(db_text(Some(-69.9)), "−69.9 dB");
        assert_eq!(db_text(Some(0.04)), "0.0 dB");
        assert_eq!(db_text(Some(-0.04)), "0.0 dB");
        assert_eq!(db_text(Some(0.05)), "+0.1 dB");
        assert_eq!(db_text(Some(-0.06)), "−0.1 dB");
    }

    #[test]
    fn the_time_steps_through_its_list() {
        assert_eq!(TIMES_MS, [500, 1000, 2000, 3000, 5000, 8000, 10000]);
        assert_eq!(time_down(2000), 1000);
        assert_eq!(time_down(500), 500);
        assert_eq!(time_down(0), 500);
        assert_eq!(time_down(2500), 2000);
        assert_eq!(time_up(2000), 3000);
        assert_eq!(time_up(10000), 10000);
        assert_eq!(time_up(60000), 10000);
        assert_eq!(time_up(2500), 3000);
        assert_eq!(time_text(500), "0.5 s");
        assert_eq!(time_text(2000), "2 s");
        assert_eq!(time_text(10000), "10 s");
        assert_eq!(time_text(1500), "1.5 s");
    }

    #[test]
    fn the_mute_chip_cycles_and_never_empties_a_row() {
        assert_eq!(mute_next(None, true), Some(SceneMute::Mute));
        assert_eq!(
            mute_next(Some(SceneMute::Mute), true),
            Some(SceneMute::Unmute)
        );
        assert_eq!(mute_next(Some(SceneMute::Unmute), true), None);
        assert_eq!(
            mute_next(Some(SceneMute::Unmute), false),
            Some(SceneMute::Mute)
        );
        assert_eq!(mute_next(None, false), Some(SceneMute::Mute));
        assert_eq!(mute_word(None), "MUTE —");
        assert_eq!(mute_word(Some(SceneMute::Mute)), "MUTE");
        assert_eq!(mute_word(Some(SceneMute::Unmute)), "UNMUTE");
    }

    #[test]
    fn a_new_row_takes_the_fader_and_two_seconds() {
        assert_eq!(new_rule(Some(-6.5)), rule(Some(-6.5)));
        assert_eq!(new_rule(None), rule(Some(0.0)));
    }

    #[test]
    fn the_last_scene_stays_while_the_document_knows_it() {
        assert!(keeps_last(&doc(), "talk"));
        assert!(!keeps_last(&doc(), "gone"));
        assert!(!keeps_last(&ScenesDoc::empty(), "talk"));
    }

    #[test]
    fn the_last_scene_is_marked() {
        assert!(is_last(Some("intro"), "intro"));
        assert!(!is_last(Some("intro"), "talk"));
        assert!(!is_last(None, "intro"));
    }
}
