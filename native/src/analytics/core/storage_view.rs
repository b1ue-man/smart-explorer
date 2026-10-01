//! Display view of an analysis tree for the Android facade: node kinds and
//! Android's approximate figures for what no app may walk (other apps' data,
//! the installed apps, apps and system). The approximate rows exist only in
//! this view, never in the result tree, so totals, transfers and the desktop
//! stay exact.
use crate::analytics::SizeNode;
use serde::Serialize;
use std::cmp::Ordering;

pub const OTHER_APP_DATA_NAME: &str = "Weitere App-Daten (laut Android, ≈)";
pub const UNCAPTURED_NAME: &str = "≈ Nicht einzeln erfasst";
/// Row of all installed apps at a whole primary volume root; it opens the
/// app list (its name is the path segment that leads there).
const APPS_NAME: &str = "≈ Apps (laut Android)";
/// The rest once the apps are listed: what neither the walk nor the apps hold.
const SYSTEM_AND_OTHER_NAME: &str = "≈ System und Sonstiges";
const AGGREGATE_PREFIX: &str = "… ";
const AGGREGATE_SUFFIX: &str = " weitere Eintraege";
const APP_DATA_PATH: [&str; 2] = ["Android", "data"];
const APP_OBB_PATH: [&str; 2] = ["Android", "obb"];

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
    /// All installed apps as Android reports them; opens the app list.
    Apps,
    /// One installed app with Android's figures (not walkable).
    App,
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

/// One installed app's storage on the primary volume as Android reports it
/// (`StorageStatsManager.queryStatsForPackage`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    pub package: String,
    /// Shown as the row's name.
    #[serde(skip)]
    pub label: String,
    /// APK, compiled code, native libraries and the app's `Android/obb`.
    pub app_bytes: u64,
    /// Private data with the app's `Android/data`, cache included.
    pub data_bytes: u64,
    /// The cache part of `data_bytes`.
    pub cache_bytes: u64,
}

impl AppUsage {
    /// What the app occupies: code plus data (the cache is part of the data).
    fn size(&self) -> u64 {
        self.app_bytes.saturating_add(self.data_bytes)
    }
}

/// Android's figures for the volume of the scan root: used bytes (`StatFs`),
/// with usage access all apps' `Android/data` bytes
/// (`ExternalStorageStats.getAppBytes()`) and, at a whole primary volume
/// root, every installed app's storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlatformTotals {
    pub volume_used_bytes: Option<u64>,
    pub other_apps_bytes: Option<u64>,
    /// Installed apps; empty = no app list.
    pub apps: Vec<AppUsage>,
}

impl PlatformTotals {
    /// Adds one app's figures: a blank label shows the package name, the
    /// cache counts at most as much as the data it is part of.
    pub fn add_app(
        &mut self,
        package: String,
        label: String,
        app_bytes: u64,
        data_bytes: u64,
        cache_bytes: u64,
    ) {
        let label = if label.trim().is_empty() {
            package.clone()
        } else {
            label
        };
        self.apps.push(AppUsage {
            package,
            label,
            app_bytes,
            data_bytes,
            cache_bytes: cache_bytes.min(data_bytes),
        });
    }
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

    /// The root is the primary volume itself (where Android lists apps).
    fn whole_primary(&self) -> bool {
        self.whole_volume && self.app_data.is_some()
    }
}

/// The approximate rows of one finished analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Approximations {
    /// Tree names of the `Android/data` node and the other apps' bytes.
    app_data: Option<(Vec<String>, u64)>,
    rest: Option<u64>,
    /// Installed apps at a whole primary volume root, largest first.
    apps: Vec<AppUsage>,
    /// What the walk measured in `Android/data` and `Android/obb`: the app
    /// figures hold it as well, so the totals count it once.
    apps_overlap: u64,
}

impl Approximations {
    /// With an app list at a whole primary volume root: the apps row (other
    /// apps' data is part of it). Otherwise other apps' data beyond what the
    /// walk measured in `Android/data` (only when positive). At a whole
    /// volume root of a result without read errors also the used space that
    /// is left (`used − measured − apps or other apps`, counting the app
    /// folders the walk measured once).
    pub fn compute(
        tree: &SizeNode,
        place: &VolumeRoot,
        totals: PlatformTotals,
        complete: bool,
    ) -> Self {
        let apps = if place.whole_primary() {
            listed(totals.apps)
        } else {
            Vec::new()
        };
        let app_data = if apps.is_empty() {
            other_app_data(tree, place, totals.other_apps_bytes)
        } else {
            None
        };
        let apps_overlap = if apps.is_empty() {
            0
        } else {
            measured_app_folders(tree).min(total_of(&apps))
        };
        let mut approx = Self {
            app_data,
            rest: None,
            apps,
            apps_overlap,
        };
        approx.rest = totals
            .volume_used_bytes
            .filter(|_| place.whole_volume && complete)
            .map(|used| used.saturating_sub(tree.size.saturating_add(approx.beyond_walk())))
            .filter(|rest| *rest > 0);
        approx
    }

    fn apps_total(&self) -> u64 {
        total_of(&self.apps)
    }

    /// Approximate bytes besides the walk's: other apps' data, or the apps
    /// without what the walk measured of them.
    fn beyond_walk(&self) -> u64 {
        let other = self.app_data.as_ref().map_or(0, |(_, bytes)| *bytes);
        other.saturating_add(self.apps_total().saturating_sub(self.apps_overlap))
    }

    /// Approximate bytes the view adds inside the node at `segments`.
    fn added_within(&self, segments: &[String]) -> u64 {
        if segments.is_empty() {
            return self.beyond_walk().saturating_add(self.rest.unwrap_or(0));
        }
        match &self.app_data {
            Some((names, bytes)) if names.starts_with(segments) => *bytes,
            _ => 0,
        }
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
        if !segments.is_empty() {
            return rows;
        }
        if !self.apps.is_empty() {
            rows.push(ChildView {
                is_dir: true,
                child_count: self.apps.len(),
                ..synthetic(APPS_NAME, self.apps_total(), NodeKind::Apps)
            });
        }
        if let Some(rest) = self.rest {
            let name = if self.apps.is_empty() {
                UNCAPTURED_NAME
            } else {
                SYSTEM_AND_OTHER_NAME
            };
            rows.push(synthetic(name, rest, NodeKind::Rest));
        }
        rows
    }

    /// The app list behind the apps row, largest first; beyond
    /// `max_children` the smallest apps share one row.
    fn apps_view(&self, max_children: usize) -> NodeView {
        let mut children: Vec<ChildView> = self.apps.iter().map(app_row).collect();
        if children.len() > max_children {
            let folded = children.split_off(max_children.saturating_sub(1));
            let size = folded
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.size));
            let name = aggregate_name(folded.len() as u64);
            children.push(synthetic(&name, size, NodeKind::Aggregate));
            children.sort_by(by_size_then_name);
        }
        NodeView {
            name: APPS_NAME.to_string(),
            size: self.apps_total(),
            measured: 0,
            is_dir: true,
            kind: NodeKind::Apps,
            children,
        }
    }
}

/// Other apps' data beyond what the walk measured in `Android/data`.
fn other_app_data(
    tree: &SizeNode,
    place: &VolumeRoot,
    other_apps_bytes: Option<u64>,
) -> Option<(Vec<String>, u64)> {
    let (segments, other) = place.app_data.as_deref().zip(other_apps_bytes)?;
    let (names, node) = resolve(tree, segments)?;
    let extra = other.saturating_sub(node.size);
    (extra > 0).then_some((names, extra))
}

/// The apps worth a row (code or data), each package once, largest first.
fn listed(mut apps: Vec<AppUsage>) -> Vec<AppUsage> {
    apps.retain(|app| app.size() > 0);
    apps.sort_by(|left, right| left.package.cmp(&right.package));
    apps.dedup_by(|later, first| later.package == first.package);
    apps.sort_by(|left, right| {
        right
            .size()
            .cmp(&left.size())
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.package.cmp(&right.package))
    });
    apps
}

fn total_of(apps: &[AppUsage]) -> u64 {
    apps.iter()
        .fold(0u64, |total, app| total.saturating_add(app.size()))
}

/// What the walk measured in `Android/data` and `Android/obb` of a volume
/// root: the own app's folders (and whatever else Android let it read
/// there), which the app figures contain as well.
fn measured_app_folders(tree: &SizeNode) -> u64 {
    [APP_DATA_PATH, APP_OBB_PATH]
        .iter()
        .filter_map(|path| resolve(tree, path).map(|(_, node)| node.size))
        .fold(0u64, u64::saturating_add)
}

fn synthetic(name: &str, size: u64, kind: NodeKind) -> ChildView {
    ChildView {
        name: name.to_string(),
        size,
        is_dir: false,
        child_count: 0,
        kind,
        app: None,
    }
}

fn app_row(app: &AppUsage) -> ChildView {
    ChildView {
        name: app.label.clone(),
        app: Some(app.clone()),
        ..synthetic("", app.size(), NodeKind::App)
    }
}

fn by_size_then_name(left: &ChildView, right: &ChildView) -> Ordering {
    right
        .size
        .cmp(&left.size)
        .then_with(|| left.name.cmp(&right.name))
}

/// Follows `segments` through directories, preferring exact names and
/// accepting other ASCII case (Android's shared storage ignores case).
fn resolve<'t, S: AsRef<str>>(
    tree: &'t SizeNode,
    segments: &[S],
) -> Option<(Vec<String>, &'t SizeNode)> {
    let mut node = tree;
    let mut names = Vec::with_capacity(segments.len());
    for segment in segments {
        let segment = segment.as_ref();
        let current = node;
        let dirs = move || current.children.iter().filter(|child| child.is_dir);
        let next = dirs()
            .find(|child| &*child.name == segment)
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
            app: None,
        })
        .collect();
    children.extend(synthetic);
    children.sort_by(by_size_then_name);
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
