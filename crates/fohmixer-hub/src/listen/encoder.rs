//! The Opus encoder of a device's stream (#210): libopus through the `opus`
//! crate (iemmixer's server's choice), 48 kHz stereo, 20 ms frames,
//! [`BITRATE`] bit/s, the restricted low-delay application (no speech
//! mode's lookahead).

use fohmixer_proto::listen::frames::FRAME_SAMPLES;
use fohmixer_proto::listen::{CHANNELS, SAMPLE_RATE};
use opus::{Application, Bitrate, Channels};

/// The bit rate (iemmixer's listen).
pub const BITRATE: i32 = 128_000;
/// The largest packet asked of libopus (its own recommended bound).
pub const MAX_PACKET: usize = 4000;

/// One stream's encoder.
pub struct Encoder(opus::Encoder);

impl Encoder {
    pub fn new() -> Result<Self, opus::Error> {
        debug_assert_eq!(CHANNELS, 2);
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::LowDelay)?;
        inner.set_bitrate(Bitrate::Bits(BITRATE))?;
        Ok(Self(inner))
    }

    /// One frame's packet (`frame`: [`FRAME_SAMPLES`] interleaved samples).
    pub fn encode(&mut self, frame: &[f32]) -> Result<Vec<u8>, opus::Error> {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES);
        self.0.encode_vec_float(frame, MAX_PACKET)
    }

    /// The bit rate libopus holds (the tests' check).
    #[cfg(test)]
    pub fn bitrate(&mut self) -> Result<Bitrate, opus::Error> {
        self.0.get_bitrate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::listen::FRAME;

    /// A 1 kHz sine at -6 dBFS, left only, for `frames` frames.
    fn sine(frames: usize) -> Vec<f32> {
        (0..frames * FRAME)
            .flat_map(|n| {
                let t = n as f32 / SAMPLE_RATE as f32;
                [0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin(), 0.0]
            })
            .collect()
    }

    #[test]
    fn a_sine_comes_back_through_the_decoder() {
        let mut enc = Encoder::new().unwrap();
        assert_eq!(enc.bitrate().unwrap(), Bitrate::Bits(128_000));
        let mut dec = opus::Decoder::new(SAMPLE_RATE, Channels::Stereo).unwrap();
        let input = sine(50);
        let mut out = Vec::new();
        for frame in input.as_chunks::<FRAME_SAMPLES>().0 {
            let packet = enc.encode(frame).unwrap();
            assert!(!packet.is_empty() && packet.len() <= MAX_PACKET);
            let mut pcm = vec![0f32; FRAME_SAMPLES];
            assert_eq!(dec.decode_float(&packet, &mut pcm, false).unwrap(), FRAME);
            out.extend(pcm);
        }
        // After the codec's start (the last half second): the left channel
        // carries the sine's energy, the right one nearly none.
        let tail = &out[out.len() / 2..];
        let rms = |ch: usize| {
            let v: Vec<f32> = tail.iter().skip(ch).step_by(2).copied().collect();
            (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt()
        };
        let (left, right) = (rms(0), rms(1));
        assert!((left - 0.3536).abs() < 0.05, "left rms {left}");
        assert!(right < 0.02, "right rms {right}");
    }
}
