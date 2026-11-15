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
