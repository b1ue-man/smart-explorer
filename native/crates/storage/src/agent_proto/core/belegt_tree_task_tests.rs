//! `tree-allocated-v1`: the agent's size tree with every node's occupied
//! bytes, the old `Tree` frame without them, and the agent's own walk.
use super::{read_frame, write_frame, Frame, ServerFeatures, WireNode};
use std::io::Cursor;

fn node(name: &str, size: u64, allocated: Option<u64>, children: Vec<WireNode>) -> WireNode {
    WireNode {
        name: name.into(),
        size,
        allocated,
        is_dir: !children.is_empty(),
        children,
    }
}

fn roundtrip(frame: &Frame) -> Frame {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, 7, frame).expect("write");
    let (id, back) = read_frame(&mut Cursor::new(bytes))
        .expect("read")
        .expect("frame");
    assert_eq!(id, 7);
    back
}

#[test]
fn belegt_task_agent_tree_frames_carry_allocation_and_old_frames_stay_unknown() {
    let tree = node(
        "root",
        5_000_000_100,
        Some(4096),
        vec![
            node("placeholder", 5_000_000_000, Some(0), Vec::new()),
            node("plain", 100, Some(4096), Vec::new()),
        ],
    );
    match roundtrip(&Frame::TreeAllocated(tree.clone())) {
        Frame::TreeAllocated(back) => assert_eq!(back, tree),
        other => panic!("unexpected {other:?}"),
    }
    match roundtrip(&Frame::WalkTreeAllocated("/srv".into())) {
        Frame::WalkTreeAllocated(path) => assert_eq!(path, "/srv"),
        other => panic!("unexpected {other:?}"),
    }
    // Not known on the walking side: the sentinel decodes back to `None`.
    let unknown = node("f", 3, None, Vec::new());
    match roundtrip(&Frame::TreeAllocated(unknown.clone())) {
        Frame::TreeAllocated(back) => assert_eq!(back, unknown),
        other => panic!("unexpected {other:?}"),
    }
    // The old `Tree` frame never carries the occupancy.
    match roundtrip(&Frame::Tree(tree)) {
        Frame::Tree(back) => {
            assert_eq!(back.allocated, None);
            assert!(back.children.iter().all(|child| child.allocated.is_none()));
        }
        other => panic!("unexpected {other:?}"),
    }
    // Only the agent announces the label; the service and old agents do not.
    assert!(ServerFeatures::parse(&super::server_version(true)).tree_allocated);
    assert!(!ServerFeatures::parse(&super::server_version_with(true, 8)).tree_allocated);
    assert!(!ServerFeatures::parse("0.5.0+credit-v1").tree_allocated);
}

/// The agent's walk (Unix): occupied bytes from `st_blocks`, one hard-link
/// group counted once, folders summing their children.
#[cfg(unix)]
#[test]
fn belegt_task_agent_walk_counts_blocks_and_links_once() {
    use std::sync::atomic::AtomicBool;
    let fixture = tempfile::tempdir().expect("temp dir");
    let base = fixture.path();
    std::fs::write(base.join("x.bin"), vec![1u8; 10_000]).expect("write");
    std::fs::create_dir(base.join("sub")).expect("dir");
    std::fs::hard_link(base.join("x.bin"), base.join("sub/y.bin")).expect("link");
    let sparse = std::fs::File::create(base.join("sparse.img")).expect("create");
    sparse.set_len(16 * 1024 * 1024).expect("extend");
    drop(sparse);
    let counter = super::WalkCounter::new();
    let tree = super::fs::walk_dir_counted(base, "root".into(), &counter, &AtomicBool::new(false))
        .expect("walk");
    let mut files = Vec::new();
    let mut stack = vec![&tree];
    while let Some(node) = stack.pop() {
        let below: Option<u64> = node
            .children
            .iter()
            .map(|child| child.allocated)
            .sum::<Option<u64>>();
        if node.is_dir {
            assert_eq!(node.allocated, below, "{}", node.name);
        } else {
            files.push((node.name.clone(), node.size, node.allocated.expect("known")));
        }
        stack.extend(node.children.iter());
    }
    files.sort();
    let links: Vec<u64> = files
        .iter()
        .filter(|file| file.0 != "sparse.img")
        .map(|file| file.2)
        .collect();
    assert_eq!(links.len(), 2);
    assert_eq!(links.iter().min(), Some(&0), "the second link adds nothing");
    assert!(links.iter().max().is_some_and(|bytes| *bytes >= 10_000));
    let sparse = files
        .iter()
        .find(|file| file.0 == "sparse.img")
        .expect("sparse");
    assert!(sparse.2 < sparse.1);
}
