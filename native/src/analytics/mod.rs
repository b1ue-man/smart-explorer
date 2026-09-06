#[path = "os/shared/reclaim/mod.rs"]
mod reclaim;
#[path = "os/shared/analytics.rs"]
mod scanner;

pub use reclaim::*;
pub use scanner::*;
mod access;
mod os;
pub use access::{
    can_request_elevation, launch_elevated_analysis, parse_analysis_startup,
    verify_analysis_startup, AnalysisStartup,
};
