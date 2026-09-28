//! The stage (S4 design note §2): the layout canvas (the TouchOSC canvas,
//! 2360×1640) scaled to the viewport and centred, every item placed from
//! its canvas frame, and the tab bars of the pages and pagers.

use fohmixer_proto::layout::{Canvas, Frame, Orientation, Page, Pager, Style, Tab, TabBar};

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

/// The part of `area` its tab bar leaves to the pages (the area when the bar
/// is hidden).
pub fn content_frame(area: Frame, bar: &TabBar) -> Frame {
    let s = bar.bar_size.max(0.0);
    let (x, y, w, h) = (area.x, area.y, area.w, area.h);
    let (shorter_w, shorter_h) = ((w - s).max(0.0), (h - s).max(0.0));
    match bar.orientation {
        Orientation::Top => frame_of(x, y + s, w, shorter_h),
        Orientation::Bottom => frame_of(x, y, w, shorter_h),
        Orientation::Left => frame_of(x + s, y, shorter_w, h),
        Orientation::Right => frame_of(x, y, shorter_w, h),
    }
}

/// The CSS of a pager's or page's fill: its box under everything it holds
/// (the items' z start at 1).
pub fn fill_style(frame: Frame, color: &str) -> String {
    format!("{}z-index:0;background:{color};", box_style(frame))
}

/// The stage's fill (the root pager's background), or nothing.
pub fn stage_fill(background: Option<&str>) -> String {
    background.map_or_else(String::new, |c| format!("background:{c};"))
}

/// Where a nested pager sits among its page's items: the lowest z of the
/// items on its pages (the import numbers them in node order, so the items
/// before the pager are below it and the ones after above), 0 without items.
pub fn pager_z(pager: &Pager) -> i64 {
    fn lowest(page: &Page) -> Option<i64> {
        let own = page.items.iter().map(|i| i.z).min();
        let nested = page
            .pager
            .as_ref()
            .and_then(|p| p.pages.iter().filter_map(lowest).min());
        own.into_iter().chain(nested).min()
    }
    pager.pages.iter().filter_map(lowest).min().unwrap_or(0)
}

/// The classes of a tab bar: a vertical one says its side, which turns its
/// titles to run along it (reading upwards on the left, downwards on the
/// right).
pub fn tabbar_class(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Top | Orientation::Bottom => "tabbar",
        Orientation::Left => "tabbar vertical left",
        Orientation::Right => "tabbar vertical right",
    }
}

/// A tab's classes: one with its own lit colour is not brightened by the
/// stylesheet when lit (`own-lit`).
pub fn tab_class(tab: &Tab) -> &'static str {
    if tab.color_on.is_some() {
        "tab own-lit"
    } else {
        "tab"
    }
}

/// A tab's colour: its lit colour while its page is shown (its colour when
/// it has none).
pub fn tab_background(tab: &Tab, lit: bool) -> Option<&str> {
    let on = tab.color_on.as_deref().filter(|_| lit);
    on.or(tab.color.as_deref())
}

/// A tab's text size before fitting: its lit size while its page is shown
/// (its size when it has none); `None`: the stylesheet's.
pub fn tab_font(tab: &Tab, lit: bool) -> Option<f64> {
    let on = tab.text_size_on.filter(|_| lit);
    on.or(tab.text_size)
}

/// A CSS length in px (`"22px"`, a computed style), or `None` for another
/// unit or no number.
pub fn px_value(text: &str) -> Option<f64> {
    text.trim().strip_suffix("px")?.trim().parse().ok()
}

/// How much of its room a fitted text may take (rounding and hinting).
pub const FIT_MARGIN: f64 = 0.96;

/// The font size at which a text measured `text` (width, height) at `base`
/// fits `room` (width, height), both in the same px: `base` when it fits
/// with the margin, else scaled down, never up. A text or room of no size
/// keeps `base`.
pub fn fitted_font(base: f64, text: (f64, f64), room: (f64, f64)) -> f64 {
    let ratio = |t: f64, r: f64| (r * FIT_MARGIN / t).min(1.0);
    let scale = ratio(text.0, room.0).min(ratio(text.1, room.1));
    if scale > 0.0 { base * scale } else { base }
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

    #[test]
    fn the_content_is_the_area_less_its_tab_bar() {
        let area = frame(229.0, 61.0, 1746.0, 773.0);
        assert_eq!(
            content_frame(area, &bar(Orientation::Left, 65.0)),
            frame(294.0, 61.0, 1681.0, 773.0)
        );
        assert_eq!(
            content_frame(area, &bar(Orientation::Right, 65.0)),
            frame(229.0, 61.0, 1681.0, 773.0)
        );
        assert_eq!(
            content_frame(area, &bar(Orientation::Top, 59.0)),
            frame(229.0, 120.0, 1746.0, 714.0)
        );
        assert_eq!(
            content_frame(area, &bar(Orientation::Bottom, 59.0)),
            frame(229.0, 61.0, 1746.0, 714.0)
        );
        assert_eq!(content_frame(area, &bar(Orientation::Top, 0.0)), area);
        // A bar thicker than the area leaves nothing, never a negative size.
        let small = frame(0.0, 0.0, 40.0, 30.0);
        assert_eq!(
            content_frame(small, &bar(Orientation::Left, 50.0)),
            frame(50.0, 0.0, 0.0, 30.0)
        );
        assert_eq!(
            content_frame(small, &bar(Orientation::Bottom, 50.0)),
            frame(0.0, 0.0, 40.0, 0.0)
        );
        assert_eq!(
            content_frame(small, &bar(Orientation::Top, -5.0)),
            small,
            "a negative bar is no bar"
        );
    }

    #[test]
    fn fills_are_boxes_under_the_items() {
        assert_eq!(
            fill_style(frame(1.0, 2.0, 3.0, 4.0), "#000000F9"),
            "left:1px;top:2px;width:3px;height:4px;z-index:0;background:#000000F9;"
        );
        assert_eq!(stage_fill(Some("#9D9DA0FF")), "background:#9D9DA0FF;");
        assert_eq!(stage_fill(None), "");
    }

    fn page_with(id: &str, zs: &[i64], pager: Option<Pager>) -> Page {
        let items = zs
            .iter()
            .map(|z| {
                serde_json::from_value(serde_json::json!({
                    "kind": "label", "frame": {"x": 0, "y": 0, "w": 1, "h": 1}, "z": z, "text": "x"
                }))
                .unwrap()
            })
            .collect();
        Page {
            id: id.into(),
            title: id.into(),
            tab: Tab::default(),
            background: None,
            items,
            pager,
        }
    }

    fn pager_of(pages: Vec<Page>) -> Pager {
        Pager {
            frame: frame(0.0, 0.0, 10.0, 10.0),
            background: None,
            tabbar: bar(Orientation::Left, 5.0),
            pages,
        }
    }

    #[test]
    fn a_pager_sits_at_its_lowest_item() {
        let inner = pager_of(vec![page_with("deep", &[7, 3], None)]);
        let pager = pager_of(vec![
            page_with("a", &[12, 9], None),
            page_with("b", &[11], Some(inner)),
            page_with("empty", &[], None),
        ]);
        assert_eq!(pager_z(&pager), 3);
        assert_eq!(pager_z(&pager_of(vec![page_with("a", &[12, 9], None)])), 9);
        assert_eq!(pager_z(&pager_of(vec![page_with("a", &[], None)])), 0);
        assert_eq!(pager_z(&pager_of(Vec::new())), 0);
    }

    #[test]
    fn a_vertical_bar_says_its_side() {
        assert_eq!(tabbar_class(Orientation::Top), "tabbar");
        assert_eq!(tabbar_class(Orientation::Bottom), "tabbar");
        assert_eq!(tabbar_class(Orientation::Left), "tabbar vertical left");
        assert_eq!(tabbar_class(Orientation::Right), "tabbar vertical right");
    }

    #[test]
    fn a_lit_tab_takes_its_lit_colour_and_size() {
        let tab = Tab {
            color: Some("#404040FF".into()),
            color_on: Some("#BAFFA657".into()),
            text_size: Some(36.0),
            text_size_on: Some(51.0),
        };
        assert_eq!(tab_class(&tab), "tab own-lit");
        assert_eq!(tab_class(&Tab::default()), "tab");
        assert_eq!(tab_background(&tab, true), Some("#BAFFA657"));
        assert_eq!(tab_background(&tab, false), Some("#404040FF"));
        assert_eq!(tab_font(&tab, true), Some(51.0));
        assert_eq!(tab_font(&tab, false), Some(36.0));
        // Without lit values a lit tab keeps its own; without any, no colour
        // and the stylesheet's size (None).
        let plain = Tab {
            color: Some("#404040FF".into()),
            color_on: None,
            text_size: Some(33.0),
            text_size_on: None,
        };
        assert_eq!(tab_background(&plain, true), Some("#404040FF"));
        assert_eq!(tab_font(&plain, true), Some(33.0));
        assert_eq!(tab_background(&Tab::default(), true), None);
        assert_eq!(tab_font(&Tab::default(), false), None);
        let only_on = Tab {
            color: None,
            color_on: Some("#FA00004A".into()),
            text_size: None,
            text_size_on: Some(20.0),
        };
        assert_eq!(tab_background(&only_on, false), None);
        assert_eq!(tab_font(&only_on, false), None);
        assert_eq!(tab_font(&only_on, true), Some(20.0));
    }

    fn close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} is not {want}");
    }

    #[test]
    fn a_css_length_in_px_is_read() {
        assert_eq!(px_value("22px"), Some(22.0));
        assert_eq!(px_value(" 15.5px "), Some(15.5));
        assert_eq!(px_value("1.2em"), None);
        assert_eq!(px_value("px"), None);
        assert_eq!(px_value(""), None);
    }

    #[test]
    fn a_text_is_scaled_down_to_fit_never_up() {
        // Fits with the margin: the base size.
        assert_eq!(fitted_font(22.0, (96.0, 20.0), (100.0, 30.0)), 22.0);
        // The margin itself: 96 of 100 fits, 96.5 does not.
        assert_eq!(fitted_font(20.0, (96.0, 10.0), (100.0, 30.0)), 20.0);
        assert!(fitted_font(20.0, (96.5, 10.0), (100.0, 30.0)) < 20.0);
        // Too wide: scaled so the text takes 96 % of the width.
        close(fitted_font(15.0, (48.0, 18.0), (40.0, 25.0)), 12.0);
        // Too tall: the height decides.
        close(fitted_font(52.0, (300.0, 62.5), (600.0, 50.0)), 39.936);
        // Both too large: the smaller scale wins.
        close(fitted_font(10.0, (200.0, 50.0), (100.0, 50.0)), 4.8);
        // Nothing measured (not laid out yet): the base size.
        assert_eq!(fitted_font(24.0, (0.0, 0.0), (104.0, 42.0)), 24.0);
        assert_eq!(fitted_font(24.0, (0.0, 0.0), (0.0, 0.0)), 24.0);
        // A room of no size never gives a zero font.
        assert_eq!(fitted_font(24.0, (50.0, 20.0), (0.0, 0.0)), 24.0);
    }
}
