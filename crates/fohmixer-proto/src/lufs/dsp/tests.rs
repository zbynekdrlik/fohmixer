use super::*;

/// A meter fed a stereo 1 kHz sine, the same on both channels: each part
/// is (seconds, peak level in dBFS per channel, or silence). A stereo
/// 1 kHz sine at −23 dBFS reads −23 LUFS (EBU Tech 3341's calibration).
struct Signal {
    meter: Meter,
    rate: f64,
    n: u64,
    readings: Vec<Readings>,
}

impl Signal {
    fn new(rate: f64) -> Self {
        Self {
            meter: Meter::new(rate),
            rate,
            n: 0,
            readings: Vec::new(),
        }
    }

    fn play(mut self, seconds: f64, dbfs: Option<f64>) -> Self {
        let amplitude = dbfs.map_or(0.0, |db| 10f64.powf(db / 20.0));
        let samples = (seconds * self.rate).round() as u64;
        for _ in 0..samples {
            let x =
                amplitude * (2.0 * std::f64::consts::PI * 1000.0 * self.n as f64 / self.rate).sin();
            self.n += 1;
            if let Some(r) = self.meter.push(x, x) {
                self.readings.push(r);
            }
        }
        self
    }

    fn last(&self) -> Readings {
        *self.readings.last().expect("a reading")
    }
}

fn near(got: f64, want: f64, tol: f64) {
    assert!((got - want).abs() <= tol, "got {got}, want {want} ±{tol}");
}

// --- K-weighting ------------------------------------------------------

#[test]
fn the_derived_48_khz_coefficients_are_the_published_ones() {
    // ITU-R BS.1770-4, tables 1 and 2.
    let pre = pre_filter(48_000.0);
    let want_b = [1.53512485958697, -2.69169618940638, 1.19839281085285];
    let want_a = [-1.69065929318241, 0.73248077421585];
    for (got, want) in pre.b.iter().zip(want_b) {
        near(*got, want, 1e-12);
    }
    for (got, want) in pre.a.iter().zip(want_a) {
        near(*got, want, 1e-12);
    }
    let rlb = rlb_filter(48_000.0);
    assert_eq!(rlb.b, [1.0, -2.0, 1.0]);
    for (got, want) in rlb.a.iter().zip([-1.99004745483398, 0.99007225036621]) {
        near(*got, want, 1e-12);
    }
}

/// A biquad's gain in dB at `f` Hz when run at `rate` Hz.
fn gain_db(q: &Biquad, f: f64, rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / rate;
    let (c1, s1, c2, s2) = (w.cos(), -w.sin(), (2.0 * w).cos(), -(2.0 * w).sin());
    let num = (
        q.b[0] + q.b[1] * c1 + q.b[2] * c2,
        q.b[1] * s1 + q.b[2] * s2,
    );
    let den = (1.0 + q.a[0] * c1 + q.a[1] * c2, q.a[0] * s1 + q.a[1] * s2);
    10.0 * ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).log10()
}

#[test]
fn the_coefficients_follow_the_sample_rate() {
    // The shelf derived at another rate answers as the published 48 kHz one
    // does from the bass to past its corner: within 0.005 dB at 44.1 kHz
    // (measured: 0.0015 at most; a derivation without the bilinear prewarp,
    // k = pi f0 / rate, is 0.018 off at the corner and 0.0096 at 1 kHz there)
    // and 0.01 dB at 96 kHz (measured: 0.006 at 3 kHz).
    let published = Biquad::new(
        [1.53512485958697, -2.69169618940638, 1.19839281085285],
        [-1.69065929318241, 0.73248077421585],
    );
    for (rate, tol) in [(44_100.0, 0.005), (96_000.0, 0.01)] {
        let pre = pre_filter(rate);
        for f in [100.0, 1000.0, SHELF_F0, 3000.0] {
            near(
                gain_db(&pre, f, rate),
                gain_db(&published, f, 48_000.0),
                tol,
            );
        }
        let rlb = rlb_filter(rate);
        // The high-pass passes Nyquist at about 0 dB: (1 + 2 + 1) / (1 - a1 + a2).
        let nyquist = 4.0 / (1.0 - rlb.a[0] + rlb.a[1]);
        near(20.0 * nyquist.log10(), 0.0, 0.06);
    }
    assert_ne!(pre_filter(44_100.0), pre_filter(48_000.0));
    assert_ne!(rlb_filter(48_000.0), rlb_filter(96_000.0));
}

#[test]
fn a_biquad_is_its_difference_equation() {
    // Direct form I written out: y[n] = b0 x[n] + b1 x[n-1] + b2 x[n-2]
    //                                  - a1 y[n-1] - a2 y[n-2].
    let (b, a) = ([0.5, -0.25, 0.125], [-0.3, 0.2]);
    let mut q = Biquad::new(b, a);
    let input = [1.0, 0.0, -0.5, 2.0, 0.0, 0.0, 0.75, -1.0];
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for x in input {
        let want = b[0] * x + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
        near(q.process(x), want, 1e-12);
        (x2, x1, y2, y1) = (x1, x, y1, want);
    }
}

// --- the loudness and the window ---------------------------------------

#[test]
fn loudness_is_bs1770s_formula_within_the_parameters_range() {
    near(loudness(1.0), -0.691, 1e-12);
    near(loudness(0.1), -10.691, 1e-12);
    near(loudness(1e-3), -30.691, 1e-12);
    assert_eq!(loudness(0.0), FLOOR_LUFS);
    assert_eq!(loudness(1e-9), FLOOR_LUFS);
    assert_eq!(loudness(10.0), TOP_LUFS);
    assert_eq!(loudness(2.0), TOP_LUFS);
    near(loudness(1.1), -0.691 + 10.0 * 1.1f64.log10(), 1e-12);
}

#[test]
fn the_voice_gate_is_minus_40_lufs_over_four_sub_blocks() {
    near(loudness(voice_gate_sum() / 4.0), VOICE_GATE_LUFS, 1e-9);
}

#[test]
fn a_sub_block_at_the_gate_is_voiced_one_just_under_is_not() {
    let mut w = Window::new();
    assert_eq!(w.push(voice_gate_sum()).voiced, 1);
    let mut w = Window::new();
    assert_eq!(w.push(voice_gate_sum().next_down()).voiced, 0);
}

#[test]
fn a_sub_block_is_voiced_by_the_400_ms_ending_with_it_for_3_s() {
    // One sub-block at the gate, then silence: it and the three after it
    // (whose 400 ms still hold it) are voiced, the fifth is not; each
    // leaves the window 30 sub-blocks after it came.
    let mut w = Window::new();
    let mut counts = vec![w.push(voice_gate_sum()).voiced];
    counts.extend((0..39).map(|_| w.push(0.0).voiced));
    let want: Vec<usize> = (1..=40usize)
        .map(|n| (1..=4).filter(|&p| p <= n && p + WINDOW_BLOCKS > n).count())
        .collect();
    assert_eq!(counts, want);
}

#[test]
fn voice_needs_ten_voiced_sub_blocks() {
    let m = 1e-2;
    let mut w = Window::new();
    for _ in 0..9 {
        assert_eq!(w.push(m).voice, FLOOR_LUFS);
    }
    let r = w.push(m);
    assert_eq!(r.voiced, MIN_VOICED);
    near(r.voice, loudness(m), 1e-9);
}

#[test]
fn voice_is_the_mean_of_the_voiced_sub_blocks_only() {
    let m = 1e-2;
    let mut w = Window::new();
    for _ in 0..10 {
        w.push(m);
    }
    // Three silent sub-blocks are still voiced (their 400 ms hold voice),
    // the next ones are not.
    let mut r = w.push(0.0);
    for _ in 0..9 {
        r = w.push(0.0);
    }
    assert_eq!(r.voiced, 13);
    near(r.voice, loudness(10.0 * m / 13.0), 1e-9);
    near(r.short_term, loudness(10.0 * m / 30.0), 1e-9);
    assert_eq!(r, w.readings());
}

#[test]
fn a_fresh_window_reads_the_floor() {
    let r = Window::default().readings();
    assert_eq!(
        r,
        Readings {
            short_term: FLOOR_LUFS,
            voice: FLOOR_LUFS,
            voiced: 0
        }
    );
}

// --- the meter: EBU Tech 3341's short-term cases -------------------------

#[test]
fn a_sub_block_is_a_tenth_of_a_second() {
    for (rate, len) in [(44_100.0, 4410), (48_000.0, 4800), (96_000.0, 9600)] {
        let mut m = Meter::new(rate);
        assert_eq!(m.block_len(), len);
        let ends = (1..=2 * len)
            .filter(|_| m.push(0.1, -0.1).is_some())
            .count();
        assert_eq!(ends, 2, "{rate}");
    }
    let mut m = Meter::new(48_000.0);
    assert!((1..4800).all(|_| m.push(0.0, 0.0).is_none()));
    assert!(m.push(0.0, 0.0).is_some());
}

#[test]
fn a_stereo_1_khz_sine_at_minus_23_dbfs_reads_minus_23_at_every_rate() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let s = Signal::new(rate).play(4.0, Some(-23.0));
        assert_eq!(s.readings.len(), 40, "{rate}");
        let r = s.last();
        near(r.short_term, -23.0, 0.1);
        near(r.voice, -23.0, 0.1);
        assert_eq!(r.voiced, WINDOW_BLOCKS);
    }
}

#[test]
fn one_channel_alone_reads_3_lu_less() {
    let mut m = Meter::new(48_000.0);
    let amplitude = 10f64.powf(-23.0 / 20.0);
    let mut last = None;
    for n in 0..4 * 48_000 {
        let x = amplitude * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / 48_000.0).sin();
        last = m.push(x, 0.0).or(last);
    }
    near(last.expect("a reading").short_term, -26.01, 0.1);
}

#[test]
fn a_20_db_step_reads_each_level_once_the_window_holds_it() {
    let s = Signal::new(48_000.0).play(3.0, Some(-33.0));
    near(s.last().short_term, -33.0, 0.1);
    let s = s.play(3.0, Some(-13.0));
    near(s.last().short_term, -13.0, 0.1);
    let s = s.play(3.0, Some(-33.0));
    near(s.last().short_term, -33.0, 0.1);
}

#[test]
fn a_window_half_at_each_level_reads_their_mean_power() {
    // 1.5 s at −36 then 1.5 s at −16 (EBU Tech 3341's step in the window).
    let s = Signal::new(48_000.0)
        .play(3.0, Some(-36.0))
        .play(1.5, Some(-36.0))
        .play(1.5, Some(-16.0));
    let want = 10.0 * ((10f64.powf(-3.6) + 10f64.powf(-1.6)) / 2.0).log10();
    near(s.last().short_term, want, 0.1);
}

#[test]
fn the_first_3_s_count_the_missing_sub_blocks_as_silence() {
    let s = Signal::new(48_000.0).play(1.0, Some(-23.0));
    near(
        s.last().short_term,
        -23.0 + 10.0 * (1.0f64 / 3.0).log10(),
        0.1,
    );
}

// --- the Voice gate (spec §3.3) -----------------------------------------

#[test]
fn after_a_2_s_phrase_and_a_2_s_pause_voice_holds_and_short_term_falls() {
    let phrase = Signal::new(48_000.0).play(2.0, Some(-25.0));
    let at_end = phrase.last();
    near(at_end.voice, -25.0, 0.1);
    let s = phrase.play(2.0, None);
    let r = s.last();
    // The last 1 s of the phrase and the 300 ms its 400 ms still hold.
    assert_eq!(r.voiced, 13);
    near(r.voice, -25.0 + 10.0 * (10.0f64 / 13.0).log10(), 0.1);
    assert!(r.voice > -26.5, "{r:?}");
    near(r.short_term, -25.0 + 10.0 * (1.0f64 / 3.0).log10(), 0.1);
    assert!(r.short_term < -29.0 && r.voice > -27.0, "{r:?}");
    // Short-term never rises through the pause, and falls once the phrase
    // leaves the window (the first 1 s of the pause drops the silence the
    // window started with).
    let pause = &s.readings[20..];
    assert!(
        pause
            .windows(2)
            .all(|w| w[1].short_term <= w[0].short_term + 1e-9)
    );
    assert!(
        pause[10..]
            .windows(2)
            .all(|w| w[1].short_term < w[0].short_term)
    );
}

#[test]
fn half_a_second_of_voice_in_3_s_is_silent() {
    let s = Signal::new(48_000.0)
        .play(1.0, None)
        .play(0.5, Some(-25.0))
        .play(3.0, None);
    assert!(
        s.readings.iter().all(|r| r.voice == FLOOR_LUFS),
        "{:?}",
        s.readings
    );
    assert!(s.readings.iter().any(|r| r.short_term > -40.0));
}

#[test]
fn a_voice_at_minus_34_is_valid_not_silent() {
    let r = Signal::new(48_000.0).play(4.0, Some(-34.0)).last();
    near(r.voice, -34.0, 0.1);
    assert_eq!(r.voiced, WINDOW_BLOCKS);
}

#[test]
fn a_1_s_breath_at_minus_38_alone_is_silent() {
    let s = Signal::new(48_000.0)
        .play(1.0, None)
        .play(1.0, Some(-38.0))
        .play(3.0, None);
    assert!(
        s.readings.iter().all(|r| r.voice == FLOOR_LUFS),
        "{:?}",
        s.readings
    );
    // Its 400 ms crossed the gate: 8 of its sub-blocks and one after it.
    assert_eq!(s.readings.iter().map(|r| r.voiced).max(), Some(9));
}

#[test]
fn silence_reads_the_floor() {
    let r = Signal::new(48_000.0).play(4.0, None).last();
    assert_eq!(r.short_term, FLOOR_LUFS);
    assert_eq!(r.voice, FLOOR_LUFS);
    assert_eq!(r.voiced, 0);
}
