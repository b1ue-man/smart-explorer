//! Headless background sync (#4). Started as `smart_explorer --sync-daemon`
//! from a per-user logon autostart entry - see `autostart.rs`. It opens no
//! window: it loops on a short tick, runs every *due* saved sync job, reacts to
//! the event triggers (on-startup, real-time change, device/USB connect), writes
//! a heartbeat the GUI can read, then sleeps. Because the daemon is the *same
//! executable*, a self-update swaps it too.
//!
//! Safety mirrors the interactive sync exactly (same `bisync::run`): only files
//! that actually changed move, both-sides-changed stays a conflict (nothing is
//! silently overwritten), changes are reversible. Unresolved conflicts are left
//! for the user to settle in the GUI - the daemon never guesses.

#[path = "os/shared/backend_server.rs"]
mod backend_server;
#[path = "os/shared/ipc.rs"]
mod ipc;
#[path = "os/shared/job.rs"]
mod job;
#[cfg(windows)]
#[path = "os/windows/platform.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/platform.rs"]
mod platform;
#[path = "os/shared/schedule.rs"]
mod schedule;
#[path = "os/shared/state.rs"]
mod state;

pub use ipc::{
    drain_share_worker_events, ensure_worker_ready, open_share_backend, refresh_share_worker,
    send_share_command,
};
#[allow(unused_imports)]
pub use platform::DriveInfo;
pub use schedule::run_daemon;
#[allow(unused_imports)]
pub use state::{
    autopause_flags, cadence_secs, clear_stop, is_running, last_heartbeat_age, pause_for_secs,
    pause_indefinite, pause_remaining, pause_until, read_log_tail, request_stop, resume,
    set_autopause_flags, set_cadence_secs,
};

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
