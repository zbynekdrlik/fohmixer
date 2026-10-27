//! The reference DSP of the `FohMixer LUFS` device (#151, spec §3.3): what
//! the device's `gen~` computes, in plain Rust, so CI can test it against
//! EBU Tech 3341's short-term cases and the Voice gate.
//!
//! - **K-weighting** ([`pre_filter`], [`rlb_filter`]): BS.1770-4's two
//!   biquads, their coefficients derived for any sample rate from the
//!   analog prototypes (libebur128's form; the published 48 kHz values are
//!   the test case).
//! - **Sub-blocks:** the mean square of each 100 ms of K-weighted audio,
//!   summed over the two channels (each weight 1.0).
//! - **Short-term** `S`: −0.691 + 10·log10 of the mean of the last 30
//!   sub-blocks (3 s), no gate.
//! - **Voice** `V`: a sub-block is voiced when the momentary loudness of
//!   the 400 ms ending with it is at least −40 LUFS; `V` is the loudness
//!   of the mean of the voiced sub-blocks among the last 30, valid with at
//!   least 10 of them, else the floor.
//!
//! Both readings lie in −70…0 LUFS, the device parameters' range (a
//! reading of no signal is the floor, −70). Before 3 s of audio the
//! missing sub-blocks count as silence, as in the device (its ring buffers
//! start at zero).

use std::collections::VecDeque;

// lufs-table begin
// The one written-out table: `live-device/FohMixer-LUFS/fohmixer_lufs.genexpr`
// holds a copy between the same markers, and `scripts/check_lufs_device.py`
// (CI) fails when the two differ in a name or a value.
/// The high-shelf pre-filter's centre frequency (Hz).
pub const SHELF_F0: f64 = 1681.974450955533;
/// The high-shelf's gain (dB).
pub const SHELF_GAIN_DB: f64 = 3.999843853973347;
/// The high-shelf's Q.
pub const SHELF_Q: f64 = 0.7071752369554196;
/// The exponent of the shelf's band gain (`Vb = Vh^x`, libebur128).
pub const SHELF_VB_EXP: f64 = 0.4996667741545416;
/// The RLB high-pass's corner frequency (Hz).
pub const HIGHPASS_F0: f64 = 38.13547087602444;
/// The RLB high-pass's Q.
pub const HIGHPASS_Q: f64 = 0.5003270373238773;
/// BS.1770's offset: loudness = −0.691 + 10·log10(mean square).
pub const LUFS_OFFSET: f64 = -0.691;
/// Sub-blocks per second (100 ms each, EBU Tech 3341's 10 Hz).
pub const BLOCKS_PER_SECOND: f64 = 10.0;
/// The short-term window in sub-blocks (3 s).
pub const WINDOW_BLOCKS: usize = 30;
/// The momentary window in sub-blocks (400 ms).
pub const MOMENTARY_BLOCKS: usize = 4;
/// A sub-block is voiced when its momentary loudness is at least this.
pub const VOICE_GATE_LUFS: f64 = -40.0;
/// `V` is valid with at least this many voiced sub-blocks in the window.
pub const MIN_VOICED: usize = 10;
/// The lowest reading: no signal, or `V` without enough voice.
pub const FLOOR_LUFS: f64 = -70.0;
/// The highest reading (the parameters' top).
pub const TOP_LUFS: f64 = 0.0;
// lufs-table end

/// One biquad, direct form II transposed, `a0` normalised to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Biquad {
    /// `b0, b1, b2`.
    pub b: [f64; 3],
    /// `a1, a2`.
    pub a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    /// A biquad with these coefficients and a silent state.
    pub fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Self { b, a, z: [0.0; 2] }
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// BS.1770's high-shelf pre-filter at `rate` Hz.
pub fn pre_filter(rate: f64) -> Biquad {
    let k = (std::f64::consts::PI * SHELF_F0 / rate).tan();
    let vh = 10f64.powf(SHELF_GAIN_DB / 20.0);
    let vb = vh.powf(SHELF_VB_EXP);
    let a0 = 1.0 + k / SHELF_Q + k * k;
    Biquad::new(
        [
            (vh + vb * k / SHELF_Q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / SHELF_Q + k * k) / a0,
        ],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / SHELF_Q + k * k) / a0],
    )
}

/// BS.1770's RLB high-pass at `rate` Hz (`b` is 1, −2, 1 as published).
pub fn rlb_filter(rate: f64) -> Biquad {
    let k = (std::f64::consts::PI * HIGHPASS_F0 / rate).tan();
    let a0 = 1.0 + k / HIGHPASS_Q + k * k;
    Biquad::new(
        [1.0, -2.0, 1.0],
        [
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / HIGHPASS_Q + k * k) / a0,
        ],
    )
}

/// The loudness of a mean square (summed over channels), in −70…0 LUFS.
pub fn loudness(mean_square: f64) -> f64 {
    (LUFS_OFFSET + 10.0 * mean_square.log10()).clamp(FLOOR_LUFS, TOP_LUFS)
}

/// The momentary sum (of [`MOMENTARY_BLOCKS`] sub-blocks' mean squares)
/// at which a sub-block is voiced: [`VOICE_GATE_LUFS`] in the power domain.
pub fn voice_gate_sum() -> f64 {
    MOMENTARY_BLOCKS as f64 * 10f64.powf((VOICE_GATE_LUFS - LUFS_OFFSET) / 10.0)
}

/// The two readings after a sub-block (§3.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readings {
    /// `S`: the short-term loudness.
    pub short_term: f64,
    /// `V`: the loudness of the voiced sub-blocks, or the floor.
    pub voice: f64,
    /// The voiced sub-blocks in the window.
    pub voiced: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Block {
    mean_square: f64,
    voiced: bool,
}

/// The last [`WINDOW_BLOCKS`] sub-blocks and the readings they give.
#[derive(Debug, Clone)]
pub struct Window {
    blocks: VecDeque<Block>,
    gate_sum: f64,
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Window {
    /// A window of silence (no voiced sub-block).
    pub fn new() -> Self {
        let silent = Block {
            mean_square: 0.0,
            voiced: false,
        };
        Self {
            blocks: std::iter::repeat_n(silent, WINDOW_BLOCKS).collect(),
            gate_sum: voice_gate_sum(),
        }
    }

    /// Adds a sub-block's mean square (summed over channels) and gives the
    /// readings of the window ending with it.
    pub fn push(&mut self, mean_square: f64) -> Readings {
        self.blocks.pop_front();
        let earlier: f64 = self
            .blocks
            .iter()
            .rev()
            .take(MOMENTARY_BLOCKS - 1)
            .map(|b| b.mean_square)
            .sum();
        self.blocks.push_back(Block {
            mean_square,
            voiced: mean_square + earlier >= self.gate_sum,
        });
        self.readings()
    }

    /// The readings of the window as it stands.
    pub fn readings(&self) -> Readings {
        let total: f64 = self.blocks.iter().map(|b| b.mean_square).sum();
        let voiced: Vec<f64> = self
            .blocks
            .iter()
            .filter(|b| b.voiced)
            .map(|b| b.mean_square)
            .collect();
        let voice = if voiced.len() >= MIN_VOICED {
            loudness(voiced.iter().sum::<f64>() / voiced.len() as f64)
        } else {
            FLOOR_LUFS
        };
        Readings {
            short_term: loudness(total / WINDOW_BLOCKS as f64),
            voice,
            voiced: voiced.len(),
        }
    }
}

/// The whole meter on a stereo input: K-weighting per channel, the
/// sub-blocks, the window.
#[derive(Debug, Clone)]
pub struct Meter {
    filters: [[Biquad; 2]; 2],
    block_len: usize,
    filled: usize,
    sum: f64,
    window: Window,
}

impl Meter {
    /// A silent meter at `rate` Hz (a sub-block is `rate / 10` samples,
    /// rounded).
    pub fn new(rate: f64) -> Self {
        let channel = [pre_filter(rate), rlb_filter(rate)];
        Self {
            filters: [channel.clone(), channel],
            block_len: (rate / BLOCKS_PER_SECOND).round() as usize,
            filled: 0,
            sum: 0.0,
            window: Window::new(),
        }
    }

    /// The samples of one sub-block.
    pub fn block_len(&self) -> usize {
        self.block_len
    }

    /// Takes one stereo sample; at the end of a sub-block gives the new
    /// readings.
    pub fn push(&mut self, left: f64, right: f64) -> Option<Readings> {
        for (filters, x) in self.filters.iter_mut().zip([left, right]) {
            let y = filters[1].process(filters[0].process(x));
            self.sum += y * y;
        }
        self.filled += 1;
        if self.filled == self.block_len {
            let mean_square = self.sum / self.block_len as f64;
            self.filled = 0;
            self.sum = 0.0;
            Some(self.window.push(mean_square))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
