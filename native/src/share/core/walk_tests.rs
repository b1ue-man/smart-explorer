use std::sync::{Arc, Mutex};

use super::fs::{ShareExportConfig, SharedRoot};
use super::walk::{NodeBatch, ServerWalker, WalkEvent};
use super::walk_assembly::{TreeAssembler, WalkTotals, WALK_BATCH_NODES};
use super::wire::{Ctrl, FsResponse, FsWalkNode};

#[test]
fn postorder_batches_assemble_and_recompute_directory_sizes() {
    let mut tree = TreeAssembler::default();
    let nodes = vec![
        flat(2, Some(0), "top.bin", false, 3),
        flat(3, Some(1), "nested.bin", false, 7),
        flat(1, Some(0), "sub", true, 0),
        flat(0, None, "/", true, 0),
    ];
    let totals = WalkTotals {
        files: 2,
        dirs: 2,
        bytes: 10,
    };
    tree.push_batch(nodes, totals).unwrap();
    let root = tree.finish(totals, 4).unwrap();
    assert_eq!(root.size, 10);
    assert!(root.children[0].is_dir);
    assert_eq!(root.children[0].size, 7);
}

#[test]
fn assembler_rejects_duplicate_and_orphaned_nodes() {
    let mut duplicate = TreeAssembler::default();
    duplicate.push(flat(1, Some(0), "a", false, 1)).unwrap();
    assert!(duplicate.push(flat(1, Some(0), "b", false, 1)).is_err());

    let mut orphan = TreeAssembler::default();
    orphan.push(flat(2, Some(1), "a", false, 1)).unwrap();
    assert!(orphan
        .finish(
            WalkTotals {
                files: 1,
                dirs: 0,
                bytes: 1,
            },
            1,
        )
        .is_err());
}

#[test]
fn assembler_rejects_excessive_tree_depth() {
    let mut tree = TreeAssembler::default();
    for id in (0..=512u64).rev() {
        let node = flat(
            id,
            id.checked_sub(1),
            if id == 0 { "/" } else { "d" },
            true,
            0,
        );
        if id == 0 {
            assert!(tree.push(node).is_err());
        } else {
            tree.push(node).unwrap();
        }
    }
}

#[test]
fn node_batches_never_exceed_the_wire_bound() {
    let mut batch = NodeBatch::default();
    for id in 0..WALK_BATCH_NODES as u64 {
        let flushed = batch.push(flat(id, id.checked_sub(1), "n", false, 1));
        if id + 1 == WALK_BATCH_NODES as u64 {
            assert_eq!(flushed.unwrap().len(), WALK_BATCH_NODES);
        } else {
            assert!(flushed.is_none());
        }
    }
    assert!(batch.is_empty());
}

#[test]
fn server_walker_uses_export_paths_and_skips_symlinks() {
    let base = std::env::temp_dir().join(format!("se-share-walk-{}", std::process::id()));
    let root = base.join("root");
    let outside = base.join("outside");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("top.bin"), [0u8; 3]).unwrap();
    std::fs::write(root.join("sub/nested.bin"), [0u8; 7]).unwrap();
    std::fs::write(outside.join("secret.bin"), [0u8; 11]).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(&outside, root.join("escape"));
    #[cfg(not(windows))]
    let _ = std::os::unix::fs::symlink(&outside, root.join("escape"));

    let exports = Arc::new(Mutex::new(ShareExportConfig {
        roots: vec![SharedRoot {
            label: "Gate".into(),
            path: root.to_string_lossy().replace('\\', "/"),
        }],
        ..Default::default()
    }));
    let mut walker = ServerWalker::new("/Gate".into(), exports.clone()).unwrap();
    let mut names = Vec::new();
    while let Some(event) = walker.next_event().unwrap() {
        if let WalkEvent::Node(node) = event {
            names.push(node.name);
        }
    }
    assert!(names.iter().any(|name| name == "nested.bin"));
    assert!(!names
        .iter()
        .any(|name| name == "escape" || name == "secret.bin"));
    assert!(ServerWalker::new("/NotShared".into(), exports)
        .unwrap()
        .next_event()
        .is_err());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn walk_wire_batch_roundtrips() {
    let ctrl = Ctrl::FsResp {
        resp: FsResponse::WalkBatch {
            nodes: vec![flat(0, None, "/", true, 0)],
            files: 0,
            dirs: 1,
            bytes: 0,
        },
    };
    let encoded = serde_json::to_vec(&ctrl).unwrap();
    assert!(matches!(
        serde_json::from_slice::<Ctrl>(&encoded).unwrap(),
        Ctrl::FsResp {
            resp: FsResponse::WalkBatch { .. }
        }
    ));
}

fn flat(id: u64, parent: Option<u64>, name: &str, is_dir: bool, size: u64) -> FsWalkNode {
    FsWalkNode {
        id,
        parent,
        name: name.into(),
        is_dir,
        size,
    }
}
