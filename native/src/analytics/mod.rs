#[path = "core/progress.rs"]
mod progress;
#[path = "os/shared/reclaim/mod.rs"]
mod reclaim;
#[path = "os/shared/analytics.rs"]
mod scanner;
pub use progress::{Progress, ScanPhase, ScanSnapshot};
#[path = "os/shared/remote.rs"]
mod remote;
#[path = "core/tree_transfer.rs"]
pub(crate) mod tree_transfer;
pub use remote::scan_remote;

pub use reclaim::*;
pub use scanner::*;
pub(crate) use scanner::{local_scan_threads, scan_with_guard};
mod os;

#[path = "core/analysis_report.rs"]
mod analysis_report;
#[path = "core/analysis_transfer.rs"]
pub(crate) mod analysis_transfer;
pub(crate) use analysis_report::AnalysisReport;

#[path = "core/protected.rs"]
mod protected;
pub(crate) use protected::ProtectedTally;
pub use protected::{protected_count, protected_note, protected_text, ProtectedOmission};
#[path = "core/status_text.rs"]
mod status_text;
pub use status_text::walk_status;
pub(crate) use status_text::{compare_status, thousands};
#[path = "core/storage_view.rs"]
mod storage_view;
pub(crate) use storage_view::aggregate_name;
pub use storage_view::{
    is_aggregate_name, node_view, Approximations, ChildView, NodeKind, NodeView, PlatformTotals,
    VolumeRoot, OTHER_APP_DATA_NAME, UNCAPTURED_NAME,
};
