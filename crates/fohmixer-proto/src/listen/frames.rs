//! 20 ms frames from a device's matrices (#210): the samples of whatever
//! sizes `jit.catch~` hands out, cut into [`FRAME`] stereo frames at
//! [`super::SAMPLE_RATE`] for the Opus encoder. A mono device is doubled to
//! both channels; a value that is no finite number (a NaN, an infinity)
//! becomes silence and is counted; a device at 96 kHz is decimated 2:1
//! first (`super::resample`: two of its 10 ms frames make one 20 ms frame).
//! The rest of a matrix waits for the next.
//!
//! Pure: no I/O.

use super::resample::{Decimator, decimation};
use super::{CHANNELS, FRAME, SAMPLE_RATE};

/// The samples of one frame (interleaved stereo).
pub const FRAME_SAMPLES: usize = FRAME * CHANNELS;

/// A device stream's frame cutter.
#[derive(Debug)]
pub struct Framer {
    pending: Vec<f32>,
    /// The values made silence since the start.
    pub non_finite: u64,
    /// The device's rate.
    rate: i32,
    /// At 96 kHz: the 2:1 decimator.
    decimator: Option<Decimator>,
}

impl Default for Framer {
    /// A framer at 48 kHz.
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            non_finite: 0,
            rate: SAMPLE_RATE as i32,
            decimator: None,
        }
    }
}

impl Framer {
    /// A framer for a device at `rate`: none for a rate the hub refuses.
    pub fn for_rate(rate: i32) -> Option<Self> {
        let step = decimation(rate)?;
        Some(Self {
            pending: Vec::new(),
            non_finite: 0,
            rate,
            decimator: (step == 2).then(Decimator::new),
        })
    }

    /// The device rate it frames.
    pub fn rate(&self) -> i32 {
        self.rate
    }

    /// Takes one matrix's interleaved samples (`planes`: 1 or 2 per cell;
    /// any other count is the caller's refusal, it takes none) and gives
    /// the whole frames now complete, oldest first.
    pub fn push(&mut self, planes: usize, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut stereo = Vec::with_capacity(samples.len() * CHANNELS);
        match planes {
            1 => {
                for &s in samples {
                    let s = self.finite(s);
                    stereo.extend_from_slice(&[s, s]);
                }
            }
            2 => {
                for &s in samples {
                    let s = self.finite(s);
                    stereo.push(s);
                }
            }
            _ => return Vec::new(),
        }
        match self.decimator.as_mut() {
            Some(decimator) => self.pending.extend(decimator.push(&stereo)),
            None => self.pending.extend(stereo),
        }
        let whole = self.pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        let frames = self.pending[..whole]
            .as_chunks::<FRAME_SAMPLES>()
            .0
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        self.pending.drain(..whole);
        frames
    }

    /// The samples waiting for the rest of their frame.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    fn finite(&mut self, s: f32) -> f32 {
        if s.is_finite() {
            s
        } else {
            self.non_finite += 1;
            0.0
        }
    }
}

#[cfg(test)]
mod tests;
