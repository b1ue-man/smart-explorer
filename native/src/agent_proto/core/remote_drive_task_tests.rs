use super::{Frame, WireMeta};

#[test]
fn remote_drive_task_write_new_codec_roundtrip_keeps_exclusive_opcode() {
    let request_id = 0x1020_3040_5060_7080;
    let frame = Frame::WriteNew("/vault/.note.md.smart-explorer-stage".into());

    let encoded = frame.encode(request_id).unwrap();
    assert_eq!(&encoded[..8], &request_id.to_le_bytes());
    assert_eq!(
        encoded[8], 33,
        "WriteNew must retain its dedicated wire tag"
    );

    let (decoded_id, decoded_frame) = Frame::decode(&encoded).unwrap();
    assert_eq!(decoded_id, request_id);
    assert_eq!(decoded_frame, frame);
}

#[test]
fn remote_drive_task_directory_frames_enforce_fifty_thousand_entry_boundary() {
    {
        let maximum = Frame::Dir(vec![WireMeta::default(); 50_000]);
        let encoded = maximum.encode(6).unwrap();
        let (decoded_id, decoded) = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded_id, 6);
        let Frame::Dir(entries) = decoded else {
            panic!("directory frame decoded as another variant");
        };
        assert_eq!(entries.len(), 50_000);
    }

    let oversized = Frame::Dir(vec![WireMeta::default(); 50_001]);
    let encode_error = oversized.encode(7).unwrap_err();
    assert_eq!(encode_error.kind(), std::io::ErrorKind::InvalidData);

    let mut hostile_header = Vec::new();
    hostile_header.extend_from_slice(&7_u64.to_le_bytes());
    hostile_header.push(4);
    hostile_header.extend_from_slice(&50_001_u32.to_le_bytes());
    let decode_error = Frame::decode(&hostile_header).unwrap_err();
    assert_eq!(decode_error.kind(), std::io::ErrorKind::InvalidData);
}
