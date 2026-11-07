use super::codec::MAX_FRAME;
use super::{read_frame, write_frame, Frame, SearchSpec, WireMeta, WireNode};
use std::io::{self, Cursor, ErrorKind, Write};

const REQUEST: u64 = 0x1020_3040_5060_7080;

/// Counts the calls of a writer that accepts every byte.
#[derive(Default)]
struct ProbeWriter {
    calls: usize,
    flushes: usize,
}

impl Write for ProbeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

fn assert_error<T>(result: io::Result<T>, expected: ErrorKind) {
    // Never format an unexpectedly successful multi-megabyte payload.
    assert_eq!(result.err().expect("operation must fail").kind(), expected);
}

#[test]
fn frame_roundtrip() {
    let tree = WireNode {
        name: "r".into(),
        size: 500,
        is_dir: true,
        children: vec![WireNode {
            name: "a".into(),
            size: 100,
            is_dir: false,
            children: vec![],
        }],
    };
    let frames = [
        Frame::Hello { proto: 8 },
        Frame::HelloOk {
            proto: 2,
            version: "0.1".into(),
        },
        Frame::ListDir("/a/b".into()),
        Frame::Dir(vec![WireMeta {
            name: "f".into(),
            is_dir: false,
            is_symlink: false,
            special: false,
            size: 9,
            mtime_ms: 1,
            content_md5: Some("0123456789abcdef0123456789abcdef".into()),
        }]),
        Frame::Stat("/x".into()),
        Frame::Meta(WireMeta {
            name: "d".into(),
            is_dir: true,
            is_symlink: false,
            special: false,
            size: 0,
            mtime_ms: 0,
            content_md5: None,
        }),
        Frame::TryExists("/present".into()),
        Frame::Exists(true),
        Frame::WalkTree("/".into()),
        Frame::Tree(tree),
        Frame::Read {
            path: "/f".into(),
            offset: 10,
            len: 0,
        },
        Frame::Write("/f".into()),
        Frame::Data(vec![1, 2, 3, 4]),
        Frame::Copy {
            src: "/a".into(),
            dst: "/b".into(),
        },
        Frame::Rename {
            src: "/a".into(),
            dst: "/b".into(),
        },
        Frame::RenameNoReplace {
            src: "/a".into(),
            dst: "/b".into(),
        },
        Frame::Promote {
            staged: "/a.stage".into(),
            destination: "/a".into(),
        },
        Frame::PromoteNoReplace {
            staged: "/b.stage".into(),
            destination: "/b".into(),
        },
        Frame::Remove {
            path: "/x".into(),
            recursive: true,
        },
        Frame::Mkdir("/d".into()),
        Frame::GetTree("/r".into()),
        Frame::PutTree("/r".into()),
        Frame::TreeEntry {
            rel: "a/b".into(),
            is_dir: false,
            size: 7,
            mtime_ms: 3,
        },
        Frame::Search {
            root: "/r".into(),
            spec: SearchSpec {
                query: "x".into(),
                glob: true,
                min_size: 1,
                max_size: 9,
                max_results: 5,
                want_dirs: true,
            },
        },
        Frame::Match {
            rel: "a".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
        },
        Frame::WalkHashed {
            root: "/r".into(),
            want_hash: true,
        },
        Frame::HashEntry {
            rel: "a".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
            md5: Some("abc".into()),
        },
        Frame::Progress { done: 3, total: 9 },
        Frame::Ok,
        Frame::End,
        Frame::Err("nope".into()),
        Frame::Cancel,
    ];
    for f in frames {
        let (id, got) = Frame::decode(&f.encode(42).unwrap()).unwrap();
        assert_eq!(id, 42);
        assert_eq!(got, f);
    }
}

#[test]
fn oversized_frame_is_rejected_before_body_read() {
    let cases: [(&str, fn()); 3] = [
        ("oversized_frame_is_rejected_before_body_read", || {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&(64 * 1024 * 1024 + 1u32).to_le_bytes());
            let err = super::read_frame(&mut Cursor::new(bytes)).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidData);
        }),
        (
            "outbound_and_inbound_frames_share_the_same_size_limit",
            || {
                let err =
                    super::codec::validate_frame_len(super::codec::MAX_FRAME + 1).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::InvalidData);
                assert!(super::codec::validate_frame_len(super::codec::MAX_FRAME).is_ok());
            },
        ),
        (
            "mount_vault_task_exact_64_mib_body_and_one_byte_over",
            || {
                assert_eq!(MAX_FRAME, 64 * 1024 * 1024);
                // Only the input and one encoded/decoded copy coexist. Do not clone the
                // giant frame, construct a transport copy, or print its contents on failure.
                let frame = Frame::Dir(vec![WireMeta {
                    name: "x".repeat(MAX_FRAME - 13 - 23),
                    ..WireMeta::default()
                }]);
                let encoded = frame.encode(REQUEST).unwrap();
                assert_eq!(encoded.len(), MAX_FRAME);
                drop(frame);
                let (request, mut decoded) = Frame::decode(&encoded).unwrap();
                assert_eq!(request, REQUEST);
                drop(encoded);
                let Frame::Dir(entries) = &mut decoded else {
                    panic!("expected directory");
                };
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name.len(), MAX_FRAME - 36);
                assert!(entries[0].name.bytes().all(|byte| byte == b'x'));
                entries[0].name.reserve_exact(1);
                entries[0].name.push('x');
                assert_error(decoded.encode(REQUEST), ErrorKind::InvalidData);
                let mut writer = ProbeWriter::default();
                assert_error(
                    write_frame(&mut writer, REQUEST, &decoded),
                    ErrorKind::InvalidData,
                );
                assert_eq!((writer.calls, writer.flushes), (0, 0));
                drop(decoded);
                // The receive path must reject the oversized length before reading a body.
                let prefix = u32::try_from(MAX_FRAME + 1).unwrap().to_le_bytes();
                let mut reader = Cursor::new(prefix);
                assert_error(read_frame(&mut reader), ErrorKind::InvalidData);
                assert_eq!(reader.position(), 4);
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn tree_entry_escape_is_rejected_on_encode_and_decode_boundaries() {
    let frame = Frame::TreeEntry {
        rel: "../escape.txt".into(),
        is_dir: false,
        size: 1,
        mtime_ms: 0,
    };
    let mut output = Vec::new();
    let error = super::write_frame(&mut output, 1, &frame).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);

    let error = frame.encode(1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[test]
fn deeply_nested_tree_is_rejected_iteratively() {
    let cases: [(&str, fn()); 3] = [
        ("deeply_nested_tree_is_rejected_iteratively", || {
            let mut body = Vec::new();
            body.extend_from_slice(&1u64.to_le_bytes());
            body.push(8);
            for _ in 0..514 {
                body.extend_from_slice(&1u32.to_le_bytes());
                body.push(b'x');
                body.extend_from_slice(&0u64.to_le_bytes());
                body.push(1);
                body.extend_from_slice(&1u32.to_le_bytes());
            }
            body.extend_from_slice(&1u32.to_le_bytes());
            body.push(b'x');
            body.extend_from_slice(&0u64.to_le_bytes());
            body.push(0);
            body.extend_from_slice(&0u32.to_le_bytes());
            assert_eq!(
                Frame::decode(&body).unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }),
        (
            "data_frames_are_limited_to_one_protocol_chunk_on_both_boundaries",
            || {
                let frame = Frame::Data(vec![0; super::CHUNK + 1]);
                assert_eq!(frame.encode(1).unwrap_err().kind(), ErrorKind::InvalidData);

                let mut body = Vec::new();
                body.extend_from_slice(&1u64.to_le_bytes());
                body.push(11);
                body.extend_from_slice(&((super::CHUNK + 1) as u32).to_le_bytes());
                body.resize(body.len() + super::CHUNK + 1, 0);
                assert_eq!(
                    Frame::decode(&body).unwrap_err().kind(),
                    ErrorKind::InvalidData
                );
            },
        ),
        ("trailing_bytes_are_rejected", || {
            let mut body = Frame::Ok.encode(1).unwrap();
            body.push(0);
            assert_eq!(
                Frame::decode(&body).unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}
