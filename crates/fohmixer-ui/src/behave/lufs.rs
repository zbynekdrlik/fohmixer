//! A strip's loudness reading (#151, spec §6.1, §6.2; the owner's ruling of
//! 27.10 evening: like Sonible trueLevel): which point a strip shows
//! (`point_key`, the hub's `LufsItem` key), and how it looks (`look`): the
//! state, the one-decimal text and the caption come from the shared
//! `fohmixer_proto::lufs` decisions, never re-derived here. Silence shows
//! `−∞` with its caption; no reading, a problem or a strip without `+LUFS`
//! show nothing at all: no number, no caption (the row keeps its height in
//! the stylesheet).

use fohmixer_proto::layout::{Binding, LufsKind, LufsSpec, Strip};
use fohmixer_proto::lufs::{LufsProblem, LufsState, caption, judge, text};
use fohmixer_proto::path::canonical_target;

/// The key the hub sends a strip's reading under (`LufsItem`'s instance
/// and target): the strip's instance and its track's target in canonical
/// form, as `Layout::lufs_points` makes it; none for a strip without
/// `+LUFS` or a target that does not parse.
pub fn point_key(strip: &Strip) -> Option<(String, String)> {
    let _ = strip.lufs?;
    let track = Binding {
        instance: strip.binding.instance.clone(),
        anchor: strip.binding.anchor.clone(),
        path: None,
    };
    let target = track.target().ok()?;
    Some((strip.binding.instance.clone(), canonical_target(&target)))
}

/// The caption's kind as the strip's `data-kind`: `in` or `out`, none
/// without `+LUFS`.
pub fn kind_name(spec: Option<LufsSpec>) -> Option<&'static str> {
    spec.map(|s| match s.kind() {
        LufsKind::In => "in",
        LufsKind::Out => "out",
    })
}

/// How a strip's reading looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LufsLook {
    /// `data-l` (`LufsState::as_str`): the stylesheet's colour (`in`
    /// green, every other state calm).
    pub state: LufsState,
    /// The number, one decimal with a typographic minus (`−25.9`, `−∞` in
    /// silence); none while nothing shows.
    pub text: Option<String>,
    /// The caption's two lines (`LUFS` over `IN` / `OUT`), only while the
    /// number shows.
    pub caption: Option<[&'static str; 2]>,
    /// The problem's words (`no LUFS meter`): the strip's `!`.
    pub problem: Option<&'static str>,
}

impl LufsLook {
    /// Nothing shown: no reading, no caption, no problem.
    pub fn nothing() -> Self {
        Self {
            state: LufsState::None,
            text: None,
            caption: None,
            problem: None,
        }
    }
}

/// How a strip with `spec` shows the reading `tenths` and its `problem`
/// (the hub's latest `LufsItem`): a strip without `+LUFS` shows nothing
/// and has no problem; a problem shows nothing but its `!`.
pub fn look(tenths: Option<i16>, problem: Option<LufsProblem>, spec: Option<LufsSpec>) -> LufsLook {
    let Some(spec) = spec else {
        return LufsLook::nothing();
    };
    let tenths = if problem.is_some() { None } else { tenths };
    let state = judge(tenths, spec.target);
    LufsLook {
        state,
        text: tenths.map(text),
        caption: tenths.map(|_| caption(spec.kind())),
        problem: problem.map(LufsProblem::text),
    }
}

/// A character's block in the reading (#151, the owner's ruling of 29.10,
/// mockup v10 A): every character is a span of its own, so a narrow strip
/// stacks them one under another, upright, as blocks of the stylesheet's
/// heights (`strip.css`: a digit's row 0.82 em, the minus's 0.5 em, the
/// point's 0.3 em) and never through `writing-mode`, whose upright advance
/// WebKit takes from the glyph's width (the digits overlapped there). A
/// wide strip lays the same spans out on one line.
pub fn glyph_class(ch: char) -> &'static str {
    match ch {
        '\u{2212}' => "g sign",
        '.' => "g dot",
        _ => "g",
    }
}

/// The reading's characters in order, each with its block's class
/// (`glyph_class`): one span each, kept by its place, so a change of the
/// reading rewrites the spans' text and class in place and adds or drops
/// only the spans past the shorter reading (#151 review: no rebuild of
/// every span per value). Nothing shown: none.
pub fn glyphs(text: Option<&str>) -> Vec<(char, &'static str)> {
    text.unwrap_or_default()
        .chars()
        .map(|ch| (ch, glyph_class(ch)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::layout::{Anchor, Layout, StripKind};
    use serde_json::json;

    const IN: Option<LufsSpec> = Some(LufsSpec { target: Some(-25) });
    const OUT: Option<LufsSpec> = Some(LufsSpec { target: None });

    fn strip(anchor: Anchor, lufs: Option<LufsSpec>) -> Strip {
        Strip {
            binding: Binding {
                instance: "band".into(),
                anchor,
                path: Some("mixer_device volume".into()),
            },
            strip_kind: StripKind::Standard,
            wide: false,
            mute_guard: false,
            pinned: false,
            label: None,
            mark: None,
            mute_links: Vec::new(),
            reverb: false,
            fader_link: None,
            fader_follows: Vec::new(),
            lufs,
        }
    }

    #[test]
    fn look_of_an_input_point() {
        let at = |tenths: i16| look(Some(tenths), None, IN);
        assert_eq!(
            at(-259),
            LufsLook {
                state: LufsState::In,
                text: Some("\u{2212}25.9".into()),
                caption: Some(["LUFS", "IN"]),
                problem: None,
            }
        );
        // Green within ±1.0 LU of −25, both ends included; calm outside.
        assert_eq!(at(-240).state, LufsState::In);
        assert_eq!(at(-260).state, LufsState::In);
        assert_eq!(at(-239).state, LufsState::Off);
        assert_eq!(at(-239).text.as_deref(), Some("\u{2212}23.9"));
        assert_eq!(at(-261).state, LufsState::Off);
        assert_eq!(at(-330).state, LufsState::Off);
        assert_eq!(at(-330).caption, Some(["LUFS", "IN"]));
    }

    #[test]
    fn look_of_an_output_point() {
        assert_eq!(
            look(Some(-183), None, OUT),
            LufsLook {
                state: LufsState::Plain,
                text: Some("\u{2212}18.3".into()),
                caption: Some(["LUFS", "OUT"]),
                problem: None,
            }
        );
        // Even inside an input's band an output point stays calm.
        assert_eq!(look(Some(-250), None, OUT).state, LufsState::Plain);
    }

    #[test]
    fn silence_shows_minus_infinity_with_its_caption() {
        assert_eq!(
            look(Some(-700), None, IN),
            LufsLook {
                state: LufsState::Silent,
                text: Some("\u{2212}\u{221e}".into()),
                caption: Some(["LUFS", "IN"]),
                problem: None,
            }
        );
        assert_eq!(
            look(Some(-701), None, OUT),
            LufsLook {
                state: LufsState::Silent,
                text: Some("\u{2212}\u{221e}".into()),
                caption: Some(["LUFS", "OUT"]),
                problem: None,
            }
        );
        // Just above silence shows its number.
        assert_eq!(
            look(Some(-699), None, OUT).text.as_deref(),
            Some("\u{2212}69.9")
        );
    }

    #[test]
    fn no_reading_shows_nothing_and_no_caption() {
        assert_eq!(look(None, None, IN), LufsLook::nothing());
        assert_eq!(look(None, None, OUT), LufsLook::nothing());
    }

    #[test]
    fn a_problem_shows_its_mark_and_no_number() {
        assert_eq!(
            look(Some(-250), Some(LufsProblem::NoLufsMeter), IN),
            LufsLook {
                state: LufsState::None,
                text: None,
                caption: None,
                problem: Some("no LUFS meter"),
            }
        );
        assert_eq!(
            look(None, Some(LufsProblem::TwoLufsMeters), OUT).problem,
            Some("two LUFS meters")
        );
    }

    #[test]
    fn no_spec_shows_nothing() {
        let none = LufsLook {
            state: LufsState::None,
            text: None,
            caption: None,
            problem: None,
        };
        assert_eq!(look(Some(-250), None, None), none);
        assert_eq!(look(Some(-250), Some(LufsProblem::NoLufsMeter), None), none);
    }

    #[test]
    fn each_character_is_its_own_block() {
        let classes = |t: &str| t.chars().map(glyph_class).collect::<Vec<_>>();
        assert_eq!(classes("\u{2212}25.9"), ["g sign", "g", "g", "g dot", "g"]);
        assert_eq!(classes("\u{2212}\u{221e}"), ["g sign", "g"]);
        assert_eq!(classes("3.0"), ["g", "g dot", "g"]);
    }

    #[test]
    fn the_glyphs_are_the_readings_characters_in_order_with_their_blocks() {
        assert_eq!(
            glyphs(Some("\u{2212}25.9")),
            [
                ('\u{2212}', "g sign"),
                ('2', "g"),
                ('5', "g"),
                ('.', "g dot"),
                ('9', "g"),
            ]
        );
        assert_eq!(
            glyphs(Some("\u{2212}\u{221e}")),
            [('\u{2212}', "g sign"), ('\u{221e}', "g")]
        );
        assert!(glyphs(Some("")).is_empty());
        assert!(glyphs(None).is_empty());
        // A new value of the same length keeps every place: only text and
        // class change (the spans are kept by their place).
        let before = glyphs(look(Some(-259), None, IN).text.as_deref());
        let after = glyphs(look(Some(-241), None, IN).text.as_deref());
        assert_eq!(before.len(), after.len());
        assert_eq!(after[1], ('2', "g"));
        assert_eq!(after[2], ('4', "g"));
    }

    #[test]
    fn the_kind_names_the_caption() {
        assert_eq!(kind_name(IN), Some("in"));
        assert_eq!(kind_name(OUT), Some("out"));
        assert_eq!(kind_name(None), None);
    }

    #[test]
    fn the_point_key_is_the_hubs() {
        // A marker strip by index, its fader's path left out.
        assert_eq!(
            point_key(&strip(Anchor::TrackAt { index: 3 }, IN)),
            Some(("band".into(), "live_set tracks 3".into()))
        );
        assert_eq!(point_key(&strip(Anchor::TrackAt { index: 3 }, None)), None);
        // A frame strip by name: the key `Layout::lufs_points` serves.
        let named = strip(
            Anchor::Track {
                name: "Vocals".into(),
            },
            OUT,
        );
        let layout: Layout = serde_json::from_value(json!({
            "schema": 2,
            "default_page": "foh",
            "pages": [{"id": "foh", "title": "FOH", "rows": [{"sections": [
                {"kind": "group", "controls": [
                    {"kind": "strip", "strip_kind": "standard", "lufs": {},
                     "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Vocals"},
                                 "path": "mixer_device volume"}}]}]}]}]
        }))
        .expect("the layout parses");
        let points = layout.lufs_points();
        assert_eq!(points.len(), 1);
        assert_eq!(
            point_key(&named),
            Some((points[0].instance.clone(), points[0].target.clone()))
        );
        assert_eq!(
            point_key(&strip(Anchor::Master, OUT)),
            Some(("band".into(), "live_set master_track".into()))
        );
    }
}
