use super::{Frame, SearchSpec, WireMeta, WireNode};
use std::io::{Cursor, ErrorKind};

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
        Frame::Hello { proto: 7 },
        Frame::HelloOk {
            proto: 2,
            version: "0.1".into(),
        },
        Frame::ListDir("/a/b".into()),
        Frame::Dir(vec![WireMeta {
            name: "f".into(),
            is_dir: false,
            is_symlink: false,
            size: 9,
            mtime_ms: 1,
        }]),
        Frame::Stat("/x".into()),
        Frame::Meta(WireMeta {
            name: "d".into(),
            is_dir: true,
            is_symlink: false,
            size: 0,
            mtime_ms: 0,
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
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(64 * 1024 * 1024 + 1u32).to_le_bytes());
    let err = super::read_frame(&mut Cursor::new(bytes)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn outbound_and_inbound_frames_share_the_same_size_limit() {
    let err = super::codec::validate_frame_len(super::codec::MAX_FRAME + 1).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(super::codec::validate_frame_len(super::codec::MAX_FRAME).is_ok());
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
fn data_frames_are_limited_to_one_protocol_chunk_on_both_boundaries() {
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
}

#[test]
fn deeply_nested_tree_is_rejected_iteratively() {
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
}

#[test]
fn trailing_bytes_are_rejected() {
    let mut body = Frame::Ok.encode(1).unwrap();
    body.push(0);
    assert_eq!(
        Frame::decode(&body).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}
