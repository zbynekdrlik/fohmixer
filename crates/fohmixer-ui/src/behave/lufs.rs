//! A strip's loudness reading (#151, spec §6.1, §6.2; mockup v9): which
//! point a strip shows (`point_key`, the hub's `LufsItem` key), and how it
//! looks (`look`): the state, the integer and the caption come from the
//! shared `fohmixer_proto::lufs` decisions, never re-derived here. Silence,
//! no reading, a problem or a strip without `+LUFS` show nothing at all:
//! no number, no caption (the row keeps its height in the stylesheet).

use fohmixer_proto::layout::{Binding, LufsKind, LufsSpec, Strip};
use fohmixer_proto::lufs::{LufsProblem, LufsState, caption, judge, shown};
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
    /// `data-l` (`LufsState::as_str`): the stylesheet's tint.
    pub state: LufsState,
    /// The number, with a typographic minus (`−25`); none while nothing
    /// shows.
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

/// The integer with a typographic minus (`−25`, `0`, `3`).
pub fn number_text(n: i16) -> String {
    if n < 0 {
        format!("\u{2212}{}", n.unsigned_abs())
    } else {
        n.to_string()
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
    let showing = !matches!(state, LufsState::Silent | LufsState::None);
    LufsLook {
        state,
        text: tenths.filter(|_| showing).map(|t| number_text(shown(t))),
        caption: showing.then(|| caption(spec.kind())),
        problem: problem.map(LufsProblem::text),
    }
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
            at(-250),
            LufsLook {
                state: LufsState::Fine,
                text: Some("\u{2212}25".into()),
                caption: Some(["LUFS", "IN"]),
                problem: None,
            }
        );
        // The tint is judged on the shown integer (the proto's `judge`).
        assert_eq!(at(-210).state, LufsState::Warn);
        assert_eq!(at(-210).text.as_deref(), Some("\u{2212}21"));
        assert_eq!(at(-300).state, LufsState::Warn);
        assert_eq!(at(-170).state, LufsState::Far);
        assert_eq!(at(-330).state, LufsState::Far);
        assert_eq!(at(-330).text.as_deref(), Some("\u{2212}33"));
        assert_eq!(at(-215).text.as_deref(), Some("\u{2212}22"));
        assert_eq!(at(-215).state, LufsState::Fine);
    }

    #[test]
    fn look_of_an_output_point() {
        assert_eq!(
            look(Some(-183), None, OUT),
            LufsLook {
                state: LufsState::Plain,
                text: Some("\u{2212}18".into()),
                caption: Some(["LUFS", "OUT"]),
                problem: None,
            }
        );
    }

    #[test]
    fn silent_shows_nothing_and_no_caption() {
        let nothing = |state| LufsLook {
            state,
            text: None,
            caption: None,
            problem: None,
        };
        assert_eq!(look(Some(-700), None, IN), nothing(LufsState::Silent));
        assert_eq!(look(Some(-700), None, OUT), nothing(LufsState::Silent));
        assert_eq!(look(None, None, IN), nothing(LufsState::None));
        assert_eq!(look(None, None, OUT), nothing(LufsState::None));
        // Just above silence shows.
        assert_eq!(
            look(Some(-699), None, OUT).text.as_deref(),
            Some("\u{2212}70")
        );
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
    fn the_text_uses_a_typographic_minus() {
        assert_eq!(number_text(-25), "\u{2212}25");
        assert_eq!(number_text(-1), "\u{2212}1");
        assert_eq!(number_text(0), "0");
        assert_eq!(number_text(3), "3");
        assert_eq!(number_text(i16::MIN), "\u{2212}32768");
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
