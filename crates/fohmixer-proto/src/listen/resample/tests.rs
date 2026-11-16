use super::*;

/// The filter's gain at `hz` (input at 96 kHz).
fn gain_at(hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / f64::from(HIGH_RATE);
    let (mut re, mut im) = (0.0, 0.0);
    for (n, h) in taps().iter().enumerate() {
        re += f64::from(*h) * (w * n as f64).cos();
        im -= f64::from(*h) * (w * n as f64).sin();
    }
    (re * re + im * im).sqrt()
}

fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

/// `frames` stereo frames of a sine at `hz` sampled at 96 kHz, left
/// `left` and right `right` in amplitude.
fn sine96(hz: f64, frames: usize, left: f32, right: f32) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            let s =
                (2.0 * std::f64::consts::PI * hz * n as f64 / f64::from(HIGH_RATE)).sin() as f32;
            [left * s, right * s]
        })
        .collect()
}

fn rms(samples: impl Iterator<Item = f32>) -> f64 {
    let (sum, n) = samples.fold((0.0, 0usize), |(s, n), x| {
        (s + f64::from(x) * f64::from(x), n + 1)
    });
    (sum / n as f64).sqrt()
}

#[test]
fn only_48_and_96_khz_are_taken() {
    assert_eq!(decimation(48_000), Some(1));
    assert_eq!(decimation(96_000), Some(2));
    assert_eq!(decimation(44_100), None);
    assert_eq!(decimation(88_200), None);
    assert_eq!(decimation(192_000), None);
    assert_eq!(decimation(0), None);
    assert_eq!(decimation(-96_000), None);
    assert_eq!((HIGH_RATE, TAPS, CUTOFF_HZ), (96_000, 95, 21_000.0));
}

#[test]
fn the_filter_is_symmetric_and_unity_at_dc() {
    let h = taps();
    assert_eq!(h.len(), 95);
    for n in 0..TAPS {
        assert!((h[n] - h[TAPS - 1 - n]).abs() < 1e-7, "tap {n}");
    }
    let sum: f64 = h.iter().map(|x| f64::from(*x)).sum();
    assert!((sum - 1.0).abs() < 1e-6, "{sum}");
    // The centre tap: 2 fc before the window and the normalisation.
    assert!((h[47] - 0.4375).abs() < 0.01, "{}", h[47]);
    assert!(h[47] > h[46] && h[46] > 0.0);
    // The window's ends are (nearly) zero.
    assert!(h[0].abs() < 1e-6 && h[94].abs() < 1e-6);
}

#[test]
fn the_passband_is_flat_and_the_stopband_deep() {
    assert!((gain_at(0.0) - 1.0).abs() < 1e-6);
    for hz in [100.0, 1_000.0, 5_000.0, 10_000.0, 15_000.0] {
        assert!(
            db(gain_at(hz)).abs() < 0.05,
            "{hz} Hz: {} dB",
            db(gain_at(hz))
        );
    }
    assert!(
        db(gain_at(18_000.0)).abs() < 0.5,
        "{}",
        db(gain_at(18_000.0))
    );
    assert!(
        (db(gain_at(21_000.0)) + 6.0).abs() < 0.5,
        "{}",
        db(gain_at(21_000.0))
    );
    // From 24 kHz (the new Nyquist) up, nothing folds back.
    for hz in [24_000.0, 26_000.0, 30_000.0, 40_000.0, 47_999.0] {
        assert!(db(gain_at(hz)) < -70.0, "{hz} Hz: {} dB", db(gain_at(hz)));
    }
}

#[test]
fn a_96_khz_sine_comes_out_at_48_khz_with_its_level_and_delay() {
    let mut d = Decimator::new();
    let input = sine96(1_000.0, 9_600, 0.5, 0.25);
    let out = d.push(&input);
    assert_eq!(
        out.len(),
        4_800 * 2,
        "one 48 kHz frame per two input frames"
    );
    // After the filter's delay (47 input samples), each output is the sine
    // at 2m − 47 input samples.
    for m in 100..4_800 {
        let t = (2 * m) as f64 - 47.0;
        let want = (2.0 * std::f64::consts::PI * 1_000.0 * t / 96_000.0).sin();
        assert!(
            (f64::from(out[2 * m]) - 0.5 * want).abs() < 1e-3,
            "left {m}"
        );
        assert!(
            (f64::from(out[2 * m + 1]) - 0.25 * want).abs() < 1e-3,
            "right {m}"
        );
    }
}

#[test]
fn a_tone_above_24_khz_is_gone() {
    let mut d = Decimator::new();
    let out = d.push(&sine96(30_000.0, 9_600, 0.5, 0.5));
    let level = rms(out[200..].iter().copied());
    assert!(level < 0.5 * 1e-3, "{level}");
    let mut d = Decimator::new();
    let pass = d.push(&sine96(10_000.0, 9_600, 0.5, 0.5));
    let level = rms(pass[200..].iter().copied());
    assert!((level - 0.5 / 2f64.sqrt()).abs() < 0.005, "{level}");
}

#[test]
fn pieces_of_any_size_give_what_the_whole_gives() {
    let input = sine96(1_234.0, 3_001, 0.7, -0.3);
    let whole = Decimator::new().push(&input);
    let mut d = Decimator::new();
    let mut pieces = Vec::new();
    let mut at = 0;
    for (i, size) in [1usize, 2, 3, 960, 7, 1_920, 5, 13]
        .iter()
        .cycle()
        .enumerate()
    {
        if at >= input.len() || i > 1_000 {
            break;
        }
        let end = (at + size * 2).min(input.len());
        pieces.extend(d.push(&input[at..end]));
        // It holds the filter's reach back and no more, whatever came.
        assert_eq!(d.held(), TAPS - 1);
        at = end;
    }
    assert_eq!(pieces, whole);
    // 3001 input frames: outputs at frames 0, 2, …, 3000.
    assert_eq!(whole.len(), 1_501 * 2);
    // The first output is the first input frame through the centre-less
    // start of the filter (zeros before it): h[0] times it.
    assert_eq!(whole[0], taps()[0] * input[0]);
}

#[test]
fn a_lone_last_sample_is_dropped() {
    let mut d = Decimator::new();
    assert_eq!(d.push(&[0.5, 0.5, 0.25]).len(), 2);
    assert_eq!(d.push(&[]).len(), 0);
}
