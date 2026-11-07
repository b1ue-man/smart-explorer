use crate::agent_proto::{Frame, WireHashEntry};

#[test]
fn abgleich_task_walk_meta_frames_refuse_unknown_bits() {
    let part = Frame::HashPart {
        entries: vec![WireHashEntry {
            rel: "f".into(),
            ..WireHashEntry::default()
        }],
        omitted: Vec::new(),
    };
    let mut bytes = part.encode(1).unwrap();
    // Request id (8), tag (1), count (4), rel length (4) and "f": the flags.
    let flags = 8 + 1 + 4 + 4 + 1;
    bytes[flags] = 4; // `hidden` without the attribute flag
    assert!(Frame::decode(&bytes).is_err());
    let mut request = Frame::WalkHashed3 {
        root: "/".into(),
        algorithm: 0,
        min_bytes: 0,
        meta: true,
        scope: Default::default(),
    }
    .encode(1)
    .unwrap();
    // Bits 1 and 2 are the scope's (`walk-filter-v1`); 8 is unknown.
    *request.last_mut().unwrap() = 8;
    assert!(Frame::decode(&request).is_err());
}
