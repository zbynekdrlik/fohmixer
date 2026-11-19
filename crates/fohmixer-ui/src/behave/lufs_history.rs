//! A fader's LUFS history in its channel detail (#219, the design on the
//! ticket; the owner's approved mockup `docs/mockups/lufs-history-v2.html`,
//! the graph's look since #253 `docs/mockups/lufs-history-v3.html`).
//! Every decision the detail's LUFS tab makes, pure and tested:
//!
//! - which middle-box tabs a strip has and which one shows (`tabs`,
//!   `shown_tab`): `EQ` always, `LUFS` only on a `+LUFS` strip; a strip
//!   with one tab shows no tab row, as before;
//! - the graph's window (`View`): `10 MIN` / `1 H` / `3 H` ending now, a
//!   window dragged back in time (`View::panned`), a past day's whole span
//!   (`View::pick_day`), and `NOW ›` back to the live edge;
//! - a finger's press on the graph (`Press`): a drag after 6 px, a hold
//!   (the cursor) after 350 ms without one;
//! - the values in screen columns (#253, the owner's approved mockup v3:
//!   `columns`, 120 of them, each the gated energy mean of its readings
//!   above silence (`gated`), silence when half or more are silent, then
//!   `glide`d 1-2-1 over its neighbours, never across a gap), the scale
//!   (`scale`, a tick every 3 LU), the line (`lines`: one monotone cubic
//!   `curve` per run, cut at silence and no reading; green is the same path
//!   clipped to ±1.0 LU of an input point's target) and its live end
//!   (`live_end`), the strip's fader on its own dB scale (`fader_zero`,
//!   `fader_y`, `fader_label`, `fader_lines`, `fader_text`), the time ticks
//!   (`time_step`, `time_ticks`) and every text (`hh_mm`, `day_label`,
//!   `cursor_text`);
//! - the API's URLs and answers (`history_url`, `days_url`,
//!   `series_answer`, `days_answer`).
//!
//! The graph draws in a `0 0 1000 1000` box stretched over the plot
//! (`preserveAspectRatio="none"`, the lines' width not scaled), so nothing
//! here needs the page's pixels; the labels sit beside it in percent.

use fohmixer_proto::lufs::{HistoryDay, HistoryDays, HistorySeries, text};

use super::Down;
use super::eq::encode_component;
use crate::diag::perf::Pointer;
use crate::net::api_message;

/// A second, a minute, an hour and a day in ms.
pub const SECOND_MS: i64 = 1000;
pub const MINUTE_MS: i64 = 60 * SECOND_MS;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// The graph's box: `0 0 BOX BOX`.
pub const BOX: f64 = 1000.0;

/// A tab of the detail's middle box (mockup v2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The channel's Pro-Q 4 cards, as before #219.
    Eq,
    /// The LUFS history (a `+LUFS` strip only).
    Lufs,
}

impl Tab {
    /// Its `data-tab` word.
    pub fn name(self) -> &'static str {
        match self {
            Tab::Eq => "eq",
            Tab::Lufs => "lufs",
        }
    }

    /// Its label on the tab.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Eq => "EQ",
            Tab::Lufs => "LUFS",
        }
    }
}

/// The tabs a strip has, in the row's order: `EQ`, then `LUFS` on a
/// `+LUFS` strip.
pub fn tabs(has_lufs: bool) -> Vec<Tab> {
    if has_lufs {
        vec![Tab::Eq, Tab::Lufs]
    } else {
        vec![Tab::Eq]
    }
}

/// Whether the tab row shows: only with more than one tab (a strip without
/// `+LUFS` keeps its middle box as before).
pub fn tab_row(has_lufs: bool) -> bool {
    tabs(has_lufs).len() > 1
}

/// The tab that shows: the page's kept tab when the strip has it, else
/// `EQ`.
pub fn shown_tab(kept: Tab, has_lufs: bool) -> Tab {
    if tabs(has_lufs).contains(&kept) {
        kept
    } else {
        Tab::Eq
    }
}

/// The windows' chips, label and span: `10 MIN` (the default), `1 H`,
/// `3 H`.
pub const WINDOWS: [(&str, i64); 3] = [
    ("10 MIN", 10 * MINUTE_MS),
    ("1 H", HOUR_MS),
    ("3 H", 3 * HOUR_MS),
];

/// The default window: 10 minutes.
pub const DEFAULT_SPAN_MS: i64 = WINDOWS[0].1;

/// The longest window a drag keeps (a past day dragged becomes at most
/// this).
pub const MAX_SPAN_MS: i64 = WINDOWS[2].1;

/// The shortest span a past day shows (a day of one reading is still a
/// window).
pub const MIN_DAY_SPAN_MS: i64 = MINUTE_MS;

/// Within this many ms of now a window's end is the live edge.
pub const LIVE_EDGE_MS: i64 = SECOND_MS;

/// What the graph shows: a window of `span` ending now (`end` none) or at
/// `end`, or a past day's whole span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// The window's length (the chip that is on).
    pub span: i64,
    /// Where a window dragged back ends (UTC ms); none at the live edge.
    pub end: Option<i64>,
    /// The past day shown whole.
    pub day: Option<HistoryDay>,
}

impl View {
    /// A window of `span` at the live edge.
    pub fn live(span: i64) -> Self {
        View {
            span,
            end: None,
            day: None,
        }
    }

    /// Whether it follows now (no `NOW ›`, the live edge drawn each
    /// second).
    pub fn is_live(&self) -> bool {
        self.end.is_none() && self.day.is_none()
    }

    /// Its start and end (UTC ms) at `now`.
    pub fn window(&self, now: i64) -> (i64, i64) {
        if let Some(day) = self.day {
            let end = day.last_ms.max(day.first_ms + MIN_DAY_SPAN_MS);
            return (day.first_ms, end);
        }
        let end = self.end.unwrap_or(now);
        (end - self.span, end)
    }

    /// A window chip: its span; from a past day, the window starting at
    /// the day's first reading.
    pub fn pick_span(self, span: i64, now: i64) -> Self {
        let end = match self.day {
            Some(_) => settle(self.window(now).0 + span, now),
            None => self.end,
        };
        View {
            span,
            end,
            day: None,
        }
    }

    /// `NOW ›`: back to the live edge, the window kept.
    pub fn to_now(self) -> Self {
        View::live(self.span)
    }

    /// A day chip: a past day whole; today is the live edge.
    pub fn pick_day(self, day: HistoryDay, today: bool) -> Self {
        if today {
            return self.to_now();
        }
        View {
            span: self.span,
            end: None,
            day: Some(day),
        }
    }

    /// A drag of `dx` px (right is back in time) over a plot `width` px
    /// wide, from the view `start` the press began on; never past now.
    pub fn panned(start: View, dx: f64, width: f64, now: i64) -> Self {
        let (t0, t1) = start.window(now);
        let span = match start.day {
            Some(_) => (t1 - t0).min(MAX_SPAN_MS),
            None => start.span,
        };
        let per_px = (t1 - t0) as f64 / width.max(1.0);
        let end = (t1 as f64 - dx * per_px).round() as i64;
        View {
            span,
            end: settle(end.min(now), now),
            day: None,
        }
    }

    /// Whether a window chip of `span` is on (never while a day shows).
    pub fn chip_on(&self, span: i64) -> bool {
        self.day.is_none() && self.span == span
    }

    /// Whether `day`'s chip is on.
    pub fn day_on(&self, day: HistoryDay) -> bool {
        self.day == Some(day)
    }

    /// Its word (`data-state`): `live`, `back` (a window dragged back) or
    /// `day` (a past day whole).
    pub fn state(&self) -> &'static str {
        match (self.day, self.end) {
            (Some(_), _) => "day",
            (None, None) => "live",
            (None, Some(_)) => "back",
        }
    }
}

/// A window's end: none (the live edge) within [`LIVE_EDGE_MS`] of now.
pub fn settle(end: i64, now: i64) -> Option<i64> {
    if now - end <= LIVE_EDGE_MS {
        None
    } else {
        Some(end)
    }
}

/// A finger held this long without a drag shows the cursor.
pub const HOLD_MS: f64 = 350.0;

/// A finger that moves this far is a drag.
pub const DRAG_PX: f64 = 6.0;

/// What a finger's move does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Nothing yet (under [`DRAG_PX`]).
    Still,
    /// A drag: `dx` px from the down.
    Pan(f64),
    /// The cursor follows the finger to `x`.
    Cursor(f64),
}

/// One finger on the graph: from its down at `x0` (px) and `at` (the page
/// clock, ms), by its pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Press {
    x0: f64,
    at: f64,
    pointer: i32,
    kind: Option<Pointer>,
    moved: bool,
    holding: bool,
}

/// What a pointer's down on the graph does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDown {
    /// A new press.
    Start,
    /// The held press's end never came (#124: iOS took the finger): it
    /// ends first, then the new press starts.
    Restart,
    /// A second finger beside the held one: nothing.
    Ignore,
}

/// A down on the graph while `held` (none: no press) is down: a primary
/// pointer of the held one's type, or the held pointer again, ends it
/// (`Down::ends`, as every control that holds a pointer since #124); a
/// second finger is ignored.
pub fn on_down(held: Option<&Press>, down: Down) -> OnDown {
    match held {
        None => OnDown::Start,
        Some(p) if down.id == p.pointer || down.ends(p.kind) => OnDown::Restart,
        Some(_) => OnDown::Ignore,
    }
}

impl Press {
    /// A down at `x` px, `at` ms, by `down`'s pointer.
    pub fn down(x: f64, at: f64, down: Down) -> Self {
        Press {
            x0: x,
            at,
            pointer: down.id,
            kind: down.kind,
            moved: false,
            holding: false,
        }
    }

    /// Whether an event of pointer `id` is this press's.
    pub fn is(&self, id: i32) -> bool {
        self.pointer == id
    }

    /// The finger moved to `x`.
    pub fn moved(&mut self, x: f64) -> Gesture {
        if self.holding {
            return Gesture::Cursor(x);
        }
        let dx = x - self.x0;
        if !self.moved && dx.abs() < DRAG_PX {
            return Gesture::Still;
        }
        self.moved = true;
        Gesture::Pan(dx)
    }

    /// The hold's check at `now`: true once, when the press becomes a hold
    /// (held [`HOLD_MS`] without a drag).
    pub fn check(&mut self, now: f64) -> bool {
        if self.moved || self.holding || now - self.at < HOLD_MS {
            return false;
        }
        self.holding = true;
        true
    }

    /// Whether the cursor shows.
    pub fn holding(&self) -> bool {
        self.holding
    }

    /// Whether it dragged.
    pub fn dragged(&self) -> bool {
        self.moved
    }

    /// Where it went down (px).
    pub fn x0(&self) -> f64 {
        self.x0
    }
}

pub use graph::*;
pub use transport::*;

/// The time ticks' steps in minutes: calm steps, at most 7 ticks; past
/// 7 h (a long past day) whole hours more.
pub const TICK_MINUTES: [i64; 11] = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720];

/// The most time ticks.
pub const MAX_TICKS: i64 = 7;

/// The step of a window `span` ms long: the first with fewer than
/// [`MAX_TICKS`] steps in it, so at most 7 ticks.
pub fn time_step(span: i64) -> i64 {
    TICK_MINUTES
        .iter()
        .map(|m| m * MINUTE_MS)
        .find(|step| span < MAX_TICKS * step)
        .unwrap_or(TICK_MINUTES[TICK_MINUTES.len() - 1] * MINUTE_MS)
}

/// The ticks of `t0..=t1` (UTC ms) at whole local steps (`offset`: local
/// minus UTC, ms). At most [`MAX_TICKS`] + 1: the walk is bounded by a
/// count, never a hand-stepped loop a mutant could spin forever
/// (`.claude/rules/hub-rust.md`; PR #223's mutation shard 37 hung on it).
pub fn time_ticks(t0: i64, t1: i64, offset: i64) -> Vec<i64> {
    let step = time_step(t1 - t0);
    let first = -((-(t0 + offset)).div_euclid(step)) * step - offset;
    (0..=MAX_TICKS)
        .map(|i| first + i * step)
        .take_while(|&t| t <= t1)
        .collect()
}

/// A time's x in the box over `t0..t1`.
pub fn x_of(t: i64, t0: i64, t1: i64) -> f64 {
    (t - t0) as f64 * BOX / (t1 - t0).max(1) as f64
}

/// A time's local `HH:MM`.
pub fn hh_mm(t: i64, offset: i64) -> String {
    let s = (t + offset).rem_euclid(DAY_MS) / SECOND_MS;
    format!("{:02}:{:02}", s / 3600, s / 60 % 60)
}

/// A time's local `HH:MM:SS`.
pub fn hh_mm_ss(t: i64, offset: i64) -> String {
    let s = (t + offset).rem_euclid(DAY_MS) / SECOND_MS;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// A time's local day (days since 1970-01-01).
pub fn local_day(t: i64, offset: i64) -> i64 {
    (t + offset).div_euclid(DAY_MS)
}

const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// A day chip's label: `TODAY`, else the weekday and `D.M`
/// (`SUN 15.11`).
pub fn day_label(day: i64, today: i64) -> String {
    if day == today {
        return "TODAY".to_string();
    }
    // 1970-01-01 was a Thursday.
    let weekday = WEEKDAYS[(day + 4).rem_euclid(7) as usize];
    let (_, month, date) = civil(day);
    format!("{weekday} {date}.{month}")
}

/// A day chip's label and whether it is today, for the day `first_day`
/// against `today` (both days since 1970-01-01; [`day_chip_at`] gives them).
pub fn day_chip(first_day: i64, today: i64) -> (String, bool) {
    (day_label(first_day, today), first_day == today)
}

/// The calendar date of a day since 1970-01-01 (H. Hinnant's
/// `civil_from_days`): year, month 1-12, day 1-31.
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The time under a finger at `frac` of the plot's width (0 left, 1
/// right, clamped).
pub fn cursor_time(frac: f64, t0: i64, t1: i64) -> i64 {
    t0 + (frac.clamp(0.0, 1.0) * (t1 - t0) as f64).round() as i64
}

/// The column a time falls in ([`columns`]), the last for the window's
/// end.
pub fn column_at(t: i64, t0: i64, t1: i64) -> usize {
    let span = (t1 - t0).max(1);
    let n = columns_for(span);
    let i = (t - t0).clamp(0, span) as i128 * n as i128 / span as i128;
    (i as usize).min(n - 1)
}

/// A finger at page `x` over a plot from `left`, `width` px wide: its
/// fraction across (unclamped; [`cursor_time`] clamps).
pub fn x_frac(x: f64, left: f64, width: f64) -> f64 {
    (x - left) / width.max(1.0)
}

/// A place in the box as a CSS percent (`top:`/`left:`), three decimals.
pub fn pct(v: f64) -> String {
    format!("{:.3}%", v * 100.0 / BOX)
}

/// The whole second of `t` (ms).
pub fn second_of(t: i64) -> i64 {
    t - t.rem_euclid(SECOND_MS)
}

/// What the live edge records for a reading: the strip's number, none
/// while the point has a problem (the strip then shows none, as the hub
/// records a hole).
pub fn edge_reading(tenths: Option<i16>, problem: bool) -> Option<i16> {
    if problem { None } else { tenths }
}

/// The live edge's reading, fader and set (#255: the transport and song
/// the hub's series ends on, `last_set`) of `second` into `tail`, once a
/// second (a tick that comes twice in one second records nothing more), the
/// samples before `from` dropped.
pub fn push_reading(
    tail: &mut Vec<Sample>,
    second: i64,
    (reading, fader): (Option<i16>, Option<i16>),
    (p, s): (Option<bool>, Option<u16>),
    from: i64,
) {
    if tail.last().is_none_or(|x| x.t < second) {
        tail.push(Sample {
            t: second,
            v: reading,
            f: fader,
            p,
            s,
        });
    }
    keep_tail(tail, from);
}

/// What the page asks the hub again at a tick (`tick` counts from 1, one a
/// second): the series once a minute while live (`series`), the days once
/// a minute and every 5 s while none is known (`days`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refresh {
    pub series: bool,
    pub days: bool,
}

/// The refresh at tick `tick` ([`Refresh`]).
pub fn refresh(tick: u32, live: bool, days_known: bool) -> Refresh {
    let minute = tick.is_multiple_of(60);
    Refresh {
        series: live && minute,
        days: minute || (!days_known && tick.is_multiple_of(5)),
    }
}

/// The cursor's text in the head: its local time, the value there and,
/// since #253, the fader there (`19:31:07 · −22.8 · FADER −3.5 dB`; `−∞`
/// in silence, `no reading` without one, no fader part while unknown).
pub fn cursor_text(t: i64, col: Column, offset: i64) -> String {
    let value = col.v.map_or_else(|| "no reading".to_string(), text);
    let fader = col
        .f
        .map(|f| format!(" \u{b7} FADER {}", db_text(f)))
        .unwrap_or_default();
    format!("{} \u{b7} {value}{fader}", hh_mm_ss(t, offset))
}

/// An input point's target line in the head: `TARGET −23`.
pub fn target_text(target: Option<i8>) -> Option<String> {
    let t = target?;
    let sign = if t < 0 { "\u{2212}" } else { "" };
    Some(format!("TARGET {sign}{}", i16::from(t).abs()))
}

/// `GET /api/lufs/history` for a point over `from..to` (UTC ms).
pub fn history_url(instance: &str, target: &str, from: i64, to: i64) -> String {
    format!(
        "/api/lufs/history?instance={}&target={}&from={from}&to={to}",
        encode_component(instance),
        encode_component(target)
    )
}

/// `GET /api/lufs/days` for a point.
pub fn days_url(instance: &str, target: &str) -> String {
    format!(
        "/api/lufs/days?instance={}&target={}",
        encode_component(instance),
        encode_component(target)
    )
}

/// An answer of `GET /api/lufs/history`: the series, or why not.
pub fn series_answer(status: u16, body: &str) -> Result<HistorySeries, String> {
    match status {
        200 => serde_json::from_str(body).map_err(|e| format!("unreadable history: {e}")),
        other => Err(format!("HTTP {other}: {}", api_message(body))),
    }
}

/// An answer of `GET /api/lufs/days`: the days, newest first, or why not.
pub fn days_answer(status: u16, body: &str) -> Result<Vec<HistoryDay>, String> {
    match status {
        200 => serde_json::from_str::<HistoryDays>(body)
            .map(|d| d.days)
            .map_err(|e| format!("unreadable days: {e}")),
        other => Err(format!("HTTP {other}: {}", api_message(body))),
    }
}

/// The UTC day of `t` (days since 1970-01-01): the day file it lies in.
pub fn utc_day(t: i64) -> i64 {
    t.div_euclid(DAY_MS)
}

/// A day chip (#219, PR #223's review): its label and whether it is today
/// (the live edge), both by the UTC day its file holds (`day.first_ms`
/// lies in it) against the UTC day of `now`. The hub keeps a file per UTC
/// day, so every chip names another date and at most one is today; a local
/// day would name two files alike around midnight.
pub fn day_chip_at(day: HistoryDay, now: i64) -> (String, bool) {
    day_chip(utc_day(day.first_ms), utc_day(now))
}

/// The time ticks of `t0..=t1` with their labels: placed at whole local
/// steps by the clock at the window's end, each labelled `HH:MM` with its
/// own offset (`offset_of`: local minus UTC at a time), so a window across
/// a DST switch reads as the cursor does (#219, PR #223's review).
pub fn tick_marks(t0: i64, t1: i64, offset_of: impl Fn(i64) -> i64) -> Vec<(i64, String)> {
    time_ticks(t0, t1, offset_of(t1))
        .into_iter()
        .map(|t| (t, hh_mm(t, offset_of(t))))
        .collect()
}

/// The hub's answers to the graph's asks (#219, PR #223's review): the
/// asks numbered, the last successful answer kept apart from the newest
/// ask, so a failed ask never replaces what is drawn and is told apart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answers {
    /// The newest ask's number.
    asked: u64,
    /// The last successful answer.
    answered: Option<HistorySeries>,
    /// The ask whose answer failed last.
    failed: Option<u64>,
}

impl Answers {
    /// A new ask: its number.
    pub fn ask(&mut self) -> u64 {
        self.asked += 1;
        self.asked
    }

    /// The answer to ask `n` (none: it failed). Only the newest ask's
    /// counts; true when its series was taken.
    pub fn answer(&mut self, n: u64, got: Option<HistorySeries>) -> bool {
        if n != self.asked {
            return false;
        }
        match got {
            Some(series) => {
                self.answered = Some(series);
                self.failed = None;
                true
            }
            None => {
                self.failed = Some(n);
                false
            }
        }
    }

    /// The series drawn: the last successful answer.
    pub fn drawn(&self) -> Option<&HistorySeries> {
        self.answered.as_ref()
    }

    /// Whether the newest ask failed (what is drawn answers an older one).
    pub fn failed(&self) -> bool {
        self.failed == Some(self.asked)
    }
}

/// Whether tick `tick` asks the series again after the newest ask failed:
/// every 5 s, a past window too (#219).
pub fn retry_series(tick: u32, failed: bool) -> bool {
    failed && tick.is_multiple_of(5)
}

mod graph;
mod transport;

#[cfg(test)]
mod tests;
