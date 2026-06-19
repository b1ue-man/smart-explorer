use super::{Frame, SearchSpec, WireMeta, WireNode};

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
        let (id, got) = Frame::decode(&f.encode(42)).unwrap();
        assert_eq!(id, 42);
        assert_eq!(got, f);
    }
}
