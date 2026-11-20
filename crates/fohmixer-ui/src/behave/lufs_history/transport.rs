//! The LUFS history's Live set in the graph (#255, the owner's approved
//! mockup `docs/mockups/lufs-history-v4.html`): the transport lane under
//! the plot (an empty track where the set's transport is known, a dim bar
//! where it plays, nothing where unknown), each song's mark (a faint dashed
//! line at its start, the song's `031 Name` at the plot's top, cut with an
//! ellipsis to its span, left out and the line at half strength when the
//! span is too narrow), the cursor's `· PLAY 031 Name` / `· STOP`, and the
//! time labels that give way to the axes' names in the corners, and the
//! live edge's set as the page hears it (`live_set`, `edge_set`). Pure and
//! tested; the glue (`components/detail_lufs/graph.rs`) measures only the
//! plot's width, never a text: a label's width is estimated from the
//! display face's advances.

use std::collections::BTreeMap;

use fohmixer_proto::client::{AutotuneState, REC_INSTANCE, RecState};

use super::graph::Column;
use super::{BOX, pct};

/// A song's span narrower than this (px) shows no name and its line at
/// half strength (the mockup's 40 px: a dense window stays calm).
pub const LABEL_MIN_PX: f64 = 40.0;
/// A label's inset from its span's start and end (px).
pub const LABEL_PAD_PX: f64 = 4.0;
/// The label's face: Barlow Condensed 700 at 11 px, 0.04 em spacing
/// (`history.css` `.lh-song`).
pub const LABEL_FONT_PX: f64 = 11.0;
pub const LABEL_SPACING_EM: f64 = 0.04;
/// A time label closer than this to the plot's left edge (px) would meet
/// `LUFS` in the corner; one closer than [`TIME_RIGHT_PX`] to its right
/// edge `FADER dB` (the mockup's `x > left + 10 && x < right - 32`).
pub const TIME_LEFT_PX: f64 = 10.0;
pub const TIME_RIGHT_PX: f64 = 32.0;

/// Barlow Condensed 700's advances (units of 1000 per em) of the printable
/// ASCII characters, space to `~`, read from the bundled
/// `fonts/barlow-condensed-latin-700-normal.woff2`.
const ADVANCES: [u16; 95] = [
    200, 279, 346, 616, 444, 766, 603, 170, 311, 311, 361, 438, 211, 331, 223, 415, 453, 284, 438,
    436, 484, 439, 440, 407, 440, 435, 275, 232, 438, 438, 438, 437, 759, 482, 470, 464, 476, 438,
    421, 467, 480, 230, 452, 491, 426, 548, 514, 473, 465, 461, 471, 444, 468, 479, 488, 689, 475,
    474, 412, 345, 415, 345, 422, 421, 220, 446, 445, 434, 445, 436, 298, 441, 447, 219, 217, 449,
    209, 675, 447, 442, 448, 448, 324, 412, 291, 447, 436, 613, 439, 423, 373, 344, 186, 344, 490,
];
/// The ellipsis' advance in the same face.
const ELLIPSIS_UNITS: u16 = 709;
/// Any other character: the widest letter's (`W`), so an estimate never
/// falls short.
const WIDE_UNITS: u16 = 689;

/// One character's width in the label's face (px), its spacing included.
pub fn char_px(c: char) -> f64 {
    let units = match c {
        ' '..='~' => ADVANCES[c as usize - ' ' as usize],
        '\u{2026}' => ELLIPSIS_UNITS,
        _ => WIDE_UNITS,
    };
    f64::from(units) * LABEL_FONT_PX / 1000.0 + LABEL_SPACING_EM * LABEL_FONT_PX
}

/// A text's width in the label's face (px).
pub fn text_px(text: &str) -> f64 {
    text.chars().map(char_px).sum()
}

/// `text` cut to fit `room` px: whole when it fits, else its longest head
/// (trailing spaces dropped) with an ellipsis that fits, none when not even
/// one character and the ellipsis do.
pub fn cut_text(text: &str, room: f64) -> Option<String> {
    if text_px(text) <= room {
        return Some(text.to_string());
    }
    let chars: Vec<char> = text.chars().collect();
    (1..chars.len()).rev().find_map(|n| {
        let head: String = chars[..n].iter().collect();
        let cut = format!("{}\u{2026}", head.trim_end());
        (text_px(&cut) <= room).then_some(cut)
    })
}

/// A song as the graph and the cursor name it: `031 Morning Light`, its
/// number alone when the hub knows no name.
pub fn song_text(number: u16, names: &BTreeMap<u16, String>) -> String {
    match names.get(&number).filter(|n| !n.is_empty()) {
        Some(name) => format!("{number:03} {name}"),
        None => format!("{number:03}"),
    }
}

/// The runs of columns `on` holds: each its first column and the column
/// after its last.
pub fn runs(cols: &[Column], on: impl Fn(&Column) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = None;
    for (i, c) in cols.iter().enumerate() {
        match (on(c), from) {
            (true, None) => from = Some(i),
            (false, Some(a)) => {
                out.push((a, i));
                from = None;
            }
            _ => {}
        }
    }
    if let Some(a) = from {
        out.push((a, cols.len()));
    }
    out
}

/// The lane's track: the runs where the set's transport is known.
pub fn known_runs(cols: &[Column]) -> Vec<(usize, usize)> {
    runs(cols, |c| c.p.is_some())
}

/// The lane's bars: the runs where the set plays.
pub fn play_runs(cols: &[Column]) -> Vec<(usize, usize)> {
    runs(cols, |c| c.p == Some(true))
}

/// The songs' spans: runs of playing columns that hold one song (a
/// playing column outside every song makes no span).
pub fn song_spans(cols: &[Column]) -> Vec<(usize, usize, u16)> {
    let mut out: Vec<(usize, usize, u16)> = Vec::new();
    for (i, c) in cols.iter().enumerate() {
        let Some(song) = c.s.filter(|_| c.p == Some(true)) else {
            continue;
        };
        match out.last_mut() {
            Some(last) if last.1 == i && last.2 == song => last.1 = i + 1,
            _ => out.push((i, i + 1, song)),
        }
    }
    out
}

/// A song mark's line at its span's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// The span began before the window: its name at the left edge, no line.
    None,
    /// Too narrow for its name: the line at half strength.
    Faint,
    Full,
}

impl Line {
    /// Its word (`data-line`).
    pub fn name(self) -> &'static str {
        match self {
            Line::None => "none",
            Line::Faint => "faint",
            Line::Full => "full",
        }
    }
}

/// A song's mark in the graph: its span (columns), its song, its line,
/// its label (none when the span is too narrow to say anything) and the
/// label's room (px: the span less [`LABEL_PAD_PX`] on each side, the
/// label's `max-width`, an ellipsis the last guard).
#[derive(Debug, Clone, PartialEq)]
pub struct SongMark {
    pub from: usize,
    pub to: usize,
    pub song: u16,
    pub line: Line,
    pub label: Option<String>,
    pub room: f64,
}

/// The songs' marks over a plot `plot_px` wide ([`song_spans`]): a span
/// under [`LABEL_MIN_PX`] keeps a faint line and no name; a wider one its
/// name cut to the span less [`LABEL_PAD_PX`] on each side.
pub fn song_marks(cols: &[Column], names: &BTreeMap<u16, String>, plot_px: f64) -> Vec<SongMark> {
    let col_px = plot_px / cols.len().max(1) as f64;
    song_spans(cols)
        .into_iter()
        .map(|(from, to, song)| {
            let width = (to - from) as f64 * col_px;
            let room = width - 2.0 * LABEL_PAD_PX;
            let wide = width >= LABEL_MIN_PX;
            let line = match (from, wide) {
                (0, _) => Line::None,
                (_, true) => Line::Full,
                (_, false) => Line::Faint,
            };
            let label = wide
                .then(|| cut_text(&song_text(song, names), room))
                .flatten();
            SongMark {
                from,
                to,
                song,
                line,
                label,
                room,
            }
        })
        .collect()
}

/// Column `i`'s left edge in the box (of `n` columns).
pub fn column_edge(i: usize, n: usize) -> f64 {
    i as f64 * BOX / n.max(1) as f64
}

/// A run of columns `from..to` (of `n`) as CSS: its left edge and width in
/// percent of the plot.
pub fn run_style(from: usize, to: usize, n: usize) -> String {
    let left = column_edge(from, n);
    format!(
        "left:{};width:{}",
        pct(left),
        pct(column_edge(to, n) - left)
    )
}

/// Whether a time label at `frac` across a plot `plot_px` wide shows: not
/// into the corners' `LUFS` and `FADER dB`; all show while the plot is not
/// measured yet.
pub fn time_label_shows(frac: f64, plot_px: f64) -> bool {
    if plot_px <= 0.0 {
        return true;
    }
    let x = frac * plot_px;
    x > TIME_LEFT_PX && x < plot_px - TIME_RIGHT_PX
}

/// The point's Live set as the page hears it now (#255): its transport and
/// its song, each `None` when not heard live and `Some(None)` when heard
/// live as unknown (Live offline: no transport; out of every song: no
/// song).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveSet {
    pub playing: Option<Option<bool>>,
    pub song: Option<Option<u16>>,
}

/// What the page hears live of `instance`'s Live set with no subscription
/// of its own (#255): AUTOTUNE's state (`autotune`: the set of its first
/// target and the state the hub told this connection) names the set's
/// transport and song; the recording watch's (`rec`, `REC_INSTANCE`'s)
/// its transport when AUTOTUNE's does not know it; else nothing. A state
/// told this connection that knows no transport is heard as unknown.
pub fn live_set(
    instance: &str,
    autotune: Option<(&str, &AutotuneState)>,
    rec: Option<&RecState>,
) -> LiveSet {
    let rec_playing = rec.filter(|_| instance == REC_INSTANCE).map(|r| r.playing);
    match autotune.filter(|(set, _)| *set == instance) {
        Some((_, state)) => LiveSet {
            playing: Some(state.playing.or(rec_playing.flatten())),
            song: Some(state.song),
        },
        None => LiveSet {
            playing: rec_playing,
            song: None,
        },
    }
}

/// The set a live edge's second records (#255): the transport heard live
/// (unknown too: Live offline is no play), else the one the hub's series
/// ends on (`last`, `last_set`); the song
/// heard live, else the series' last song while that bucket played; no
/// song while the set does not play. So the lane follows a start or a
/// stop at once, and a window in the past (which records no live second)
/// is never touched.
pub fn edge_set(live: LiveSet, last: (Option<bool>, Option<u16>)) -> (Option<bool>, Option<u16>) {
    let playing = live.playing.unwrap_or(last.0);
    let song = match live.song {
        Some(song) => song,
        None => last.1.filter(|_| last.0 == Some(true)),
    };
    (playing, song.filter(|_| playing == Some(true)))
}

/// Ticks (seconds) between two asks of the hub for a song's name.
pub const NAME_RETRY_TICKS: u32 = 5;

/// Whether the page asks the hub's series again for the name of the song
/// heard live (#255): its name is not known yet, and no ask for it went in
/// the last [`NAME_RETRY_TICKS`] ticks (`last_ask`: the tick of the last
/// one), so a song started after the last answer is named within seconds.
pub fn name_due(
    song: Option<u16>,
    names: &BTreeMap<u16, String>,
    last_ask: Option<u32>,
    tick: u32,
) -> bool {
    song.is_some_and(|n| !names.contains_key(&n))
        && last_ask.is_none_or(|t| tick >= t + NAME_RETRY_TICKS)
}

/// The cursor's second line (#255): `· PLAY 031 Morning Light`, `· PLAY`
/// outside every song, `· STOP`, nothing while the transport is unknown.
pub fn transport_text(col: Column, names: &BTreeMap<u16, String>) -> String {
    match (col.p, col.s) {
        (Some(true), Some(song)) => format!("\u{b7} PLAY {}", song_text(song, names)),
        (Some(true), None) => "\u{b7} PLAY".to_string(),
        (Some(false), _) => "\u{b7} STOP".to_string(),
        (None, _) => String::new(),
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
