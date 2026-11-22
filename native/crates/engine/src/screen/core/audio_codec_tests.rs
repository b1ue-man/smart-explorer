use super::*;

/// One 5 ms frame of a 1 kHz tone at half scale, starting at `frame`.
fn tone(frame: usize) -> Vec<f32> {
    (0..FRAME_SAMPLES)
        .flat_map(|index| {
            let t = (frame * FRAME_SAMPLES + index) as f32 / SAMPLE_RATE as f32;
            let value = 0.5 * (2.0 * std::f32::consts::PI * 1_000.0 * t).sin();
            [value, value]
        })
        .collect()
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32
}

#[test]
fn audio_task_codec_opus_frames_are_small_and_decode_to_the_tone() {
    let mut encoder = AudioEncoder::new(AudioCodec::Opus).unwrap();
    let mut decoder = AudioDecoder::new(AudioCodec::Opus).unwrap();
    assert_eq!(encoder.codec(), AudioCodec::Opus);
    assert_eq!(decoder.codec(), AudioCodec::Opus);
    let mut packet = Vec::new();
    let mut out = vec![0.0; FRAME_VALUES];
    let mut decoded = Vec::new();
    for frame in 0..40 {
        encoder.encode(&tone(frame), &mut packet).unwrap();
        // 128 kbit/s for 5 ms is 80 bytes; VBR stays well inside a datagram.
        assert!(
            !packet.is_empty() && packet.len() <= MAX_PAYLOAD,
            "{}",
            packet.len()
        );
        decoder.decode(Some(packet.as_slice()), &mut out).unwrap();
        decoded.extend_from_slice(&out);
    }
    // After the codec settled the tone's energy comes back (0.125 for a
    // sine of amplitude 0.5), within a codec's error.
    let settled = energy(&decoded[decoded.len() / 2..]);
    assert!((0.08..0.17).contains(&settled), "{settled}");
    // A lost frame is concealed, not an error, and fills the frame.
    out.fill(9.0);
    decoder.decode(None, &mut out).unwrap();
    assert!(out.iter().all(|value| value.abs() <= 2.0));
    // Wrong lengths are refused.
    assert!(encoder.encode(&[0.0; 10], &mut packet).is_err());
    assert!(decoder
        .decode(Some(packet.as_slice()), &mut [0.0; 10])
        .is_err());
}

#[test]
fn audio_task_codec_pcm16_round_trips_and_conceals_with_silence() {
    let mut encoder = AudioEncoder::new(AudioCodec::Pcm16).unwrap();
    let mut decoder = AudioDecoder::new(AudioCodec::Pcm16).unwrap();
    let frame = tone(3);
    let mut packet = Vec::new();
    encoder.encode(&frame, &mut packet).unwrap();
    assert_eq!(packet.len(), FRAME_VALUES * 2);
    assert!(packet.len() <= MAX_PAYLOAD);
    let mut out = vec![0.0; FRAME_VALUES];
    decoder.decode(Some(packet.as_slice()), &mut out).unwrap();
    for (value, original) in out.iter().zip(&frame) {
        assert!((value - original).abs() <= 1.0 / 32_768.0);
    }
    decoder.decode(None, &mut out).unwrap();
    assert!(out.iter().all(|value| *value == 0.0));
    decoder.decode(Some(&[1u8, 2, 3][..]), &mut out).unwrap();
    assert!(out.iter().all(|value| *value == 0.0));
    assert!(AudioEncoder::new(AudioCodec::Other).is_err());
    assert!(AudioDecoder::new(AudioCodec::Other).is_err());
}

/// Codex 7/7: under a low „Höchste Datenrate“ the sound, heads included,
/// leaves the pictures their floor; without a ceiling it keeps its rate.
#[test]
fn review1119_task_sound_follows_the_ceiling() {
    assert_eq!(opus_bitrate_for(None), OPUS_BITRATE);
    assert_eq!(opus_bitrate_for(Some(10_000_000)), OPUS_BITRATE);
    let head_bps = (super::super::audio_wire::PACKET_HEAD * 8 * 200) as u64;
    for ceiling in [150_000u64, 200_000, 300_000] {
        let opus = opus_bitrate_for(Some(ceiling));
        let sound = u64::try_from(opus).unwrap() + head_bps;
        let floor = super::super::rate_control::picture_floor_bps(ceiling);
        assert!(opus < OPUS_BITRATE, "{ceiling}: {opus}");
        assert!(
            ceiling - sound >= floor,
            "{ceiling}: sound {sound}, floor {floor}"
        );
    }
    assert_eq!(opus_bitrate_for(Some(1)), OPUS_MIN_BITRATE);
    // Round 9: the picture goes first. Sound goes only where it leaves the
    // pictures their floor (28.4 kbit/s of sound and heads, 56.8 kbit/s in
    // all); below, none is sent.
    assert_eq!(sound_min_bps(), 28_400);
    assert!(sound_fits(None));
    assert!(sound_fits(Some(56_800)));
    assert!(!sound_fits(Some(56_798)));
    assert!(!sound_fits(Some(40_000)));
    for ceiling in [56_800u64, 80_000, 150_000, 300_000, 1_000_000] {
        let sound = u64::try_from(opus_bitrate_for(Some(ceiling))).unwrap() + head_bps;
        let floor = super::super::rate_control::picture_floor_bps(ceiling);
        assert!(sound_fits(Some(ceiling)));
        assert!(
            ceiling - sound >= floor,
            "{ceiling}: sound {sound}, floor {floor}"
        );
    }
    let mut encoder = AudioEncoder::new(AudioCodec::Opus).unwrap();
    encoder.set_bitrate(opus_bitrate_for(Some(150_000)));
    let mut payload = Vec::new();
    encoder.encode(&tone(0), &mut payload).unwrap();
    assert!(!payload.is_empty());
    // Round 8 (Q2): one second (200 frames of 5 ms) under a ceiling of
    // 150 kbit/s stays at its Opus rate (constant below `OPUS_CBR_BELOW`;
    // the CBR rounding adds at most half a byte a frame, 800 bit/s).
    let opus = opus_bitrate_for(Some(150_000));
    let mut bytes = 0usize;
    for frame in 1..=200 {
        encoder.encode(&tone(frame), &mut payload).unwrap();
        bytes += payload.len();
    }
    let sent = u64::try_from(bytes * 8).unwrap();
    let rate = u64::try_from(opus).unwrap();
    assert!(sent <= rate + 800, "{sent} bit/s for {rate} bit/s");
}
