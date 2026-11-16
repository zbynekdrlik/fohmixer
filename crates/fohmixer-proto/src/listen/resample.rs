//! 96 kHz to 48 kHz (#210, the main session's PC probe: the master Live runs
//! at 96 kHz). Opus takes 48 kHz at most, so a device streaming at 96 kHz
//! is low-pass filtered and decimated 2:1 on the hub: a [`TAPS`]-tap
//! windowed-sinc FIR (a Blackman window, cut-off [`CUTOFF_HZ`] at
//! [`HIGH_RATE`], unity gain at DC), evaluated only at every second input
//! sample, per channel, its history kept across a stream's matrices.
//! Dropping every second sample instead would fold everything above 24 kHz
//! back into the audio band.
//!
//! Pure: no I/O.

use super::{CHANNELS, SAMPLE_RATE};

/// The device rate decimated 2:1.
pub const HIGH_RATE: u32 = 96_000;
/// The filter's length: odd, so its delay is a whole number of samples
/// ((`TAPS` − 1) / 2 = 47 input samples, 0.49 ms).
pub const TAPS: usize = 95;
/// The low-pass's cut-off (the −6 dB point): the passband is flat to about
/// 18 kHz, and from about 24 kHz on the stopband holds 70 dB or more.
pub const CUTOFF_HZ: f64 = 21_000.0;

/// The rates a device may stream at, and how many of its samples make one
/// at [`SAMPLE_RATE`]: 1 at 48 kHz, 2 at 96 kHz; none for any other.
pub fn decimation(rate: i32) -> Option<usize> {
    match u32::try_from(rate).ok()? {
        SAMPLE_RATE => Some(1),
        HIGH_RATE => Some(2),
        _ => None,
    }
}

/// The filter's coefficients: `sin(2π fc (n − M)) / (π (n − M))` (the
/// centre `2 fc`) times the Blackman window. Their sum (the gain at DC) is
/// 1 within 1e-6 as they stand (tested), so they need no normalising.
pub fn taps() -> Vec<f32> {
    let m = (TAPS - 1) as f64 / 2.0;
    let fc = CUTOFF_HZ / f64::from(HIGH_RATE);
    let last = (TAPS - 1) as f64;
    (0..TAPS)
        .map(|n| {
            let n = n as f64;
            let x = n - m;
            let sinc = if x == 0.0 {
                2.0 * fc
            } else {
                (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n / last).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * n / last).cos();
            (sinc * w) as f32
        })
        .collect()
}

/// One stream's 2:1 decimator of interleaved stereo samples.
#[derive(Debug, Clone)]
pub struct Decimator {
    taps: Vec<f32>,
    /// The last input frames (interleaved stereo), the oldest first: the
    /// filter's history (zeros at the start) and what came since.
    history: Vec<f32>,
    /// The frame in `history` the next output ends at.
    next: usize,
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new()
    }
}

impl Decimator {
    pub fn new() -> Self {
        Self {
            taps: taps(),
            history: vec![0.0; (TAPS - 1) * CHANNELS],
            next: TAPS - 1,
        }
    }

    /// Takes interleaved stereo samples at 96 kHz (a whole number of
    /// frames; a lone last sample is the caller's error and is dropped)
    /// and gives the 48 kHz ones now due: one for each two input frames,
    /// the first at the stream's first sample.
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let whole = input.len() / CHANNELS * CHANNELS;
        self.history.extend_from_slice(&input[..whole]);
        let frames = self.history.len() / CHANNELS;
        let mut out = Vec::with_capacity(whole / 2 + CHANNELS);
        let mut next = self.next;
        for end in (self.next..frames).step_by(2) {
            for channel in 0..CHANNELS {
                let acc: f32 = self
                    .taps
                    .iter()
                    .enumerate()
                    .map(|(k, h)| h * self.history[(end - k) * CHANNELS + channel])
                    .sum();
                out.push(acc);
            }
            next = end + 2;
        }
        // The next output ends at `next` (at or past the last frame): keep
        // the TAPS − 1 frames before the last one, all it can reach back to.
        let drop = frames - (TAPS - 1);
        self.history.drain(..drop * CHANNELS);
        self.next = next - drop;
        out
    }

    /// The input frames it holds between pushes: the filter's reach back,
    /// never more (a long stream keeps no growing history).
    pub fn held(&self) -> usize {
        self.history.len() / CHANNELS
    }
}

#[cfg(test)]
mod tests;
