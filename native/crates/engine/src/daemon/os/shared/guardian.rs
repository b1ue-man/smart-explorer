//! Desktop supervisor. The child owns the existing daemon singleton, IPC
//! generation and handoff protocol; only an abnormal exit is restarted.
use std::path::Path;
use std::time::{Duration, Instant};
const CHILD_ENV: &str = "SE_SYNC_GUARDIAN_CHILD";
const SESSION_ENV: &str = "SE_SYNC_SESSION";
/// The same bounds the child applies to its handoff (`run_loop.rs`).
const HANDOFF_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(2);
const HANDOFF_QUEUE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long one attempt on the guardian lock waits before the handoff is
/// checked again.
const HANDOFF_QUEUE_POLL: Duration = Duration::from_millis(500);

pub(super) fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}
pub fn run_guardian() {
    let directory = crate::support_dirs::sync_data_dir();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        super::state::log(&error.to_string());
        return;
    }
    let Some(_guard) = lock_guardian(&directory.join("daemon.guardian.lock")) else {
        return;
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

/// The replacement a client asked for: its generation and the one it retires.
struct GuardianHandoff {
    generation: String,
    retiring_generation: Option<String>,
}

/// An ordinary start takes the guardian lock or ends at once. A handoff
/// guardian queues behind the running guardian only while its handoff is
/// still the current one, by the same rules its child applies to the daemon
/// singleton (`handoff::acquire_instance_guard`): once another replacement
/// superseded it, or a stop was requested, it ends instead of remaining a
/// detached `--sync-daemon` process for up to five minutes.
fn lock_guardian(path: &Path) -> Option<crate::syncjobs::RuntimeFileLock> {
    let handoff = match handoff_from_environment() {
        Ok(Some(handoff)) => handoff,
        Ok(None) => {
            return crate::syncjobs::RuntimeFileLock::acquire(path, Duration::ZERO)
                .inspect_err(|error| super::state::log(&format!("guardian not started: {error}")))
                .ok();
        }
        Err(reason) => {
            super::state::log(&format!("guardian not started: {reason}"));
            return None;
        }
    };
    if !super::handoff::wait_for_handoff_activation(&handoff.generation, HANDOFF_ACTIVATION_TIMEOUT)
    {
        return None;
    }
    let deadline = Instant::now() + HANDOFF_QUEUE_TIMEOUT;
    loop {
        if !handoff_still_current(&handoff) {
            super::state::log("guardian handoff stopped while waiting for the running guardian");
            return None;
        }
        match crate::syncjobs::RuntimeFileLock::acquire(path, HANDOFF_QUEUE_POLL) {
            // The handoff may have been superseded while the lock was taken.
            Ok(guard) if handoff_still_current(&handoff) => return Some(guard),
            Ok(_) => {
                super::state::log("guardian handoff was superseded before the guardian lock");
                return None;
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                if Instant::now() >= deadline {
                    super::state::log(
                        "guardian handoff timed out waiting for the running guardian",
                    );
                    return None;
                }
            }
            Err(error) => {
                super::state::log(&format!("guardian not started: {error}"));
                return None;
            }
        }
    }
}

fn handoff_still_current(handoff: &GuardianHandoff) -> bool {
    super::handoff::should_continue_waiting_for_singleton(
        &handoff.generation,
        handoff.retiring_generation.as_deref(),
    )
}

/// The handoff the client handed to this guardian. The child receives the
/// same variables and refuses invalid values too (`schedule::run_daemon`).
fn handoff_from_environment() -> Result<Option<GuardianHandoff>, &'static str> {
    let Some(generation) = std::env::var_os(crate::autostart::DAEMON_HANDOFF_ENV) else {
        return Ok(None);
    };
    let generation = generation
        .into_string()
        .ok()
        .filter(|value| valid_generation(value))
        .ok_or("invalid handoff generation")?;
    let retiring_generation =
        match std::env::var_os(crate::autostart::DAEMON_RETIRING_GENERATION_ENV) {
            Some(value) => Some(
                value
                    .into_string()
                    .ok()
                    .filter(|value| valid_generation(value))
                    .ok_or("invalid retiring generation")?,
            ),
            None => None,
        };
    Ok(Some(GuardianHandoff {
        generation,
        retiring_generation,
    }))
}

fn valid_generation(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
