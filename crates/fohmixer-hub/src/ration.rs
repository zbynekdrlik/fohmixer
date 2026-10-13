//! A ration of records or log lines of one kind (#90): at most so many per
//! [`GAP`], and how many were left out since the last one written, so a
//! client in a loop can neither flood the hub's log nor use up the day's
//! event-log space (`events.rs`: past 256 MB a day only warn-class records
//! are written), while the count of what was left out stays on the record.

use std::time::{Duration, Instant};

/// The window of a ration.
pub const GAP: Duration = Duration::from_secs(10);

/// One kind's ration: the window it is in, what it used of it, and what it
/// left out since the last record it allowed.
#[derive(Debug, Default)]
pub struct Ration {
    window: Option<Instant>,
    used: u32,
    skipped: u64,
}

impl Ration {
    /// Whether one more is allowed at `now`, at most `per_gap` in a window
    /// of [`GAP`] (the first one opens it): the count left out before it
    /// (then back to 0), or none when it is left out too (counted).
    pub fn take(&mut self, now: Instant, per_gap: u32) -> Option<u64> {
        if self
            .window
            .is_none_or(|start| now.saturating_duration_since(start) >= GAP)
        {
            self.window = Some(now);
            self.used = 0;
        }
        if self.used < per_gap {
            self.used += 1;
            Some(std::mem::take(&mut self.skipped))
        } else {
            self.skipped += 1;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ration_allows_so_many_per_gap_and_counts_what_it_left_out() {
        assert_eq!(GAP, Duration::from_secs(10));
        let t = Instant::now();
        let ms = Duration::from_millis;
        let mut one = Ration::default();
        let taken: Vec<Option<u64>> = [ms(0), ms(1), ms(9_999), GAP, GAP + ms(1), GAP * 3]
            .iter()
            .map(|&at| one.take(t + at, 1))
            .collect();
        assert_eq!(taken, [Some(0), None, None, Some(2), None, Some(1)]);
        let mut two = Ration::default();
        let taken: Vec<Option<u64>> = [ms(0), ms(1), ms(2), ms(3), GAP + ms(1)]
            .iter()
            .map(|&at| two.take(t + at, 2))
            .collect();
        assert_eq!(taken, [Some(0), Some(0), None, None, Some(2)]);
        // A clock read before the window's start (another thread's older
        // `now`) stays in the window.
        let mut late = Ration::default();
        assert_eq!(late.take(t + GAP, 1), Some(0));
        assert_eq!(late.take(t, 1), None);
    }
}
