use super::*;
use crate::screen::core::wire::WINDOWS;

/// Round 10 (Codex 10/10): a large control frame sent in parts is put
/// together whole; a part out of order or beyond its frame is refused.
#[test]
fn review1119_task_control_parts_reassemble() {
    let payload: Vec<u8> = (0..40_000u32).map(|n| n as u8).collect();
    let mut assembler = PartAssembler::default();
    let mut whole = None;
    let mut offset = 0;
    while offset < payload.len() {
        let len = 16_384.min(payload.len() - offset);
        let part = encode_part(WINDOWS, &payload, offset, len);
        assert_eq!(part.len(), PART_HEAD + len);
        let done = assembler.push(&part).unwrap();
        offset += len;
        assert_eq!(done.is_some(), offset == payload.len());
        whole = done.or(whole);
    }
    assert_eq!(whole, Some((WINDOWS, payload.clone())));
    // A part that does not start at the frame's next byte.
    let mut fresh = PartAssembler::default();
    assert!(fresh
        .push(&encode_part(WINDOWS, &payload, 100, 10))
        .is_err());
    let mut skipped = PartAssembler::default();
    skipped
        .push(&encode_part(WINDOWS, &payload, 0, 10))
        .unwrap();
    assert!(skipped
        .push(&encode_part(WINDOWS, &payload, 20, 10))
        .is_err());
    // A part of another frame while one is being put together.
    let mut mixed = PartAssembler::default();
    mixed.push(&encode_part(WINDOWS, &payload, 0, 10)).unwrap();
    assert!(mixed
        .push(&encode_part(
            crate::screen::core::wire::FAMILIES,
            &payload,
            10,
            10
        ))
        .is_err());
    // Too short, and a part of a part.
    assert!(PartAssembler::default().push(&[WINDOWS, 0, 0]).is_err());
    assert!(PartAssembler::default()
        .push(&encode_part(CONTROL_PART, &payload[..10], 0, 10))
        .is_err());
}
