//! Where the controls go (the redesign, #21; spec §4.2): the page is laid
//! out by the stylesheet (rows top to bottom by weight, sections left to
//! right), and this module decides the one number the stylesheet cannot:
//! the strip width every row shares, so the strips of all rows line up. A
//! row that does not fit at the narrowest strip scrolls inside itself; the
//! page never scrolls.

use fohmixer_proto::layout::{Control, Group, Pager, Row, Section};

/// The stylesheet's measures the width depends on (px): keep them equal to
/// `style.css` (`.row` gap, `.group` padding + border, `.group-body` gap).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Between two sections of a row.
    pub section_gap: f64,
    /// A group's padding and border, both sides.
    pub section_pad: f64,
    /// Between two strips of a group.
    pub strip_gap: f64,
    /// The least a group of buttons (no strips) takes.
    pub free_min: f64,
    /// The strip width's bounds.
    pub min: f64,
    pub max: f64,
    /// A wide strip's width, in strips.
    pub wide: f64,
}

/// The surface's measures.
pub const METRICS: Metrics = Metrics {
    section_gap: 10.0,
    section_pad: 10.0,
    strip_gap: 4.0,
    free_min: 180.0,
    min: 64.0,
    max: 120.0,
    wide: 1.1,
};

/// The width a row (or a group) needs: `units` strips and `fixed` px.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Shape {
    pub units: f64,
    pub fixed: f64,
}

impl Shape {
    fn add(self, other: Shape) -> Shape {
        Shape {
            units: self.units + other.units,
            fixed: self.fixed + other.fixed,
        }
    }

    /// The px the shape takes with strips `width` wide.
    pub fn width(self, width: f64) -> f64 {
        self.units * width + self.fixed
    }
}

/// Whether a control is drawn as a strip-wide column (a strip, a parameter
/// fader); the other controls are buttons.
pub fn is_column(control: &Control) -> bool {
    matches!(control, Control::Strip(_) | Control::ParamFader { .. })
}

/// How many strips wide a control is.
fn units(control: &Control, m: &Metrics) -> f64 {
    match control {
        Control::Strip(strip) if strip.wide => m.wide,
        _ if is_column(control) => 1.0,
        _ => 0.0,
    }
}

/// A group's shape: its columns, the gaps between them and its padding, and
/// the least its buttons take when it has any.
pub fn group_shape(group: &Group, m: &Metrics) -> Shape {
    let columns = group.controls.iter().filter(|c| is_column(c)).count();
    let buttons = group.controls.len() - columns;
    let gaps = columns.saturating_sub(1) as f64 * m.strip_gap;
    let free = if buttons > 0 { m.free_min } else { 0.0 };
    Shape {
        units: group.controls.iter().map(|c| units(c, m)).sum(),
        fixed: m.section_pad + gaps + free,
    }
}

/// Groups side by side: their shapes and the gaps between them.
fn side_by_side<'a>(groups: impl Iterator<Item = &'a Group>, m: &Metrics) -> (Shape, usize) {
    groups.fold((Shape::default(), 0), |(shape, n), g| {
        (shape.add(group_shape(g, m)), n + 1)
    })
}

/// The wider of two shapes (more strips; then more fixed px).
fn wider(a: (Shape, usize), b: (Shape, usize)) -> (Shape, usize) {
    let by_units = a.0.units.total_cmp(&b.0.units);
    let by_fixed = a.0.fixed.total_cmp(&b.0.fixed);
    if by_units.then(by_fixed).is_lt() {
        b
    } else {
        a
    }
}

/// A pager's widest sub-page: its groups and their count.
fn widest(pager: &Pager, m: &Metrics) -> (Shape, usize) {
    pager
        .pages
        .iter()
        .map(|sub| side_by_side(sub.sections.iter().flat_map(Section::groups), m))
        .fold((Shape::default(), 0), wider)
}

/// The width a pager takes: its widest sub-page, groups and the gaps between
/// them. The pager is that wide whichever sub-page it shows, so switching it
/// never moves the row's other groups (a tap aimed at a bus strip stays on
/// it).
pub fn pager_shape(pager: &Pager, m: &Metrics) -> Shape {
    let (shape, count) = widest(pager, m);
    Shape {
        units: shape.units,
        fixed: shape.fixed + count.saturating_sub(1) as f64 * m.section_gap,
    }
}

/// A row's shape. A pager counts as its widest sub-page (see [`pager_shape`]).
pub fn row_shape(row: &Row, m: &Metrics) -> Shape {
    let (shape, count) =
        row.sections
            .iter()
            .fold((Shape::default(), 0usize), |(shape, count), section| {
                let (s, n) = match section {
                    Section::Group(group) => (group_shape(group, m), 1),
                    Section::Pager(pager) => widest(pager, m),
                };
                (shape.add(s), count + n)
            });
    Shape {
        units: shape.units,
        fixed: shape.fixed + count.saturating_sub(1) as f64 * m.section_gap,
    }
}

/// The strip width every row shares: the widest at which each row with
/// strips fits `avail` px, within the bounds.
pub fn strip_width(rows: &[Shape], avail: f64, m: &Metrics) -> f64 {
    rows.iter()
        .filter(|r| r.units > 0.0)
        .map(|r| (avail - r.fixed) / r.units)
        .fold(m.max, f64::min)
        .clamp(m.min, m.max)
}

/// How much a row may pass its room before it scrolls (rounding).
pub const SLACK: f64 = 0.5;

/// Whether a row with strips `width` wide passes `avail` (it then scrolls
/// inside itself).
pub fn overflows(row: Shape, width: f64, avail: f64) -> bool {
    row.width(width) > avail + SLACK
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::layout::{Binding, Press, Strip, StripKind, SubPage};

    fn strip(wide: bool) -> Control {
        Control::Strip(Box::new(Strip {
            binding: Binding {
                instance: "band".into(),
                anchor: fohmixer_proto::layout::Anchor::Master,
                path: None,
            },
            strip_kind: StripKind::Standard,
            wide,
            mute_guard: false,
        }))
    }

    fn toggle() -> Control {
        Control::ParamToggle {
            label: "Vox 1 TU".into(),
            targets: vec![],
            press: Press::Toggle,
            color: None,
        }
    }

    fn fader() -> Control {
        Control::ParamFader {
            label: "All".into(),
            targets: vec![],
        }
    }

    fn group(controls: Vec<Control>) -> Group {
        Group {
            id: None,
            title: None,
            color: None,
            controls,
        }
    }

    fn row(sections: Vec<Section>) -> Row {
        Row {
            sections,
            weight: 1.0,
        }
    }

    fn pager(pages: Vec<Vec<Group>>) -> Section {
        Section::Pager(Pager {
            id: "p".into(),
            default_page: "s0".into(),
            pages: pages
                .into_iter()
                .enumerate()
                .map(|(i, groups)| SubPage {
                    id: format!("s{i}"),
                    title: format!("S{i}"),
                    sections: groups.into_iter().map(Section::Group).collect(),
                })
                .collect(),
        })
    }

    #[test]
    fn strips_and_parameter_faders_are_columns() {
        assert!(is_column(&strip(false)));
        assert!(is_column(&fader()));
        assert!(!is_column(&toggle()));
        assert!(!is_column(&Control::Refresh { label: None }));
    }

    #[test]
    fn a_group_is_its_columns_gaps_padding_and_its_buttons_room() {
        let m = METRICS;
        // Three strips, one wide: 3.1 strips, two 4 px gaps and 10 px of padding.
        let g = group(vec![strip(false), strip(true), fader()]);
        assert_eq!(
            group_shape(&g, &m),
            Shape {
                units: 3.1,
                fixed: 18.0
            }
        );
        // One strip: no gap.
        assert_eq!(
            group_shape(&group(vec![strip(false)]), &m),
            Shape {
                units: 1.0,
                fixed: 10.0
            }
        );
        // Buttons only: their least width and the padding.
        assert_eq!(
            group_shape(&group(vec![toggle(), toggle()]), &m),
            Shape {
                units: 0.0,
                fixed: 190.0
            }
        );
        // An empty group is its padding.
        assert_eq!(
            group_shape(&group(vec![]), &m),
            Shape {
                units: 0.0,
                fixed: 10.0
            }
        );
    }

    #[test]
    fn a_row_adds_its_sections_and_the_gaps_between_them() {
        let m = METRICS;
        let r = row(vec![
            Section::Group(group(vec![strip(false), strip(false)])),
            Section::Group(group(vec![strip(true)])),
        ]);
        // 2 + 1.1 strips; 10 + 4 and 10 px of groups; one 10 px gap.
        assert_eq!(
            row_shape(&r, &m),
            Shape {
                units: 3.1,
                fixed: 34.0
            }
        );
        assert_eq!(row_shape(&row(vec![]), &m), Shape::default());
    }

    #[test]
    fn a_pager_counts_as_its_widest_sub_page() {
        let m = METRICS;
        let narrow = vec![group(vec![strip(false)])];
        let wide = vec![
            group(vec![strip(false), strip(false)]),
            group(vec![strip(false)]),
        ];
        let r = row(vec![
            pager(vec![narrow.clone(), wide.clone()]),
            Section::Group(group(vec![strip(false)])),
        ]);
        // The wide sub-page (3 strips, two groups: 14 + 10 px) and the fixed
        // group (10 px): three sections, two gaps.
        assert_eq!(
            row_shape(&r, &m),
            Shape {
                units: 4.0,
                fixed: 54.0
            }
        );
        // The order of the sub-pages does not matter.
        let r2 = row(vec![
            pager(vec![wide, narrow]),
            Section::Group(group(vec![strip(false)])),
        ]);
        assert_eq!(row_shape(&r2, &m), row_shape(&r, &m));
        // Equal strips: the one with more fixed px (buttons) is wider.
        let a = vec![group(vec![strip(false)])];
        let b = vec![group(vec![strip(false), toggle()])];
        let r3 = row(vec![pager(vec![a.clone(), b.clone()])]);
        let r4 = row(vec![pager(vec![b, a])]);
        let want = Shape {
            units: 1.0,
            fixed: 190.0,
        };
        assert_eq!(row_shape(&r3, &m), want);
        assert_eq!(row_shape(&r4, &m), want);
        // An empty pager takes nothing.
        assert_eq!(row_shape(&row(vec![pager(vec![])]), &m), Shape::default());
    }

    #[test]
    fn a_pager_is_as_wide_as_its_widest_sub_page_whichever_it_shows() {
        let m = METRICS;
        let narrow = vec![group(vec![strip(false)])];
        let wide = vec![
            group(vec![strip(false), strip(false)]),
            group(vec![strip(true)]),
        ];
        let Section::Pager(p) = pager(vec![narrow, wide]) else {
            panic!("a pager")
        };
        // 3.1 strips; groups of 14 and 10 px and one 10 px gap between them.
        let shape = pager_shape(&p, &m);
        assert_eq!(shape.units, 3.1);
        assert_eq!(shape.fixed, 34.0);
        let Section::Pager(one) = pager(vec![vec![group(vec![strip(false)])]]) else {
            panic!("a pager")
        };
        assert_eq!(
            pager_shape(&one, &m),
            Shape {
                units: 1.0,
                fixed: 10.0
            }
        );
        let Section::Pager(empty) = pager(vec![]) else {
            panic!("a pager")
        };
        assert_eq!(pager_shape(&empty, &m), Shape::default());
    }

    #[test]
    fn the_strip_width_is_the_widest_that_every_row_fits() {
        let m = METRICS;
        let rows = [
            Shape {
                units: 10.0,
                fixed: 100.0,
            },
            Shape {
                units: 5.0,
                fixed: 50.0,
            },
        ];
        // 1000 px: the first row fits 90 px strips, the second 190 (above max).
        assert_eq!(strip_width(&rows, 1000.0, &m), 90.0);
        // A row without strips (buttons only) does not limit the width.
        let buttons = Shape {
            units: 0.0,
            fixed: 900.0,
        };
        assert_eq!(strip_width(&[rows[1], buttons], 1000.0, &m), 120.0);
        // Not even when its buttons need more than the room.
        let crowded = Shape {
            units: 0.0,
            fixed: 1100.0,
        };
        assert_eq!(strip_width(&[rows[1], crowded], 1000.0, &m), 120.0);
        assert_eq!(strip_width(&[], 1000.0, &m), 120.0);
        // Too narrow: the least width, and the row overflows.
        assert_eq!(strip_width(&rows, 500.0, &m), 64.0);
        // Exactly at the bounds.
        let one = Shape {
            units: 1.0,
            fixed: 0.0,
        };
        assert_eq!(strip_width(&[one], 64.0, &m), 64.0);
        assert_eq!(strip_width(&[one], 63.0, &m), 64.0);
        assert_eq!(strip_width(&[one], 120.0, &m), 120.0);
        assert_eq!(strip_width(&[one], 119.0, &m), 119.0);
        assert_eq!(strip_width(&[one], 65.0, &m), 65.0);
    }

    #[test]
    fn a_row_overflows_only_past_its_room_and_the_slack() {
        let r = Shape {
            units: 2.0,
            fixed: 10.0,
        };
        assert_eq!(r.width(50.0), 110.0);
        assert!(!overflows(r, 50.0, 110.0));
        assert!(!overflows(r, 50.0, 109.5));
        assert!(overflows(r, 50.0, 109.49));
        assert!(overflows(r, 50.0, 100.0));
    }
}
