use super::*;

#[test]
fn audio_task_packets_round_trip_and_bad_ones_are_refused() {
    let packet = AudioPacket {
        codec: AudioCodec::Opus,
        sequence: u32::MAX,
        host_us: 1_234_567,
        payload: vec![7; 80],
    };
    let bytes = packet.encode().unwrap();
    assert_eq!(bytes.len(), PACKET_HEAD + 80);
    assert!(AudioPacket::is_audio(&bytes));
    assert_eq!(AudioPacket::decode(&bytes).unwrap(), packet);
    // A codec byte of a newer host stays readable and is never played.
    let mut newer = bytes.clone();
    newer[1] = 9;
    assert_eq!(
        AudioPacket::decode(&newer).unwrap().codec,
        AudioCodec::Other
    );
    // Other datagrams, short heads and oversized payloads are refused.
    let mut other = bytes.clone();
    other[0] = 0;
    assert!(!AudioPacket::is_audio(&other));
    assert!(AudioPacket::decode(&other).is_err());
    assert!(AudioPacket::decode(&bytes[..PACKET_HEAD - 1]).is_err());
    let big = AudioPacket {
        payload: vec![0; MAX_PAYLOAD + 1],
        ..packet
    };
    assert!(big.encode().is_err());
    let mut long = bytes;
    long.resize(MAX_PACKET + 1, 0);
    assert!(AudioPacket::decode(&long).is_err());
}

#[test]
fn audio_task_pipe_frames_carry_whole_packets() {
    let mut pipe = Vec::new();
    write_piped(&mut pipe, &[1, 2, 3]).unwrap();
    write_piped(&mut pipe, &[]).unwrap();
    assert!(write_piped(&mut pipe, &vec![0; MAX_PACKET + 1]).is_err());
    let mut reader = pipe.as_slice();
    assert_eq!(read_piped(&mut reader).unwrap(), Some(vec![1, 2, 3]));
    assert_eq!(read_piped(&mut reader).unwrap(), Some(Vec::new()));
    assert_eq!(read_piped(&mut reader).unwrap(), None);
    // A length above the limit is refused before memory is taken.
    let oversized = (MAX_PACKET as u16 + 1).to_be_bytes();
    assert!(read_piped(&mut oversized.as_slice()).is_err());
    // A packet cut off inside is an error, not a clean end.
    let cut = [0u8, 5, 1];
    assert!(read_piped(&mut cut.as_slice()).is_err());
}

#[test]
fn audio_task_host_answers_only_full_sessions_that_offer_sound() {
    let cases = [
        // (offer, mode, metadata only, capture, answered)
        (Some(AUDIO_V1), SessionMode::View, false, true, true),
        (Some(AUDIO_V1), SessionMode::Control, false, true, true),
        (Some(AUDIO_V1), SessionMode::Preview, false, true, false),
        (Some(AUDIO_V1), SessionMode::Probe, false, true, false),
        (Some(AUDIO_V1), SessionMode::View, true, true, false),
        (Some(AUDIO_V1), SessionMode::View, false, false, false),
        (None, SessionMode::View, false, true, false),
        (Some(2), SessionMode::View, false, true, false),
    ];
    for (offered, mode, metadata, capture, answered) in cases {
        let decided = answer(offered, mode, metadata, capture);
        assert_eq!(
            decided.is_some(),
            answered,
            "{offered:?} {mode:?} {metadata} {capture}"
        );
        if let Some(accept) = decided {
            assert!(accept.playable());
            assert_eq!(accept, AudioAccept::opus());
        }
    }
    assert_eq!(offer(SessionMode::View), Some(AUDIO_V1));
    assert_eq!(offer(SessionMode::Control), Some(AUDIO_V1));
    assert_eq!(offer(SessionMode::Preview), None);
    assert_eq!(offer(SessionMode::Probe), None);
}

#[test]
fn audio_task_accept_json_is_additive_and_foreign_formats_are_not_played() {
    let json = serde_json::to_value(AudioAccept::opus()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"version": 1, "codec": "opus", "sample_rate": 48000,
            "channels": 2, "frame_us": 5000})
    );
    let newer: AudioAccept = serde_json::from_value(serde_json::json!({"version": 1,
        "codec": "flac", "sample_rate": 48000, "channels": 2, "frame_us": 5000}))
    .unwrap();
    assert_eq!(newer.codec, AudioCodec::Other);
    assert!(!newer.playable());
    let surround = AudioAccept {
        channels: 6,
        ..AudioAccept::opus()
    };
    assert!(!surround.playable());
}

#[test]
fn audio_task_audio_frame_is_one_byte_both_ways() {
    use super::super::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};
    for on in [true, false] {
        let (tag, payload) = encode_viewer_frame(&ViewerFrame::Audio(on)).unwrap();
        assert_eq!(tag, AUDIO);
        assert_eq!(payload, vec![u8::from(on)]);
        assert_eq!(
            decode_viewer_frame(tag, &payload).unwrap(),
            ViewerFrame::Audio(on)
        );
    }
    // Another value or a longer payload is refused, not read as on.
    assert!(decode_viewer_frame(AUDIO, &[2]).is_err());
    assert!(decode_viewer_frame(AUDIO, &[1, 1]).is_err());
    assert_eq!(super::super::wire::tag_limit(AUDIO), 1);
}
