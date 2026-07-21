use super::Frame;

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
