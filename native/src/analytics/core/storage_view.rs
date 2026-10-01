//! Display view of an analysis tree for the Android facade: node kinds and
//! Android's approximate totals for what no app may walk (other apps' data,
//! apps and system). The approximate rows exist only in this view, never in
//! the result tree, so totals, transfers and the desktop stay exact.
use crate::analytics::SizeNode;
use serde::Serialize;

pub const OTHER_APP_DATA_NAME: &str = "Weitere App-Daten (laut Android, ≈)";
pub const UNCAPTURED_NAME: &str = "≈ Nicht einzeln erfasst";
const AGGREGATE_PREFIX: &str = "… ";
const AGGREGATE_SUFFIX: &str = " weitere Eintraege";
const APP_DATA_PATH: [&str; 2] = ["Android", "data"];

/// Name of the node that folds the files beyond a directory's retained ones.
pub(crate) fn aggregate_name(count: u64) -> String {
    format!("{AGGREGATE_PREFIX}{count}{AGGREGATE_SUFFIX}")
}

pub fn is_aggregate_name(name: &str) -> bool {
    name.strip_prefix(AGGREGATE_PREFIX)
        .and_then(|rest| rest.strip_suffix(AGGREGATE_SUFFIX))
        .is_some_and(|count| !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
    /// Files folded into one node (`… N weitere Eintraege`).
    Aggregate,
    /// Other apps' data as Android reports it (approximate, not walkable).
    Protected,
    /// Used space of the volume that no walk captured (apps, system).
    Rest,
}

impl NodeKind {
    fn of(node: &SizeNode) -> Self {
        if node.is_dir {
            Self::Dir
        } else if is_aggregate_name(&node.name) {
            Self::Aggregate
        } else {
            Self::File
        }
    }
}

/// Android's totals for the volume of the scan root: used bytes (`StatFs`)
/// and, with usage access, all apps' `Android/data` bytes
/// (`ExternalStorageStats.getAppBytes()`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlatformTotals {
    pub volume_used_bytes: Option<u64>,
    pub other_apps_bytes: Option<u64>,
}

/// Where the scan root lies on its volume, as far as the view needs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeRoot {
    /// Segments from the root to `Android/data` when that folder is the root
    /// or below it, on the primary volume (the only one Android reports
    /// other apps' data for).
    pub app_data: Option<Vec<String>>,
    /// The root is the volume root itself.
    pub whole_volume: bool,
}

impl VolumeRoot {
    /// From the root's segments below its volume root.
    pub fn from_segments(below: &[String], primary: bool) -> Self {
        let app_data = (primary
            && below.len() <= APP_DATA_PATH.len()
            && below
                .iter()
                .zip(APP_DATA_PATH)
                .all(|(have, want)| have.eq_ignore_ascii_case(want)))
        .then(|| {
            APP_DATA_PATH[below.len()..]
                .iter()
                .map(|name| name.to_string())
                .collect()
        });
        Self {
            app_data,
            whole_volume: below.is_empty(),
        }
    }
}

/// The approximate rows of one finished analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Approximations {
    /// Tree names of the `Android/data` node and the other apps' bytes.
    app_data: Option<(Vec<String>, u64)>,
    rest: Option<u64>,
}

impl Approximations {
    /// Other apps' data beyond what the walk measured in `Android/data`
    /// (only when positive), and at a whole volume root of a result without
    /// read errors the used space that is left (`used − measured − other`).
    pub fn compute(
        tree: &SizeNode,
        place: &VolumeRoot,
        totals: PlatformTotals,
        complete: bool,
    ) -> Self {
        let app_data = place
            .app_data
            .as_deref()
            .zip(totals.other_apps_bytes)
            .and_then(|(segments, other)| {
                let (names, node) = resolve(tree, segments)?;
                let extra = other.saturating_sub(node.size);
                (extra > 0).then_some((names, extra))
            });
        let extra = app_data.as_ref().map_or(0, |(_, bytes)| *bytes);
        let rest = totals
            .volume_used_bytes
            .filter(|_| place.whole_volume && complete)
            .map(|used| used.saturating_sub(tree.size.saturating_add(extra)))
            .filter(|rest| *rest > 0);
        Self { app_data, rest }
    }

    /// Approximate bytes the view adds inside the node at `segments`.
    fn added_within(&self, segments: &[String]) -> u64 {
        let app = match &self.app_data {
            Some((names, bytes)) if names.starts_with(segments) => *bytes,
            _ => 0,
        };
        let rest = if segments.is_empty() {
            self.rest.unwrap_or(0)
        } else {
            0
        };
        app.saturating_add(rest)
    }

    /// Approximate bytes inside the child `name` of the node at `segments`.
    fn added_within_child(&self, segments: &[String], name: &str) -> u64 {
        match &self.app_data {
            Some((names, bytes))
                if names.len() > segments.len()
                    && names.starts_with(segments)
                    && names[segments.len()] == name =>
            {
                *bytes
            }
            _ => 0,
        }
    }

    fn rows_at(&self, segments: &[String]) -> Vec<ChildView> {
        let mut rows = Vec::new();
        if let Some((names, bytes)) = &self.app_data {
            if names.as_slice() == segments {
                rows.push(synthetic(OTHER_APP_DATA_NAME, *bytes, NodeKind::Protected));
            }
        }
        if let (true, Some(rest)) = (segments.is_empty(), self.rest) {
            rows.push(synthetic(UNCAPTURED_NAME, rest, NodeKind::Rest));
        }
        rows
    }
}

fn synthetic(name: &str, size: u64, kind: NodeKind) -> ChildView {
    ChildView {
        name: name.to_string(),
        size,
        is_dir: false,
        child_count: 0,
        kind,
    }
}

/// Follows `segments` through directories, preferring exact names and
/// accepting other ASCII case (Android's shared storage ignores case).
fn resolve<'t>(tree: &'t SizeNode, segments: &[String]) -> Option<(Vec<String>, &'t SizeNode)> {
    let mut node = tree;
    let mut names = Vec::with_capacity(segments.len());
    for segment in segments {
        let current = node;
        let dirs = move || current.children.iter().filter(|child| child.is_dir);
        let next = dirs()
            .find(|child| &*child.name == segment.as_str())
            .or_else(|| dirs().find(|child| child.name.eq_ignore_ascii_case(segment)))?;
        names.push(next.name.to_string());
        node = next;
    }
    Some((names, node))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildView {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub child_count: usize,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub name: String,
    /// Shown size: measured plus the approximate rows below this node.
    pub size: u64,
    /// What the walk measured, without approximations.
    pub measured: u64,
    pub is_dir: bool,
    pub kind: NodeKind,
    pub children: Vec<ChildView>,
}

/// The node at `segments` (exact tree names) with its largest children, at
/// most `max_children`; approximate rows are always included.
pub fn node_view(
    tree: &SizeNode,
    segments: &[String],
    approx: &Approximations,
    max_children: usize,
) -> Option<NodeView> {
    let mut node = tree;
    for segment in segments {
        node = node
            .children
            .iter()
            .find(|child| &*child.name == segment.as_str())?;
    }
    let synthetic = approx.rows_at(segments);
    let mut sized: Vec<(&SizeNode, u64)> = node
        .children
        .iter()
        .map(|child| {
            let added = if child.is_dir {
                approx.added_within_child(segments, &child.name)
            } else {
                0
            };
            (child, child.size.saturating_add(added))
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
        .map(|(child, size)| ChildView {
            name: child.name.to_string(),
            size,
            is_dir: child.is_dir,
            child_count: child.children.len(),
            kind: NodeKind::of(child),
        })
        .collect();
    children.extend(synthetic);
    children.sort_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.name.cmp(&right.name))
    });
    Some(NodeView {
        name: node.name.to_string(),
        size: node.size.saturating_add(approx.added_within(segments)),
        measured: node.size,
        is_dir: node.is_dir,
        kind: NodeKind::of(node),
        children,
    })
}

#[cfg(test)]
#[path = "storage_view_tests.rs"]
mod tests;
