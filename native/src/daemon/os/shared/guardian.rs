//! Desktop supervisor. The child owns the existing daemon singleton, IPC
//! generation and handoff protocol; only an abnormal exit is restarted.
use std::time::{Duration, Instant};
const CHILD_ENV: &str = "SE_SYNC_GUARDIAN_CHILD";
const SESSION_ENV: &str = "SE_SYNC_SESSION";

pub(super) fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}
pub fn run_guardian() {
    let directory = crate::support_dirs::sync_data_dir();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        super::state::log(&error.to_string());
        return;
    }
    let handoff = std::env::var_os(crate::autostart::DAEMON_HANDOFF_ENV).is_some();
    let wait = if handoff {
        Duration::from_secs(300)
    } else {
        Duration::ZERO
    };
    let _guard = match crate::syncjobs::RuntimeFileLock::acquire(
        &directory.join("daemon.guardian.lock"),
        wait,
    ) {
        Ok(guard) => guard,
        Err(error) => {
            super::state::log(&format!("guardian not started: {error}"));
            return;
        }
    };
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            super::state::log(&error.to_string());
            return;
        }
    };
    let session = super::platform::session_marker();
    let mut retry = 1;
    let mut first = true;
    loop {
        let mut command = super::platform::daemon_command(&executable);
        command.arg("--sync-daemon").env(CHILD_ENV, "1");
        if let Some(session) = &session {
            command.env(SESSION_ENV, session);
        }
        if !first {
            command
                .env_remove(crate::autostart::DAEMON_HANDOFF_ENV)
                .env_remove(crate::autostart::DAEMON_RETIRING_GENERATION_ENV);
        }
        first = false;
        let started = Instant::now();
        match command.spawn().and_then(|mut child| child.wait()) {
            Ok(status) if status.success() => return,
            Ok(status) => super::state::log(&format!(
                "daemon exited abnormally ({status}); restarting in {retry}s"
            )),
            Err(error) => super::state::log(&format!(
                "daemon launch failed ({error}); retrying in {retry}s"
            )),
        }
        if super::state::stop_path().exists() {
            return;
        }
        if started.elapsed() >= Duration::from_secs(60) {
            retry = 1;
        }
        for _ in 0..retry {
            if super::state::stop_path().exists() {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        retry = (retry * 2).min(60);
    }
}
