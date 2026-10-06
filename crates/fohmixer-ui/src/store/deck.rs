//! The Stream Deck on the page (#52, spec §6), pure: what the hub said of
//! it (`deck`: Companion online, the grid, the tab's title), the size of a
//! square key in the measured area, and the page's presses waiting for their
//! `deck_ack` (a failed down flashes its key; nothing is retried, nothing is
//! sent again after a reconnect). `store/live/deck.rs` carries it out.

use std::collections::BTreeMap;

/// The Stream Deck as the hub's latest `deck` message described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckInfo {
    /// Companion's link is up (false also while the page's own socket is
    /// down: the keys dim, the tab shows its red dot).
    pub online: bool,
    pub columns: u32,
    pub rows: u32,
    pub title: String,
}

/// The side (px, whole) of the largest square key that fits `columns` ×
/// `rows` keys with `gap` between them into `width` × `height`.
pub fn key_side(width: f64, height: f64, columns: u32, rows: u32, gap: f64) -> f64 {
    let per =
        |room: f64, n: u32| (room - gap * f64::from(n.saturating_sub(1))) / f64::from(n.max(1));
    per(width, columns).min(per(height, rows)).floor().max(0.0)
}

/// The page's presses waiting for their `deck_ack` (only downs: an up's
/// failure shows nothing), by the page's own press numbers (one counter per
/// kind of id: never `set`'s).
#[derive(Debug)]
pub struct DeckWaiting<F> {
    seq: u64,
    waiting: BTreeMap<u64, F>,
}

impl<F> Default for DeckWaiting<F> {
    fn default() -> Self {
        Self {
            seq: 0,
            waiting: BTreeMap::new(),
        }
    }
}

impl<F> DeckWaiting<F> {
    /// The next press's number.
    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Press `seq` went onto the socket: a down keeps `on_fail` until its ack.
    pub fn sent(&mut self, seq: u64, down: bool, on_fail: F) {
        if down {
            self.waiting.insert(seq, on_fail);
        }
    }

    /// The ack of `seq`: the failure handler of a down that failed.
    pub fn ack(&mut self, seq: u64, ok: bool) -> Option<F> {
        let on_fail = self.waiting.remove(&seq)?;
        (!ok).then_some(on_fail)
    }

    /// The socket closed: no ack will come.
    pub fn clear(&mut self) {
        self.waiting.clear();
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_as_large_as_the_area_allows_and_square() {
        // The FOH iPad's area (1194 × 834 minus the bar, the rail and the
        // padding): the width decides.
        assert_eq!(key_side(1084.0, 758.0, 8, 4, 8.0), 128.0);
        // The height decides.
        assert_eq!(key_side(1000.0, 200.0, 4, 2, 10.0), 95.0);
        // Three columns: the gaps count twice (with `/` for `*`, 31).
        assert_eq!(key_side(100.0, 1000.0, 3, 1, 10.0), 26.0);
        // Three rows likewise.
        assert_eq!(key_side(1000.0, 100.0, 1, 3, 10.0), 26.0);
        // One key fills the area.
        assert_eq!(key_side(100.0, 120.0, 1, 1, 8.0), 100.0);
        // Too small an area, or no grid yet: no key, never a negative size.
        assert_eq!(key_side(10.0, 10.0, 8, 4, 8.0), 0.0);
        assert_eq!(key_side(100.0, 100.0, 0, 0, 8.0), 100.0);
    }

    #[test]
    fn presses_have_their_own_numbers() {
        let mut waiting = DeckWaiting::<&str>::default();
        assert_eq!(
            (waiting.next_seq(), waiting.next_seq(), waiting.next_seq()),
            (1, 2, 3)
        );
    }

    #[test]
    fn a_failed_down_flashes_and_nothing_else_does() {
        let mut waiting = DeckWaiting::<&str>::default();
        waiting.sent(1, true, "flash 1");
        waiting.sent(2, false, "never kept");
        waiting.sent(3, true, "flash 3");
        assert_eq!(waiting.waiting(), 2, "only downs wait");
        assert_eq!(waiting.ack(1, true), None, "an ok down");
        assert_eq!(waiting.ack(1, false), None, "answered once");
        assert_eq!(waiting.ack(2, false), None, "an up never flashes");
        assert_eq!(waiting.ack(3, false), Some("flash 3"));
        assert_eq!(waiting.ack(9, false), None, "a seq it never sent");
        waiting.sent(4, true, "flash 4");
        waiting.clear();
        assert_eq!(waiting.waiting(), 0);
        assert_eq!(waiting.ack(4, false), None, "a closed socket forgot it");
    }
}
