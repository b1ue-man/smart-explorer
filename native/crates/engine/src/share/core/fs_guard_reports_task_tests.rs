use super::*;
use crate::{agent_proto::WireNode, share::export_config::ExportAccess};

fn node(name: &str, size: u64, children: Vec<WireNode>) -> WireNode {
    WireNode {
        name: name.into(),
        size,
        is_dir: !children.is_empty(),
        children,
    }
}

#[test]
fn review_task_host_fast_report_removes_private_sizes_from_every_ancestor() {
    let policy = TargetPolicy::new(ExportAccess::ReadOnly, false, false);
    let tree = node(
        "export",
        100,
        vec![
            node(".se-versions", 20, vec![]),
            node(
                "folder",
                80,
                vec![
                    node("kept", 30, vec![]),
                    node(".held.se-recycle-0123456789abcdef", 30, vec![]),
                ],
            ),
        ],
    );
    let filtered = wire(tree, "/export", &policy).unwrap();
    assert_eq!(filtered.size, 50);
    assert_eq!(filtered.children.len(), 1);
    assert_eq!(filtered.children[0].size, 50);
    assert_eq!(filtered.children[0].children.len(), 1);
    assert_eq!(filtered.children[0].children[0].name, "kept");
}

#[test]
fn review_task_host_fast_report_preserves_provider_stage_and_rejects_path_injection() {
    let policy = TargetPolicy::new(ExportAccess::ReadOnly, false, false);
    let tree = node(
        "root",
        7,
        vec![node(".file.smart-explorer-abcd.part", 7, vec![])],
    );
    assert_eq!(wire(tree, "/root", &policy).unwrap().size, 7);
    let invalid = node("root", 1, vec![node("../secret", 1, vec![])]);
    assert!(wire(invalid, "/root", &policy).is_err());
}
