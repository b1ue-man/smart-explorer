//! The view of one node with its largest children (Android facade).
use super::{by_size_then_name, AppUsage, Approximations, NodeKind, APPS_NAME};
use crate::analytics::SizeNode;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildView {
    pub name: String,
    pub size: u64,
    /// Occupied bytes (`SizeNode::allocated`); approximate rows: their size.
    pub allocated: u64,
    pub is_dir: bool,
    pub child_count: usize,
    pub kind: NodeKind,
    /// Android's figures of an app row (`kind: "app"`): `package`,
    /// `appBytes`, `dataBytes`, `cacheBytes`.
    #[serde(flatten)]
    pub app: Option<AppUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub name: String,
    /// Shown size: measured plus the approximate rows below this node (bytes
    /// in both the measured app folders and the apps row counted once).
    pub size: u64,
    /// Occupied bytes of the shown size: the walk's `SizeNode::allocated`
    /// plus the approximate rows below this node at their size.
    pub allocated: u64,
    /// What the walk measured, without approximations.
    pub measured: u64,
    pub is_dir: bool,
    pub kind: NodeKind,
    pub children: Vec<ChildView>,
}

/// The node at `segments` (exact tree names) with its largest children, at
/// most `max_children`; approximate rows are always included. With an app
/// list, the apps row's name at the root leads to the app list.
pub fn node_view(
    tree: &SizeNode,
    segments: &[String],
    approx: &Approximations,
    max_children: usize,
) -> Option<NodeView> {
    if let [first, below @ ..] = segments {
        if !approx.apps.is_empty() && first == APPS_NAME {
            return below.is_empty().then(|| approx.apps_view(max_children));
        }
    }
    let mut node = tree;
    for segment in segments {
        node = node
            .children
            .iter()
            .find(|child| &*child.name == segment.as_str())?;
    }
    let synthetic = approx.rows_at(segments);
    let mut sized: Vec<(&SizeNode, u64, u64)> = node
        .children
        .iter()
        .map(|child| {
            let added = if child.is_dir {
                approx.added_within_child(segments, &child.name)
            } else {
                0
            };
            (
                child,
                child.size.saturating_add(added),
                child.allocated.saturating_add(added),
            )
        })
        .collect();
    sized.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.name.cmp(&right.0.name))
    });
    sized.truncate(max_children.saturating_sub(synthetic.len()));
    let mut children: Vec<ChildView> = sized
        .into_iter()
        .map(|(child, size, allocated)| ChildView {
            name: child.name.to_string(),
            size,
            allocated,
            is_dir: child.is_dir,
            child_count: child.children.len(),
            kind: NodeKind::of(child),
            app: None,
        })
        .collect();
    children.extend(synthetic);
    children.sort_by(by_size_then_name);
    Some(NodeView {
        name: node.name.to_string(),
        size: node.size.saturating_add(approx.added_within(segments)),
        allocated: node.allocated.saturating_add(approx.added_within(segments)),
        measured: node.size,
        is_dir: node.is_dir,
        kind: NodeKind::of(node),
        children,
    })
}
