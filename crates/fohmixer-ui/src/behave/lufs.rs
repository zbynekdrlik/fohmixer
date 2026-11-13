//! A strip's loudness reading (#151, spec §6.1, §6.2; the owner's ruling of
//! 27.10 evening: like Sonible trueLevel): which point a strip shows
//! (`point_key`, the hub's `LufsItem` key), and how it looks (`look`): the
//! state, the one-decimal text and the caption come from the shared
//! `fohmixer_proto::lufs` decisions, never re-derived here. Silence shows
//! `−∞` with its caption; no reading, a problem or a strip without `+LUFS`
//! show nothing at all: no number, no caption (the row keeps its height in
//! the stylesheet). An input point also has its spring (`spring`, mockup
//! v13): the pause-skipping `V` on the meter's scale.

use fohmixer_proto::layout::{Binding, Layout, LufsKind, LufsSpec, Strip};
use fohmixer_proto::lufs::{LufsProblem, LufsState, caption, in_band, judge, text};
use fohmixer_proto::path::canonical_target;
use fohmixer_proto::sectors::{SectorLayout, pool};

use super::fader::VolumeLaw;
use super::login::one_instance;
use super::scale::tick_pos;

/// The key the hub sends a strip's reading under (`LufsItem`'s instance
/// and target): the strip's instance and its track's target in canonical
/// form, as `Layout::lufs_points` makes it; none for a strip without
/// `+LUFS` or a target that does not parse.
pub fn point_key(strip: &Strip) -> Option<(String, String)> {
    let _ = strip.lufs?;
    track_key(&strip.binding)
}

/// The key of the point on `binding`'s track (its instance and the track's
/// canonical target, its path left out): a strip's and a `lufs_readout`'s
/// (#184) point; none while it names no track (a `+CTRL` id no Tuner
/// filled).
pub fn track_key(binding: &Binding) -> Option<(String, String)> {
    let track = Binding {
        instance: binding.instance.clone(),
        anchor: binding.anchor.clone(),
        path: None,
    };
    let target = track.target().ok()?;
    Some((binding.instance.clone(), canonical_target(&target)))
}

/// Whether a `lufs_readout` of the Live set `instance` shows (#184, mockup
/// v3): always on a page whose faders are one Live set's (`one_instance`,
/// the MBC hub's), else only while a picked layout holds a fader of
/// `instance` (the FOH page with a sector paged to the MBC faders).
pub fn stream_shown(instance: &str, picked: &[&SectorLayout], one_instance: bool) -> bool {
    one_instance
        || picked
            .iter()
            .any(|layout| layout.refs().any(|s| s.instance == instance))
}

/// Whether `layout` is a page of one Live set's faders (#184: the MBC
/// hub's), where a readout always shows, at its full size: a pool of at
/// least one fader, all of one Live set. An empty pool (no marker read yet,
/// the Lives down) is none: the FOH page then shows no readout of its own.
pub fn stream_page(layout: &Layout) -> bool {
    !pool(layout).is_empty() && one_instance(layout)
}

/// A readout's target line (#184, mockup v3, in English, #141): a target
/// is always below 0 (`lufs_target_ok`), with a typographic minus.
pub fn target_text(target: i8) -> String {
    format!("target \u{2212}{}", target.unsigned_abs())
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

/// The spring's coil in its `0 0 6 100` box (mockup v13): stretched by its
/// height, it keeps its turns.
pub const COIL: &str = "3,0 6,5 0,15 6,25 0,35 6,45 0,55 6,65 0,75 6,85 3,100";

/// The spring's body (mockup v13): from the target to the reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringBody {
    /// The reading's line, percent from the zone's top.
    pub now: f64,
    /// The coil's top and height, percent of the zone.
    pub top: f64,
    pub height: f64,
    /// Inside the target's band: green.
    pub in_band: bool,
}

/// An input point's spring on the meter's own scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringLook {
    /// The target's line, percent from the zone's top.
    pub goal: f64,
    /// None in silence: the target's line alone.
    pub body: Option<SpringBody>,
}

impl SpringLook {
    /// `data-l`: `in`, `off`, `silent` without a body.
    pub fn state(&self) -> &'static str {
        match self.body {
            Some(b) if b.in_band => "in",
            Some(_) => "off",
            None => "silent",
        }
    }

    /// The stylesheet's positions.
    pub fn vars(&self) -> String {
        let body = self.body.unwrap_or(SpringBody {
            now: self.goal,
            top: self.goal,
            height: 0.0,
            in_band: false,
        });
        format!(
            "--goal:{:.3}%;--now:{:.3}%;--top:{:.3}%;--h:{:.3}%;",
            self.goal, body.now, body.top, body.height
        )
    }
}

/// Percent from the zone's top of `tenths` LU read on the dB scale.
fn top_of(tenths: i16, law: VolumeLaw) -> f64 {
    (1.0 - tick_pos(law, f64::from(tenths) / 10.0)) * 100.0
}

/// The spring of a point with `spec` and the hub's `voice` (`V`): none
/// without a target or a value (mockup v13, the owner's sketch: a line at
/// the target, a spring to a line at the reading; the number stays `S`).
pub fn spring(voice: Option<i16>, spec: Option<LufsSpec>, law: VolumeLaw) -> Option<SpringLook> {
    let target = spec?.target?;
    let voice = voice?;
    let goal = top_of(i16::from(target) * 10, law);
    let body = (judge(Some(voice), Some(target)) != LufsState::Silent).then(|| {
        let now = top_of(voice, law);
        SpringBody {
            now,
            top: goal.min(now),
            height: (now - goal).abs(),
            in_band: in_band(voice, target),
        }
    });
    Some(SpringLook { goal, body })
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

    fn layout_of(strips: &[(&str, &str)]) -> SectorLayout {
        SectorLayout {
            id: "rd-1".into(),
            title: "RD".into(),
            strips: strips
                .iter()
                .map(|(instance, label)| {
                    Some(fohmixer_proto::sectors::StripRef {
                        instance: (*instance).into(),
                        label: (*label).into(),
                    })
                })
                .chain([None])
                .collect(),
        }
    }

    #[test]
    fn stream_lufs_shows_on_one_instance_or_while_a_sector_shows_its_faders() {
        let foh = layout_of(&[("band", "Vox 1"), ("master", "Hand1 #")]);
        let mbc = layout_of(&[("mbc", "Hand 1"), ("mbc", "Choir")]);
        let mixed = layout_of(&[("band", "Vox 1"), ("mbc", "Choir")]);
        // The MBC hub's page: shown with nothing picked.
        assert!(stream_shown("mbc", &[], true));
        assert!(stream_shown("mbc", &[&foh], true));
        // The FOH page: RD paged to the MBC faders, then back.
        assert!(stream_shown("mbc", &[&foh, &mbc], false));
        assert!(stream_shown("mbc", &[&mixed], false));
        assert!(!stream_shown("mbc", &[&foh, &foh], false));
        assert!(!stream_shown("mbc", &[], false));
        assert!(!stream_shown("band", &[&mbc], false));
    }

    #[test]
    fn a_stream_page_is_a_pool_of_one_live_sets_faders_and_never_empty() {
        let page = |strips: Vec<(&str, &str)>| -> Layout {
            let controls: Vec<serde_json::Value> = strips
                .into_iter()
                .map(|(instance, label)| {
                    json!({"kind": "strip", "strip_kind": "standard", "label": label,
                        "binding": {"instance": instance, "anchor": {"kind": "track_at", "index": 0}}})
                })
                .collect();
            serde_json::from_value(json!({"schema": 2, "default_page": "p", "pages": [
                {"id": "p", "title": "P", "rows": [{"sections": [
                    {"kind": "group", "id": "g", "controls": controls}]}]}]}))
            .unwrap()
        };
        assert!(stream_page(&page(vec![
            ("mbc", "Hand 1"),
            ("mbc", "Choir")
        ])));
        assert!(stream_page(&page(vec![("mbc", "Hand 1")])));
        assert!(!stream_page(&page(vec![
            ("band", "Vox 1"),
            ("mbc", "Choir")
        ])));
        assert!(!stream_page(&page(vec![])));
    }

    #[test]
    fn a_readouts_key_is_its_tracks_point_and_its_target_line_is_english() {
        let by_index = Binding {
            instance: "mbc".into(),
            anchor: Anchor::TrackAt { index: 8 },
            path: None,
        };
        assert_eq!(
            track_key(&by_index),
            Some(("mbc".into(), "live_set tracks 8".into()))
        );
        let unfilled = Binding {
            instance: "mbc".into(),
            anchor: Anchor::Ctrl {
                id: fohmixer_proto::layout::CtrlId::Stream,
            },
            path: None,
        };
        assert_eq!(track_key(&unfilled), None);
        assert_eq!(target_text(-14), "target \u{2212}14");
        assert_eq!(target_text(-40), "target \u{2212}40");
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

    const LAW: VolumeLaw = VolumeLaw::Live;

    #[test]
    fn the_spring_stands_on_the_meters_own_scale() {
        let at = |db: f64| (1.0 - tick_pos(LAW, db)) * 100.0;
        let quiet = spring(Some(-340), IN, LAW).expect("a spring");
        assert!((quiet.goal - at(-25.0)).abs() < 1e-9);
        let body = quiet.body.expect("a body");
        assert!((body.now - at(-34.0)).abs() < 1e-9);
        // Quieter: from the target's line down to the reading's.
        assert!(body.now > quiet.goal);
        assert!((body.top - quiet.goal).abs() < 1e-9);
        assert!((body.height - (body.now - quiet.goal)).abs() < 1e-9);
        // Louder: from the reading's line down to the target's.
        let loud = spring(Some(-180), IN, LAW).expect("a spring");
        let body = loud.body.expect("a body");
        assert!(body.now < loud.goal);
        assert!((body.top - body.now).abs() < 1e-9);
        assert!((body.height - (loud.goal - body.now)).abs() < 1e-9);
    }

    #[test]
    fn the_spring_is_green_only_in_the_band() {
        let state = |v: i16| spring(Some(v), IN, LAW).expect("a spring").state();
        assert_eq!(state(-240), "in");
        assert_eq!(state(-260), "in");
        assert_eq!(state(-239), "off");
        assert_eq!(state(-261), "off");
    }

    #[test]
    fn silence_leaves_the_targets_line_alone() {
        let silent = spring(Some(-700), IN, LAW).expect("a spring");
        assert_eq!(silent.body, None);
        assert_eq!(silent.state(), "silent");
        assert_eq!(
            silent.vars(),
            format!(
                "--goal:{0:.3}%;--now:{0:.3}%;--top:{0:.3}%;--h:0.000%;",
                silent.goal
            )
        );
        // Just above silence is a reading.
        assert!(
            spring(Some(-699), IN, LAW)
                .expect("a spring")
                .body
                .is_some()
        );
    }

    #[test]
    fn no_target_or_no_value_has_no_spring() {
        assert_eq!(spring(Some(-250), OUT, LAW), None);
        assert_eq!(spring(Some(-250), None, LAW), None);
        assert_eq!(spring(None, IN, LAW), None);
    }

    #[test]
    fn the_springs_positions_go_to_the_stylesheet() {
        let look = SpringLook {
            goal: 60.0,
            body: Some(SpringBody {
                now: 70.0,
                top: 60.0,
                height: 10.0,
                in_band: false,
            }),
        };
        assert_eq!(
            look.vars(),
            "--goal:60.000%;--now:70.000%;--top:60.000%;--h:10.000%;"
        );
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
