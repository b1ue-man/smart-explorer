#[path = "os/shared/reclaim/backend_recycle.rs"]
mod backend_recycle;
#[path = "core/progress.rs"]
mod progress;
#[path = "os/shared/reclaim/mod.rs"]
mod reclaim;
pub use backend_recycle::{recycle_plan, RecyclePlan};
#[path = "os/shared/analytics.rs"]
mod scanner;
pub use progress::{Progress, ScanPhase, ScanSnapshot};
#[path = "os/shared/remote.rs"]
mod remote;
#[path = "core/tree_transfer.rs"]
pub mod tree_transfer;
pub use remote::finish_legacy_tree;
pub use remote::scan_remote;

pub use reclaim::*;
pub use scanner::scan_confined;
pub use scanner::scan_confined_with;
pub use scanner::*;
pub use scanner::{local_scan_threads, scan_with, scan_with_guard, ScanBudget};
#[cfg(all(test, windows))]
#[path = "os/windows/access_task.rs"]
mod access_task;
mod os;
pub use os::host_recycle_available;
pub use os::{
    host_permission_note, recycle as recycle_local, volume_usage as host_volume_usage,
};

#[path = "core/analysis_budget.rs"]
mod analysis_budget;
#[path = "core/analysis_report.rs"]
mod analysis_report;
pub use analysis_budget::fit_tree;
#[path = "core/analysis_transfer.rs"]
pub mod analysis_transfer;
pub use analysis_report::AnalysisReport;
#[path = "core/host_figures.rs"]
mod host_figures;
#[path = "core/tree_deflate.rs"]
pub mod tree_deflate;
pub use host_figures::{
    remember_platform_totals, remembered_platform_totals, PlatformApp, PlatformFigures, VolumeUsage,
};

#[path = "core/protected.rs"]
mod protected;
pub use protected::ProtectedTally;
pub use protected::{protected_count, protected_note, protected_text, ProtectedOmission};
#[path = "core/status_text.rs"]
mod status_text;
pub use status_text::walk_status;
pub use status_text::{compare_status, thousands};
#[path = "core/storage_view.rs"]
mod storage_view;
pub use storage_view::aggregate_name;
pub use storage_view::{
    is_aggregate_name, node_view, Approximations, ChildView, NodeKind, NodeView, PlatformTotals,
    VolumeRoot, OTHER_APP_DATA_NAME, UNCAPTURED_NAME,
};

#[path = "core/partial_merge.rs"]
mod partial_merge;
pub use partial_merge::{
    merge_retry_tree, preserve_retry_omissions, retry_node, RetryOmission,
};
