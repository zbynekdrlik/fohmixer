//! The stage (S4 design note §2): the layout canvas (the TouchOSC canvas,
//! 2360×1640) scaled to the viewport and centred, every item placed from
//! its canvas frame, and the tab bars of the pages and pagers.

use fohmixer_proto::layout::{Canvas, Frame, Orientation, Style, TabBar};

/// How the canvas sits in the viewport: its scale and its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

/// The canvas fitted into a `vw`×`vh` viewport: as large as fits, centred.
pub fn fit(vw: f64, vh: f64, canvas: Canvas) -> Fit {
    let scale = (vw / canvas.w).min(vh / canvas.h);
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    Fit {
        scale,
        x: (vw - canvas.w * scale) / 2.0,
        y: (vh - canvas.h * scale) / 2.0,
    }
}

/// The stage's CSS transform for a fit (origin at the top-left corner).
pub fn transform(fit: Fit) -> String {
    format!("translate({}px, {}px) scale({})", fit.x, fit.y, fit.scale)
}

/// A frame (every field spelled out: no struct-update base, see
/// `.claude/rules/hub-rust.md`).
fn frame_of(x: f64, y: f64, w: f64, h: f64) -> Frame {
    Frame { x, y, w, h }
}

/// `frame` relative to `origin`'s top-left corner.
pub fn relative(frame: Frame, origin: Frame) -> Frame {
    frame_of(frame.x - origin.x, frame.y - origin.y, frame.w, frame.h)
}

/// The CSS box of a frame (absolutely positioned in its parent).
pub fn box_style(frame: Frame) -> String {
    format!(
        "left:{}px;top:{}px;width:{}px;height:{}px;",
        frame.x, frame.y, frame.w, frame.h
    )
}

/// The CSS of a placed item: its box, its z-order (node order) and its
/// layout style (background, text colour and size).
pub fn item_style(frame: Frame, z: i64, style: &Style) -> String {
    let mut css = format!("{}z-index:{z};", box_style(frame));
    if let Some(bg) = &style.bg {
        css.push_str(&format!("background:{bg};"));
    }
    if let Some(color) = style.text_color.as_ref().or(style.color.as_ref()) {
        css.push_str(&format!("color:{color};"));
    }
    if let Some(size) = style.text_size {
        css.push_str(&format!("font-size:{size}px;"));
    }
    css
}

/// A tab bar's place and its tabs, in the coordinates of its area.
#[derive(Debug, Clone, PartialEq)]
pub struct TabLayout {
    pub bar: Frame,
    pub tabs: Vec<Frame>,
    /// Whether the tabs run top to bottom.
    pub vertical: bool,
}

/// The tab bar of `count` pages inside `area` (the canvas for the root
/// pages, the pager's frame for a pager): on the bar's side, `bar_size`
/// thick, the tabs sharing its length equally except `reserve` at its end
/// (the root bar keeps room for the connection badges and the version).
/// `None` when the bar is hidden (`bar_size` 0).
pub fn tab_layout(area: Frame, bar: &TabBar, count: usize, reserve: f64) -> Option<TabLayout> {
    let size = bar.bar_size;
    if size <= 0.0 {
        return None;
    }
    let (x, y, w, h) = (area.x, area.y, area.w, area.h);
    let (frame, vertical) = match bar.orientation {
        Orientation::Top => (frame_of(x, y, w, size), false),
        Orientation::Bottom => (frame_of(x, y + h - size, w, size), false),
        Orientation::Left => (frame_of(x, y, size, h), true),
        Orientation::Right => (frame_of(x + w - size, y, size, h), true),
    };
    let length = if vertical { frame.h } else { frame.w };
    let step = (length - reserve).max(0.0) / count.max(1) as f64;
    let tabs = (0..count)
        .map(|i| {
            let at = step * i as f64;
            if vertical {
                frame_of(frame.x, frame.y + at, frame.w, step)
            } else {
                frame_of(frame.x + at, frame.y, step, frame.h)
            }
        })
        .collect();
    Some(TabLayout {
        bar: frame,
        tabs,
        vertical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Canvas = Canvas {
        w: 2360.0,
        h: 1640.0,
    };

    fn frame(x: f64, y: f64, w: f64, h: f64) -> Frame {
        Frame { x, y, w, h }
    }

    #[test]
    fn the_canvas_fits_the_viewport_and_is_centred() {
        // 16:9 (a desktop browser): the height limits, bars left and right.
        let f = fit(1280.0, 720.0, CANVAS);
        assert_eq!(f.scale, 720.0 / 1640.0);
        assert!((f.x - (1280.0 - 2360.0 * f.scale) / 2.0).abs() < 1e-9 && f.y.abs() < 1e-9);
        assert!(f.x > 100.0);
        // 4:3: the width limits, bars top and bottom.
        let f = fit(1024.0, 768.0, CANVAS);
        assert_eq!(f.scale, 1024.0 / 2360.0);
        assert!(f.x.abs() < 1e-9);
        assert!((f.y - (768.0 - 1640.0 * f.scale) / 2.0).abs() < 1e-9 && f.y > 10.0);
        // The iPad Pro 11 in landscape (1194×834): almost exactly its shape.
        let f = fit(1194.0, 834.0, CANVAS);
        assert_eq!(f.scale, 1194.0 / 2360.0);
        assert!(f.y > 0.0 && f.y < 3.0, "{}", f.y);
        // The canvas's own size is scale 1.
        assert_eq!(
            fit(2360.0, 1640.0, CANVAS),
            Fit {
                scale: 1.0,
                x: 0.0,
                y: 0.0
            }
        );
    }

    #[test]
    fn a_degenerate_viewport_keeps_scale_one() {
        assert_eq!(fit(0.0, 800.0, CANVAS).scale, 1.0);
        let empty = Canvas { w: 0.0, h: 0.0 };
        assert_eq!(fit(800.0, 600.0, empty).scale, 1.0);
    }

    #[test]
    fn the_transform_and_the_boxes_are_css() {
        let f = Fit {
            scale: 0.5,
            x: 10.0,
            y: 2.5,
        };
        assert_eq!(transform(f), "translate(10px, 2.5px) scale(0.5)");
        assert_eq!(
            box_style(frame(1.0, 2.5, 30.0, 40.0)),
            "left:1px;top:2.5px;width:30px;height:40px;"
        );
        assert_eq!(
            relative(
                frame(2020.0, 188.0, 131.0, 553.0),
                frame(2005.0, 105.0, 161.0, 700.0)
            ),
            frame(15.0, 83.0, 131.0, 553.0)
        );
    }

    fn bar(orientation: Orientation, bar_size: f64) -> TabBar {
        TabBar {
            orientation,
            bar_size,
            default_page: 0,
        }
    }

    #[test]
    fn an_item_carries_its_box_z_and_style() {
        let f = frame(10.0, 20.0, 30.0, 40.0);
        assert_eq!(
            item_style(f, 3, &Style::default()),
            "left:10px;top:20px;width:30px;height:40px;z-index:3;"
        );
        let style = Style {
            bg: Some("#FF00001F".into()),
            color: Some("#00FF00".into()),
            text: None,
            text_color: None,
            text_size: Some(33.0),
            vertical: false,
        };
        assert_eq!(
            item_style(f, -1, &style),
            "left:10px;top:20px;width:30px;height:40px;z-index:-1;background:#FF00001F;color:#00FF00;font-size:33px;"
        );
        let text = Style {
            text_color: Some("#FFFFFF".into()),
            ..style
        };
        assert!(
            item_style(f, 0, &text).contains("color:#FFFFFF;"),
            "the text colour wins"
        );
    }

    #[test]
    fn a_top_bar_shares_its_width_minus_the_reserve() {
        let area = frame(0.0, 0.0, 2360.0, 1640.0);
        let t = tab_layout(area, &bar(Orientation::Top, 59.0), 3, 380.0).unwrap();
        assert_eq!(t.bar, frame(0.0, 0.0, 2360.0, 59.0));
        assert!(!t.vertical);
        assert_eq!(
            t.tabs,
            [
                frame(0.0, 0.0, 660.0, 59.0),
                frame(660.0, 0.0, 660.0, 59.0),
                frame(1320.0, 0.0, 660.0, 59.0)
            ]
        );
    }

    #[test]
    fn a_left_bar_stacks_its_tabs() {
        let area = frame(229.0, 61.0, 1746.0, 773.0);
        let t = tab_layout(area, &bar(Orientation::Left, 65.0), 2, 0.0).unwrap();
        assert_eq!(t.bar, frame(229.0, 61.0, 65.0, 773.0));
        assert!(t.vertical);
        assert_eq!(
            t.tabs,
            [
                frame(229.0, 61.0, 65.0, 386.5),
                frame(229.0, 447.5, 65.0, 386.5)
            ]
        );
    }

    #[test]
    fn bottom_and_right_bars_sit_at_the_far_side() {
        let area = frame(100.0, 200.0, 400.0, 300.0);
        let t = tab_layout(area, &bar(Orientation::Bottom, 50.0), 2, 100.0).unwrap();
        assert_eq!(t.bar, frame(100.0, 450.0, 400.0, 50.0));
        assert_eq!(t.tabs[1], frame(250.0, 450.0, 150.0, 50.0));
        let t = tab_layout(area, &bar(Orientation::Right, 40.0), 3, 0.0).unwrap();
        assert_eq!(t.bar, frame(460.0, 200.0, 40.0, 300.0));
        assert_eq!(t.tabs[2], frame(460.0, 400.0, 40.0, 100.0));
    }

    #[test]
    fn a_hidden_bar_has_no_tabs_and_no_pages_no_tab() {
        let area = frame(0.0, 0.0, 100.0, 100.0);
        assert_eq!(tab_layout(area, &bar(Orientation::Top, 0.0), 3, 0.0), None);
        let t = tab_layout(area, &bar(Orientation::Top, 10.0), 0, 0.0).unwrap();
        assert!(t.tabs.is_empty());
        // A reserve longer than the bar leaves empty tabs, never negative.
        let t = tab_layout(area, &bar(Orientation::Top, 10.0), 2, 500.0).unwrap();
        assert_eq!(t.tabs[1], frame(0.0, 0.0, 0.0, 10.0));
    }
}
