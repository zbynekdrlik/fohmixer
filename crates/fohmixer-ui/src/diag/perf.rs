//! The page's frame rate and simultaneous touches (#5, K4): what a `perf`
//! report carries, so the hub log says how smoothly a real tablet draws the
//! surface and how many fingers it saw at once, without anyone watching it.
//!
//! Pure, tested natively. [`Perf`] counts the frames of the one animation
//! loop (`raf::tick` hands each frame's gap to the previous one, the glue
//! adds the page clock's now) over windows of [`WINDOW_MS`] of frames: a window gives the frame
//! rate ([`fps`]) and its longest gap. A visibility change pauses the count
//! ([`Perf::pause`]): a hidden page draws no frames, and the gap the first
//! frame after it carries is the time it was hidden. When a window closes a
//! periodic report is due, at most once per [`PERF_GAP_MS`] ([`perf_due`]).
//!
//! It also counts the pointers down at once ([`Perf::down`], [`Perf::up`]):
//! the most since the last report (`touches_max`), and a report of its own
//! when the page sees [`TOUCHES_REPORTED`] or more at once, more than ever
//! before. A report ([`Perf::report`]) says the last window's numbers, the
//! most pointers at once since the report before and the latest pointer's
//! type; [`PerfReport::fill`] writes them into the report's fields.

use fohmixer_proto::client::ReportFields;

/// A window of frames: the rate and the longest gap are measured over this
/// many ms of frames.
pub const WINDOW_MS: f64 = 10_000.0;
/// A periodic perf report goes at most once per this interval (page clock,
/// ms).
pub const PERF_GAP_MS: f64 = 60_000.0;
/// Pointers at once that make a report of their own when the page has not
/// seen as many before (one finger is every touch).
pub const TOUCHES_REPORTED: u32 = 2;
/// The most pointers counted as down at once (a bound, should the browser
/// lose some `pointerup`s).
pub const POINTERS_MAX: usize = 20;

/// A pointer's type (`PointerEvent.pointerType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Touch,
    Mouse,
    Pen,
}

impl Pointer {
    /// The type a browser names; none for another or an empty one.
    pub fn parse(pointer_type: &str) -> Option<Pointer> {
        match pointer_type {
            "touch" => Some(Pointer::Touch),
            "mouse" => Some(Pointer::Mouse),
            "pen" => Some(Pointer::Pen),
            _ => None,
        }
    }

    /// The name a report carries.
    pub fn name(self) -> &'static str {
        match self {
            Pointer::Touch => "touch",
            Pointer::Mouse => "mouse",
            Pointer::Pen => "pen",
        }
    }
}

/// A closed window of frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// Frames per second.
    pub fps: f64,
    /// The longest gap between two frames (ms).
    pub long_ms: f64,
}

/// Whether `span` ms of frames fill a window.
pub fn window_full(span: f64) -> bool {
    span >= WINDOW_MS
}

/// The frame rate of `frames` frames over `span` ms.
pub fn fps(frames: u32, span: f64) -> f64 {
    f64::from(frames) * 1000.0 / span
}

/// Whether a periodic report is due at `now` after the last perf report at
/// `last`: none sent yet, [`PERF_GAP_MS`] passed, or a clock that went
/// backwards.
pub fn perf_due(now: f64, last: Option<f64>) -> bool {
    last.is_none_or(|t| now - t >= PERF_GAP_MS || now < t)
}

/// What a perf report says.
#[derive(Debug, Clone, PartialEq)]
pub struct PerfReport {
    /// The last closed window; none before the first one (or after a pause).
    pub window: Option<Window>,
    /// The most pointers down at once since the report before.
    pub touches_max: u32,
    /// The latest pointer's type.
    pub pointer: Option<Pointer>,
}

impl PerfReport {
    /// Writes the report's numbers into its fields as short words: the rate
    /// with one decimal, the longest gap in whole ms.
    pub fn fill(&self, fields: &mut ReportFields) {
        fields.fps = self.window.map(|w| format!("{:.1}", w.fps));
        fields.long_frame_ms = self.window.map(|w| format!("{:.0}", w.long_ms));
        fields.touches_max = Some(self.touches_max.to_string());
        fields.pointer = self.pointer.map(|p| p.name().to_string());
    }
}

/// The page's frame and pointer counts.
#[derive(Debug, Clone, Default)]
pub struct Perf {
    /// Frames in the open window.
    frames: u32,
    /// The open window's time: the sum of its frames' gaps (ms).
    span: f64,
    /// The open window's longest gap (ms).
    long: f64,
    /// The next frame's gap is not counted (the page was hidden or shown).
    skip: bool,
    /// The last closed window.
    window: Option<Window>,
    /// The pointers down: id and type.
    down: Vec<(i32, Option<Pointer>)>,
    /// The most pointers down at once since the last report.
    touches_max: u32,
    /// The most pointers down at once on this page that made a report.
    high: u32,
    /// The latest pointer's type.
    pointer: Option<Pointer>,
    /// When the last perf report went (page clock, ms).
    reported: Option<f64>,
}

impl Perf {
    /// A frame at `now`, `gap` ms after the one before (a negative gap, a
    /// frame time a little before the loop started, counts as none): true
    /// when it closed a window and a periodic report is due.
    pub fn frame(&mut self, now: f64, gap: f64) -> bool {
        if std::mem::take(&mut self.skip) {
            return false;
        }
        let gap = gap.max(0.0);
        self.frames += 1;
        self.span += gap;
        self.long = self.long.max(gap);
        if !window_full(self.span) {
            return false;
        }
        self.window = Some(Window {
            fps: fps(self.frames, self.span),
            long_ms: self.long,
        });
        self.clear_window();
        perf_due(now, self.reported)
    }

    /// The page was hidden or shown: the open window and the last one are
    /// dropped, the next frame's gap is not counted, and no pointer is down.
    pub fn pause(&mut self) {
        self.clear_window();
        self.window = None;
        self.skip = true;
        self.down.clear();
    }

    /// A pointer `id` of type `pointer_type` went down (`primary`: the first
    /// of its type, so the others of that type still counted lost their
    /// up): true when the page has never had as many at once, and at least
    /// [`TOUCHES_REPORTED`].
    pub fn down(&mut self, id: i32, pointer_type: &str, primary: bool) -> bool {
        let pointer = Pointer::parse(pointer_type);
        if primary {
            self.down.retain(|&(_, kind)| kind != pointer);
        }
        let known = self.down.iter().any(|&(down_id, _)| down_id == id);
        if !known && self.down.len() < POINTERS_MAX {
            self.down.push((id, pointer));
        }
        self.pointer = pointer;
        let now_down = self.count();
        self.touches_max = self.touches_max.max(now_down);
        if now_down > self.high && now_down >= TOUCHES_REPORTED {
            self.high = now_down;
            return true;
        }
        false
    }

    /// A pointer went up or was cancelled.
    pub fn up(&mut self, id: i32) {
        self.down.retain(|&(down_id, _)| down_id != id);
    }

    /// Whether pointer `id` is down (#43 PR G: a capture it loses now is no
    /// lift's, `diag::trace::sys::records`). One whose up the page missed
    /// goes when a primary pointer of its type comes, or the page is hidden
    /// or shown.
    pub fn is_down(&self, id: i32) -> bool {
        self.down.iter().any(|&(down_id, _)| down_id == id)
    }

    /// A perf report goes at `now`: what it says. The most pointers at once
    /// starts again from those down now.
    pub fn report(&mut self, now: f64) -> PerfReport {
        let report = PerfReport {
            window: self.window,
            touches_max: self.touches_max,
            pointer: self.pointer,
        };
        self.reported = Some(now);
        self.touches_max = self.count();
        report
    }

    /// The pointers down now.
    fn count(&self) -> u32 {
        self.down.len() as u32
    }

    /// Empties the open window.
    fn clear_window(&mut self) {
        self.frames = 0;
        self.span = 0.0;
        self.long = 0.0;
    }
}

#[cfg(test)]
mod tests;
