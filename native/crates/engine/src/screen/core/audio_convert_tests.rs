use super::*;

#[test]
fn audio_task_convert_mixes_channels_down_to_stereo() {
    let cases: [(u16, Vec<f32>, [f32; 2]); 4] = [
        (1, vec![0.25], [0.25, 0.25]),
        (2, vec![0.5, -0.5], [0.5, -0.5]),
        // 5.1: FL FR FC LFE BL BR; the LFE is left out.
        (
            6,
            vec![0.1, 0.2, 0.4, 0.9, 0.2, 0.0],
            [0.1 + 0.4 * SURROUND + 0.2 * SURROUND, 0.2 + 0.4 * SURROUND],
        ),
        // 7.1 at full scale clips instead of wrapping.
        (8, vec![1.0; 8], [1.0, 1.0]),
    ];
    for (channels, frame, expected) in cases {
        let mut convert = ToStereo48::new(SAMPLE_RATE, channels).unwrap();
        let mut out = Vec::new();
        // Two frames and a trailing partial one, which is ignored.
        let mut input = frame.repeat(2);
        if channels > 1 {
            input.push(0.3);
        }
        convert.push(&input, &mut out);
        assert_eq!(out.len(), 4, "{channels} channels");
        for (value, wanted) in out.iter().zip(expected.iter().chain(expected.iter())) {
            assert!(
                (value - wanted).abs() < 1e-6,
                "{channels}: {value} {wanted}"
            );
        }
    }
    assert!(ToStereo48::new(0, 2).is_err());
    assert!(ToStereo48::new(48_000, 0).is_err());
    assert!(ToStereo48::new(48_000, MAX_CHANNELS + 1).is_err());
}

#[test]
fn audio_task_convert_resamples_44100_continuously_across_chunks() {
    let mut convert = ToStereo48::new(44_100, 2).unwrap();
    let mut out = Vec::new();
    // One second of a ramp in chunks of 441 frames (10 ms).
    let total = 44_100;
    for chunk in 0..100 {
        let input: Vec<f32> = (0..441)
            .flat_map(|index| {
                let value = (chunk * 441 + index) as f32 / total as f32;
                [value, -value]
            })
            .collect();
        convert.push(&input, &mut out);
    }
    // 48 000 frames per second, give or take the one in flight.
    let frames = out.len() / 2;
    assert!((47_998..=48_001).contains(&frames), "{frames}");
    // The ramp stays a ramp: monotonic, no step at chunk borders.
    let lefts: Vec<f32> = out.iter().step_by(2).copied().collect();
    for pair in lefts.windows(2) {
        let step = pair[1] - pair[0];
        assert!((0.0..1.0 / 44_000.0).contains(&step), "{step}");
    }
    assert!(out
        .chunks_exact(2)
        .all(|frame| (frame[0] + frame[1]).abs() < 1e-6));
}

#[test]
fn audio_task_convert_frames_are_5ms_and_stamped_with_their_last_sample() {
    let mut assembler = FrameAssembler::default();
    assert!(assembler.pop().is_none());
    // 10 ms and a bit arrive at once, the last sample captured at 50 ms.
    let samples = (2 * FRAME_SAMPLES + 48) * 2;
    assembler.push(&vec![0.5; samples], 50_000);
    let (first, first_us) = assembler.pop().unwrap();
    let (second, second_us) = assembler.pop().unwrap();
    assert_eq!(first.len(), FRAME_SAMPLES * 2);
    assert_eq!(second.len(), FRAME_SAMPLES * 2);
    // 48 samples (1 ms) follow the second frame, 288 (6 ms) the first.
    assert_eq!(second_us, 49_000);
    assert_eq!(first_us, 44_000);
    assert!(assembler.pop().is_none());
    // PCM16 round trip within one step, clamped at full scale.
    let mut floats = Vec::new();
    pcm16_to_f32(&[i16::MIN, 0, 16_384], &mut floats);
    assert_eq!(floats, vec![-1.0, 0.0, 0.5]);
    assert_eq!(f32_to_pcm16(0.5), 16_384);
    assert_eq!(f32_to_pcm16(2.0), i16::MAX);
    assert_eq!(f32_to_pcm16(-2.0), i16::MIN);
}
