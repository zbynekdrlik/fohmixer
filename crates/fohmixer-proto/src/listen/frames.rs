//! 20 ms frames from a device's matrices (#210): the samples of whatever
//! sizes `jit.catch~` hands out, cut into [`FRAME`] stereo frames at
//! [`super::SAMPLE_RATE`] for the Opus encoder. A mono device is doubled to
//! both channels; a value that is no finite number (a NaN, an infinity)
//! becomes silence and is counted. The rest of a matrix waits for the next.
//!
//! Pure: no I/O.

use super::{CHANNELS, FRAME};

/// The samples of one frame (interleaved stereo).
pub const FRAME_SAMPLES: usize = FRAME * CHANNELS;

/// A device stream's frame cutter.
#[derive(Debug, Default)]
pub struct Framer {
    pending: Vec<f32>,
    /// The values made silence since the start.
    pub non_finite: u64,
}

impl Framer {
    /// Takes one matrix's interleaved samples (`planes`: 1 or 2 per cell;
    /// any other count is the caller's refusal, it takes none) and gives
    /// the whole frames now complete, oldest first.
    pub fn push(&mut self, planes: usize, samples: &[f32]) -> Vec<Vec<f32>> {
        match planes {
            1 => {
                for &s in samples {
                    let s = self.finite(s);
                    self.pending.extend_from_slice(&[s, s]);
                }
            }
            2 => {
                for &s in samples {
                    let s = self.finite(s);
                    self.pending.push(s);
                }
            }
            _ => return Vec::new(),
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
