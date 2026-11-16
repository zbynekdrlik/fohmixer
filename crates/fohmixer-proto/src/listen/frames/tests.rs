use super::*;

#[test]
fn stereo_samples_make_whole_frames_and_keep_the_rest() {
    let mut f = Framer::default();
    let first: Vec<f32> = (0..FRAME_SAMPLES + 10).map(|i| i as f32).collect();
    let frames = f.push(2, &first);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0], first[..FRAME_SAMPLES]);
    assert_eq!(f.pending(), 10);
    let second: Vec<f32> = vec![0.5; FRAME_SAMPLES * 2 - 10];
    let frames = f.push(2, &second);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0][..10], first[FRAME_SAMPLES..]);
    assert_eq!(frames[0][10], 0.5);
    assert_eq!(f.pending(), 0);
}

#[test]
fn mono_is_doubled() {
    let mut f = Framer::default();
    let mono: Vec<f32> = (0..FRAME).map(|i| i as f32).collect();
    let frames = f.push(1, &mono);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0][0..4], [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(frames[0][FRAME_SAMPLES - 1], (FRAME - 1) as f32);
    assert_eq!(f.push(1, &[1.0]).len(), 0);
    assert_eq!(f.pending(), 2);
}

#[test]
fn a_value_that_is_no_number_is_silence_and_counted() {
    let mut f = Framer::default();
    let mut s = vec![0.25f32; FRAME_SAMPLES];
    s[0] = f32::NAN;
    s[1] = f32::INFINITY;
    s[2] = 2.0;
    let frames = f.push(2, &s);
    assert_eq!(frames[0][..3], [0.0, 0.0, 2.0]);
    assert_eq!(f.non_finite, 2);
    f.push(1, &[f32::NEG_INFINITY]);
    assert_eq!(f.non_finite, 3);
}

#[test]
fn another_plane_count_takes_nothing() {
    let mut f = Framer::default();
    assert!(f.push(3, &[0.0; FRAME_SAMPLES * 3]).is_empty());
    assert!(f.push(0, &[0.0; 4]).is_empty());
    assert_eq!(f.pending(), 0);
    assert_eq!(FRAME_SAMPLES, 1920);
}

#[test]
fn a_framer_is_made_for_48_and_96_khz_only() {
    assert_eq!(Framer::default().rate(), 48_000);
    assert_eq!(Framer::for_rate(48_000).map(|f| f.rate()), Some(48_000));
    assert_eq!(Framer::for_rate(96_000).map(|f| f.rate()), Some(96_000));
    assert!(Framer::for_rate(44_100).is_none());
    assert!(Framer::for_rate(0).is_none());
    // At 48 kHz a framer passes the samples as they are.
    let mut f = Framer::for_rate(48_000).unwrap();
    let s: Vec<f32> = (0..FRAME_SAMPLES).map(|i| i as f32).collect();
    assert_eq!(f.push(2, &s), vec![s]);
}

fn tone96(n: usize) -> f32 {
    (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / 96_000.0).sin()
}

#[test]
fn two_10_ms_frames_at_96_khz_make_one_20_ms_frame() {
    let mut f = Framer::for_rate(96_000).unwrap();
    // One device frame (960 at 96 kHz, 10 ms): half a frame at 48 kHz.
    let first: Vec<f32> = (0..FRAME)
        .flat_map(|n| [0.5 * tone96(n), 0.25 * tone96(n)])
        .collect();
    assert!(f.push(2, &first).is_empty());
    assert_eq!(f.pending(), FRAME_SAMPLES / 2);
    let second: Vec<f32> = (FRAME..2 * FRAME)
        .flat_map(|n| [0.5 * tone96(n), 0.25 * tone96(n)])
        .collect();
    let frames = f.push(2, &second);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].len(), FRAME_SAMPLES);
    assert_eq!(f.pending(), 0);
    // The 48 kHz frame carries the tone, delayed by the filter.
    for m in 100..FRAME {
        let t = (2 * m) as f64 - 47.0;
        let want = (2.0 * std::f64::consts::PI * 1000.0 * t / 96_000.0).sin();
        assert!(
            (f64::from(frames[0][2 * m]) - 0.5 * want).abs() < 2e-3,
            "{m}"
        );
        assert!(
            (f64::from(frames[0][2 * m + 1]) - 0.25 * want).abs() < 2e-3,
            "{m}"
        );
    }
}

#[test]
fn a_mono_device_at_96_khz_is_doubled_then_decimated() {
    let mut f = Framer::for_rate(96_000).unwrap();
    let mono: Vec<f32> = (0..2 * FRAME).map(tone96).collect();
    let frames = f.push(1, &mono);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0][400], frames[0][401]);
    assert!(frames[0][400].abs() > 0.1);
}
