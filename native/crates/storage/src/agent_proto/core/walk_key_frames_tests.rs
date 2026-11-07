//! The frames of the keyed walk and of the attribute listing and status
//! (`walk-keys-v1`, `meta-list-v1`) travel unchanged; known keys must be
//! whole 16-byte prefixes within their bound.
use crate::agent_proto::Frame;

fn roundtrip(frame: Frame) -> Frame {
    let bytes = frame.encode(11).unwrap();
    assert_eq!(bytes.len(), frame.wire_len().unwrap());
    let (id, decoded) = Frame::decode(&bytes).unwrap();
    assert_eq!(id, 11);
    decoded
}

#[test]
fn abgleich_task_walk_key_frames_refuse_partial_known_keys() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_walk_key_frames_refuse_partial_known_keys",
            || {
                let frame = Frame::WalkKeyed {
                    root: "/".into(),
                    algorithm: 0,
                    min_bytes: 0,
                    known: vec![1; 17],
                    keys_only: false,
                    scope: Default::default(),
                };
                assert!(frame.encode(1).is_err());
                let mut bytes = Frame::WalkKeyed {
                    root: "/".into(),
                    algorithm: 0,
                    min_bytes: 0,
                    known: vec![1; 16],
                    keys_only: false,
                    scope: Default::default(),
                }
                .encode(1)
                .unwrap();
                // A mark byte other than 0/1 is refused.
                let mark = Frame::FolderKey {
                    rel: "r".into(),
                    key: [0; 32],
                    same: false,
                    tally: None,
                };
                let mut encoded = mark.encode(2).unwrap();
                *encoded.last_mut().unwrap() = 7;
                assert!(Frame::decode(&encoded).is_err());
                // Cutting the option byte and one key byte leaves a frame that cannot
                // decode.
                bytes.truncate(bytes.len() - 2);
                assert!(Frame::decode(&bytes).is_err());
            },
        ),
        (
            "abgleich_task_folder_key_tally_must_be_whole_and_in_range",
            || {
                let tally = crate::agent_proto::KeyTally {
                    files: 1,
                    bytes: 1 << 40,
                    omitted: 0,
                    untimed: 0,
                    height: 3,
                };
                let frame = Frame::FolderKey {
                    rel: "a".into(),
                    key: [2; 32],
                    same: false,
                    tally: Some(tally),
                };
                let mut bytes = frame.encode(5).unwrap();
                assert_eq!(bytes.len(), frame.wire_len().unwrap());
                bytes.pop();
                assert!(Frame::decode(&bytes).is_err(), "a cut tally cannot decode");
                // The largest depth survives the varint round trip.
                let deep = Frame::FolderKey {
                    rel: "a".into(),
                    key: [2; 32],
                    same: false,
                    tally: Some(crate::agent_proto::KeyTally {
                        height: u32::MAX,
                        ..tally
                    }),
                };
                assert_eq!(roundtrip(deep.clone()), deep);
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}
