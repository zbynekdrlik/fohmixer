//! The LUFS history's graph (#253, the owner's approved mockup
//! `docs/mockups/lufs-history-v3.html`; split from `lufs_history.rs`):
//! the samples in columns (each the gated energy mean of its readings, then
//! glided 1-2-1), the scale (a tick every 3 LU), the line (monotone cubic
//! curves, green the same path clipped to the band) and its live end, and
//! the strip's fader on its own dB scale. Everything in the `0 0 BOX BOX`
//! box; pure and tested.

use fohmixer_proto::lufs::{
    FADER_FLOOR_TENTHS, HistorySeries, SILENT_TENTHS, bucket_of, in_band, median, play_of, song_of,
};

use super::{BOX, SECOND_MS};

/// The graph's columns: values drawn across the plot (#253: 120, 5 s a
/// column in `10 MIN`, 30 s in `1 H`, 90 s in `3 H`).
pub const COLUMNS: usize = 120;

/// The columns of a window `span` ms long: [`COLUMNS`], or one a second
/// for a window shorter than that (a short past day), so readings a second
/// apart draw a line, never lone points.
pub fn columns_for(span: i64) -> usize {
    (span / SECOND_MS).clamp(1, COLUMNS as i64) as usize
}

/// One second of a point (a hub bucket, or the page's own second at the
/// live edge): its time (UTC ms), reading and strip's fader (dB tenths,
/// #253), and since #255 its Live set's transport (`p`) and song (`s`),
/// each none when unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub t: i64,
    pub v: Option<i16>,
    pub f: Option<i16>,
    pub p: Option<bool>,
    pub s: Option<u16>,
}

/// The series' samples: each value at its bucket's start, with its fader,
/// its set's transport and song (none from an older hub, which sends none).
pub fn samples(series: &HistorySeries) -> Vec<Sample> {
    series
        .values
        .iter()
        .enumerate()
        .map(|(i, v)| Sample {
            t: series.from + i as i64 * series.step_ms,
            v: *v,
            f: series.fader.get(i).copied().flatten(),
            p: series.play.get(i).copied().flatten(),
            s: series.song.get(i).copied().flatten(),
        })
        .collect()
}

/// The set's transport and song the series ends on (#255, its last
/// bucket's): what the live edge's own seconds carry until the next answer
/// (the mockup's "as at the last second").
pub fn last_set(series: &HistorySeries) -> (Option<bool>, Option<u16>) {
    (
        series.play.last().copied().flatten(),
        series.song.last().copied().flatten(),
    )
}

/// The page's own samples after the series' end: one a second at the live
/// edge (`tail`), kept only from `from` on.
pub fn keep_tail(tail: &mut Vec<Sample>, from: i64) {
    tail.retain(|s| s.t >= from);
}

/// Where the hub's series ends (its last bucket's end); `from` when empty.
pub fn series_end(series: &HistorySeries) -> i64 {
    series.from + series.values.len() as i64 * series.step_ms
}

/// A column of the graph: its reading ([`columns`]), its fader (the
/// median of its seconds' fader, none without one) and its set's transport
/// and song (#255: `fohmixer_proto::lufs::play_of` of its known samples,
/// `song_of` of its playing samples' songs while it plays).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Column {
    pub v: Option<i16>,
    pub f: Option<i16>,
    pub p: Option<bool>,
    pub s: Option<u16>,
}

/// The samples in [`columns_for`] columns over `t0..t1`: each column's
/// reading by the hub's own bucket rule (`fohmixer_proto::lufs::bucket_of`:
/// none without a reading, silence when half or more are silent, else the
/// gated energy mean of those above silence, #253) and its fader (the
/// median of the faders known). In `10 MIN` the hub's buckets are its
/// seconds; in `1 H` and `3 H` they are 4–5 s and 12–13 s buckets gated by
/// the same rule, so a column is the gated loudness of gated buckets.
pub fn columns(samples: &[Sample], t0: i64, t1: i64) -> Vec<Column> {
    let span = (t1 - t0).max(1);
    let n = columns_for(span);
    let mut readings: Vec<Vec<i16>> = vec![Vec::new(); n];
    let mut faders: Vec<Vec<i16>> = vec![Vec::new(); n];
    let mut plays: Vec<Vec<bool>> = vec![Vec::new(); n];
    let mut songs: Vec<Vec<u16>> = vec![Vec::new(); n];
    for s in samples {
        if s.t < t0 || s.t >= t1 {
            continue;
        }
        let i = ((s.t - t0) as i128 * n as i128 / span as i128) as usize;
        readings[i].extend(s.v);
        faders[i].extend(s.f);
        plays[i].extend(s.p);
        songs[i].extend(s.s.filter(|_| s.p == Some(true)));
    }
    (0..n)
        .map(|i| {
            let p = play_of(&plays[i]);
            Column {
                v: bucket_of(&readings[i]),
                f: median(&faders[i]),
                p,
                s: song_of(&songs[i]).filter(|_| p == Some(true)),
            }
        })
        .collect()
}

/// A value that draws: a reading above silence.
fn sounding(v: Option<i16>) -> Option<i16> {
    v.filter(|v| *v > SILENT_TENTHS)
}

/// The short glide over neighbours (#253): each sounding value as
/// `(left + 2 × it + right) / 4`, a neighbour that is silence or none left
/// out (the weights then 2 and 1, or it alone), rounded; never across a
/// gap: silence and none stay as they are.
pub fn glide(values: &[Option<i16>]) -> Vec<Option<i16>> {
    let at = |i: Option<usize>| i.and_then(|i| values.get(i).copied().flatten());
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let c = sounding(*v)?;
            let sides: Vec<f64> = [at(i.checked_sub(1)), at(Some(i + 1))]
                .into_iter()
                .filter_map(sounding)
                .map(f64::from)
                .collect();
            let sum = 2.0 * f64::from(c) + sides.iter().sum::<f64>();
            Some((sum / (2.0 + sides.len() as f64)).round() as i16)
        })
        .zip(values)
        .map(|(glided, v)| glided.or(*v))
        .collect()
}

/// The columns the graph draws (#253): [`columns`], each reading then
/// [`glide`]d over its neighbours; the fader as its column holds it.
pub fn drawn_columns(samples: &[Sample], t0: i64, t1: i64) -> Vec<Column> {
    let raw = columns(samples, t0, t1);
    let glided = glide(&raw.iter().map(|c| c.v).collect::<Vec<_>>());
    raw.iter()
        .zip(glided)
        .map(|(c, v)| Column {
            v,
            f: c.f,
            p: c.p,
            s: c.s,
        })
        .collect()
}

/// Column `i`'s x in the box (of `n` columns): its middle.
pub fn column_x(i: usize, n: usize) -> f64 {
    (i as f64 + 0.5) * BOX / n.max(1) as f64
}

/// The graph's scale in tenths: `lo` at the bottom, `hi` at the top, the
/// labelled ticks, and an input point's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scale {
    pub lo: i32,
    pub hi: i32,
    pub ticks: Vec<i32>,
    pub target: Option<i32>,
}

/// An input point's half range: ±12 LU.
pub const IN_HALF: i32 = 120;
/// A label and a grid line every 3 LU (#253).
pub const TICK_STEP: i32 = 30;
/// An output point's step: 6 LU.
pub const OUT_STEP: i32 = 60;
/// An output point's least range: 18 LU.
pub const OUT_MIN: i32 = 180;
/// An output point's range with no reading: −30 to −12.
pub const OUT_EMPTY: (i32, i32) = (-300, -120);

/// The scale: an input point's target ±12 LU, an output point's readings
/// in 6 LU steps, at least 18 LU; a tick every [`TICK_STEP`] (3 LU, #253),
/// an input point's target one of them.
pub fn scale(target: Option<i8>, values: &[Option<i16>]) -> Scale {
    if let Some(target) = target {
        let t = i32::from(target) * 10;
        return Scale {
            lo: t - IN_HALF,
            hi: t + IN_HALF,
            ticks: (t - IN_HALF..=t + IN_HALF)
                .step_by(TICK_STEP as usize)
                .collect(),
            target: Some(t),
        };
    }
    let sounding: Vec<i32> = values
        .iter()
        .flatten()
        .filter(|v| **v > SILENT_TENTHS)
        .map(|v| i32::from(*v))
        .collect();
    let (lo, hi) = match (sounding.iter().min(), sounding.iter().max()) {
        (Some(min), Some(max)) => out_range(*min, *max),
        _ => OUT_EMPTY,
    };
    Scale {
        lo,
        hi,
        ticks: (lo..=hi).step_by(TICK_STEP as usize).collect(),
        target: None,
    }
}

/// An output point's range for readings `min..=max`: whole 6 LU steps
/// around them, widened about their middle to at least 18 LU.
pub fn out_range(min: i32, max: i32) -> (i32, i32) {
    let lo = min.div_euclid(OUT_STEP) * OUT_STEP;
    let hi = -((-max).div_euclid(OUT_STEP)) * OUT_STEP;
    if hi - lo >= OUT_MIN {
        return (lo, hi);
    }
    let lo = (hi + lo - OUT_MIN).div_euclid(2 * OUT_STEP) * OUT_STEP;
    (lo, lo + OUT_MIN)
}

/// A value's y in the box (`hi` at 0), a value past an edge on the edge.
pub fn y_of(v: i32, scale: &Scale) -> f64 {
    let v = v.clamp(scale.lo, scale.hi);
    BOX - f64::from(v - scale.lo) * BOX / f64::from(scale.hi - scale.lo)
}

/// The band's two dashed lines: an input point's target ±1.0 LU.
pub fn band(scale: &Scale) -> Option<(f64, f64)> {
    let t = scale.target?;
    let b = i32::from(fohmixer_proto::lufs::BAND_TENTHS);
    Some((y_of(t + b, scale), y_of(t - b, scale)))
}

/// A tick's label: whole LU with a typographic minus.
pub fn tick_label(tenths: i32) -> String {
    let lu = tenths / 10;
    if lu < 0 {
        format!("\u{2212}{}", -lu)
    } else {
        lu.to_string()
    }
}

/// Whether a reading draws green: an input point within ±1.0 LU of its
/// target.
pub fn green(v: i16, target: Option<i8>) -> bool {
    target.is_some_and(|t| in_band(v, t))
}

/// A monotone cubic curve through `points` (box coordinates) as an SVG
/// path (#253, Fritsch-Carlson as the mockup's `smooth`): it passes every
/// point and never swings past them; none for under two points.
pub fn curve(points: &[(f64, f64)]) -> String {
    let n = points.len();
    if n < 2 {
        return String::new();
    }
    let d: Vec<f64> = points
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0))
        .collect();
    let mut m = Vec::with_capacity(n);
    m.push(d[0]);
    m.extend(d.windows(2).map(|w| {
        if w[0] * w[1] <= 0.0 {
            0.0
        } else {
            (w[0] + w[1]) / 2.0
        }
    }));
    m.push(d[n - 2]);
    for (i, di) in d.iter().enumerate() {
        // A flat segment stays flat: both its tangents are already 0 (an
        // inner one by the sign rule above, an end one is the segment's own
        // slope), so it is skipped, never divided by. Elsewhere tangents past
        // a radius of 3 (against the segment's slope) are scaled back onto it.
        if *di == 0.0 {
            continue;
        }
        let k = (3.0 / (m[i] / di).hypot(m[i + 1] / di)).min(1.0);
        m[i] *= k;
        m[i + 1] *= k;
    }
    let mut out = format!("M{:.1},{:.1}", points[0].0, points[0].1);
    for (i, w) in points.windows(2).enumerate() {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let h = (x1 - x0) / 3.0;
        out.push_str(&format!(
            "C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            x0 + h,
            y0 + m[i] * h,
            x1 - h,
            y1 - m[i + 1] * h,
            x1,
            y1
        ));
    }
    out
}

/// The line (#253): one [`curve`] per run of sounding values, cut at
/// silence and no reading; a run of one point draws nothing. Green is the
/// same path clipped to the band (the glue's `clipPath`).
pub fn lines(values: &[Option<i16>], scale: &Scale) -> Vec<String> {
    let n = values.len();
    let mut out = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        match sounding(*v) {
            Some(v) => run.push((column_x(i, n), y_of(i32::from(v), scale))),
            None => out.push(curve(&std::mem::take(&mut run))),
        }
    }
    out.push(curve(&run));
    out.retain(|path| !path.is_empty());
    out
}

/// Whether the graph says `NO LUFS IN THIS WINDOW` (#253): neither a run of
/// the line nor its live end's dot is drawn (a single sounding column at
/// the live edge draws a dot and no curve yet, and is no empty window).
pub fn empty_window(runs: usize, end: Option<(usize, i16)>) -> bool {
    runs == 0 && end.is_none()
}

/// The line's live end (#253): the last column that sounds and its value,
/// where the graph draws its dot; none without one, and none unless the
/// view is `live` (a window dragged back or a past day has no live end).
pub fn live_end(live: bool, values: &[Option<i16>]) -> Option<(usize, i16)> {
    values
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, v)| sounding(*v).map(|v| (i, v)))
        .filter(|_| live)
}

/// Where the fader's 0 dB sits on the scale (#253, 1 dB as tall as 1 LU):
/// an input point's target, an output point's scale's middle (whole LU:
/// the scale's ends are whole 6 LU steps).
pub fn fader_zero(scale: &Scale) -> i32 {
    scale.target.unwrap_or((scale.lo + scale.hi) / 20 * 10)
}

/// A fader's y in the box: its dB on the LUFS scale from [`fader_zero`]
/// (past an edge on the edge, −∞ at the bottom).
pub fn fader_y(f: i16, scale: &Scale) -> f64 {
    y_of(fader_zero(scale) + i32::from(f), scale)
}

/// The fader scale's label at a tick (#253): its whole dB from
/// [`fader_zero`], `+` above it, a typographic minus under it.
pub fn fader_label(tick: i32, scale: &Scale) -> String {
    let db = (tick - fader_zero(scale)) / 10;
    match db {
        d if d > 0 => format!("+{d}"),
        d if d < 0 => format!("\u{2212}{}", -d),
        _ => "0".to_string(),
    }
}

/// The fader's line (#253): runs of columns with a fader, each an SVG
/// `polyline`'s points (straight between columns: a fader moves in steps),
/// cut where no fader is known; a run of one column draws nothing.
pub fn fader_lines(cols: &[Column], scale: &Scale) -> Vec<String> {
    let n = cols.len();
    let mut out = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for (i, c) in cols.iter().enumerate() {
        match c.f {
            Some(f) => run.push(format!("{:.1},{:.1}", column_x(i, n), fader_y(f, scale))),
            None => out.push(std::mem::take(&mut run)),
        }
    }
    out.push(run);
    out.into_iter()
        .filter(|r| r.len() > 1)
        .map(|r| r.join(" "))
        .collect()
}

/// A fader's dB in tenths as text (#253): one decimal, `+` above 0, a
/// typographic minus under it, `−∞` at the floor: `−4.5 dB`.
pub fn db_text(f: i16) -> String {
    if f <= FADER_FLOOR_TENTHS {
        return "\u{2212}\u{221e} dB".to_string();
    }
    let sign = match f {
        f if f > 0 => "+",
        f if f < 0 => "\u{2212}",
        _ => "",
    };
    let abs = f.unsigned_abs();
    format!("{sign}{}.{} dB", abs / 10, abs % 10)
}

/// The head's fader beside the target (#253): `FADER −4.5 dB`, none while
/// the fader is unknown.
pub fn fader_text(f: Option<i16>) -> Option<String> {
    f.map(|f| format!("FADER {}", db_text(f)))
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
