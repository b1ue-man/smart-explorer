use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{ipc_storage, platform, state};

static CLAIM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
enum StopControl {
    Manual,
    Handoff(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HandoffActivation {
    Waiting,
    Active,
    Superseded,
}

pub(crate) fn request_handoff(generation: &str) -> io::Result<()> {
    if !valid_generation(generation) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "handoff generation must be 32 hexadecimal characters",
        ));
    }
    state::write_control(&state::stop_path(), &format!("handoff:{generation}"))
}

pub(crate) fn wait_for_handoff_activation(generation: &str, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match handoff_activation_checked(generation) {
            Ok(HandoffActivation::Active) => return true,
            Ok(HandoffActivation::Waiting | HandoffActivation::Superseded)
                if std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(HandoffActivation::Waiting) => {
                // v0.5.125 and older remove every daemon.stop value during
                // shutdown. The singleton remains the authoritative gate.
                state::log("daemon handoff activation was consumed by a legacy worker");
                return true;
            }
            Ok(HandoffActivation::Superseded) => {
                state::log("daemon handoff was superseded before activation");
                return false;
            }
            Err(error) => {
                state::log(&format!(
                    "daemon handoff activation could not be verified: {error}"
                ));
                return false;
            }
        }
    }
}

/// Claims the stop control that existed when this child won the singleton.
/// Superseding controls are restored without replacing a still newer writer.
pub(crate) fn claim_handoff_after_singleton(generation: &str) -> io::Result<bool> {
    let Some(claimed) = claim_current_stop()? else {
        return Ok(true);
    };
    let accepted = matches!(
        &claimed.control,
        StopControl::Handoff(target) if target == generation
    );
    if accepted {
        claimed.consume()?;
    } else {
        claimed.restore()?;
    }
    Ok(accepted)
}

pub(crate) fn discard_stop_after_singleton() -> io::Result<()> {
    if let Some(claimed) = claim_current_stop()? {
        claimed.consume()?;
    }
    Ok(())
}

pub(crate) fn should_continue_waiting_for_singleton(
    generation: &str,
    retiring_generation: Option<&str>,
) -> bool {
    let control = match stop_control_checked() {
        Ok(control) => control,
        Err(error) => {
            state::log(&format!(
                "daemon handoff stopped: control could not be verified: {error}"
            ));
            return false;
        }
    };
    should_continue_waiting(
        control.as_ref(),
        ipc_storage::read_ipc_generation().as_deref(),
        generation,
        retiring_generation,
    )
}

pub(crate) fn acquire_instance_guard(
    handoff: bool,
    generation: &str,
    retiring_generation: Option<&str>,
    timeout: std::time::Duration,
) -> Option<platform::DaemonInstanceGuard> {
    if !handoff {
        return platform::acquire_daemon_instance_guard(std::time::Duration::ZERO);
    }
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !should_continue_waiting_for_singleton(generation, retiring_generation) {
            state::log("daemon handoff stopped while waiting for the singleton");
            return None;
        }
        if let Some(guard) =
            platform::acquire_daemon_instance_guard(std::time::Duration::from_millis(500))
        {
            return Some(guard);
        }
        if std::time::Instant::now() >= deadline {
            state::log("daemon handoff timed out waiting for the previous instance");
            return None;
        }
    }
}

fn should_continue_waiting(
    control: Option<&StopControl>,
    published_generation: Option<&str>,
    generation: &str,
    retiring_generation: Option<&str>,
) -> bool {
    match control {
        Some(StopControl::Handoff(target)) => target == generation,
        Some(StopControl::Manual) => false,
        None => match published_generation {
            Some(published) => retiring_generation == Some(published),
            None => true,
        },
    }
}

pub(crate) fn stop_requested_for(generation: &str) -> bool {
    stop_requested_checked_for(generation).unwrap_or(true)
}

pub(crate) fn stop_requested_checked_for(generation: &str) -> io::Result<bool> {
    Ok(match stop_control_checked()? {
        None => false,
        Some(StopControl::Manual) => true,
        Some(StopControl::Handoff(target)) => target != generation,
    })
}

fn handoff_activation_checked(generation: &str) -> io::Result<HandoffActivation> {
    Ok(match stop_control_checked()? {
        None => HandoffActivation::Waiting,
        Some(StopControl::Handoff(target)) if target == generation => HandoffActivation::Active,
        Some(StopControl::Manual | StopControl::Handoff(_)) => HandoffActivation::Superseded,
    })
}

fn stop_control_checked() -> io::Result<Option<StopControl>> {
    match state::read_optional(&state::stop_path())? {
        Some(value) => parse_stop_control(&value).map(Some),
        None => Ok(None),
    }
}

fn parse_stop_control(value: &str) -> io::Result<StopControl> {
    let value = value.trim();
    if value == "stop" {
        return Ok(StopControl::Manual);
    }
    if let Some(generation) = value.strip_prefix("handoff:") {
        if valid_generation(generation) {
            return Ok(StopControl::Handoff(generation.to_string()));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid daemon stop control",
    ))
}

fn valid_generation(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct ClaimedStop {
    original: PathBuf,
    claimed: PathBuf,
    control: StopControl,
}

impl ClaimedStop {
    fn consume(self) -> io::Result<()> {
        std::fs::remove_file(self.claimed)
    }

    fn restore(self) -> io::Result<()> {
        restore_claimed_path(&self.original, &self.claimed)
    }
}

fn claim_current_stop() -> io::Result<Option<ClaimedStop>> {
    claim_stop_at(&state::stop_path())
}

fn claim_stop_at(original: &Path) -> io::Result<Option<ClaimedStop>> {
    let parent = original
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "stop path has no parent"))?;
    let sequence = CLAIM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let claimed = parent.join(format!(
        ".daemon.stop.claim.{}.{}",
        std::process::id(),
        sequence
    ));
    match std::fs::rename(original, &claimed) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }

    let parsed = read_claimed_control(&claimed);
    match parsed {
        Ok(control) => Ok(Some(ClaimedStop {
            original: original.to_path_buf(),
            claimed,
            control,
        })),
        Err(error) => match restore_claimed_path(original, &claimed) {
            Ok(()) => Err(error),
            Err(restore) => Err(io::Error::new(
                error.kind(),
                format!("{error}; stop control restore failed: {restore}"),
            )),
        },
    }
}

fn read_claimed_control(path: &Path) -> io::Result<StopControl> {
    let metadata = std::fs::symlink_metadata(path)?;
    if platform::metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon stop control must be a regular file, not a link",
        ));
    }
    parse_stop_control(&std::fs::read_to_string(path)?)
}

fn restore_claimed_path(original: &Path, claimed: &Path) -> io::Result<()> {
    match platform::restore_control_if_absent(claimed, original)? {
        true => Ok(()),
        false => std::fs::remove_file(claimed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "11111111111111111111111111111111";
    const SECOND: &str = "22222222222222222222222222222222";

    #[test]
    fn controls_are_strict_and_generation_scoped() {
        assert_eq!(parse_stop_control("stop").unwrap(), StopControl::Manual);
        assert_eq!(
            parse_stop_control(&format!("handoff:{FIRST}")).unwrap(),
            StopControl::Handoff(FIRST.into())
        );
        assert_eq!(
            parse_stop_control("handoff:short").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn superseding_control_is_restored_for_other_waiters() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("daemon.stop");
        state::write_control(&path, "stop").unwrap();
        let claimed = claim_stop_at(&path).unwrap().unwrap();
        assert_eq!(claimed.control, StopControl::Manual);
        claimed.restore().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "stop");
    }

    #[test]
    fn restore_never_replaces_a_later_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("daemon.stop");
        state::write_control(&path, &format!("handoff:{FIRST}")).unwrap();
        let claimed = claim_stop_at(&path).unwrap().unwrap();
        state::write_control(&path, &format!("handoff:{SECOND}")).unwrap();

        claimed.restore().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            format!("handoff:{SECOND}")
        );
    }

    #[test]
    fn queued_child_exits_after_a_successor_publishes() {
        let own = StopControl::Handoff(FIRST.into());
        let other = StopControl::Handoff(SECOND.into());
        assert!(should_continue_waiting(Some(&own), None, FIRST, None));
        assert!(!should_continue_waiting(Some(&other), None, FIRST, None));
        assert!(should_continue_waiting(
            None,
            Some(SECOND),
            FIRST,
            Some(SECOND)
        ));
        assert!(!should_continue_waiting(None, Some(SECOND), FIRST, None));
        assert!(!should_continue_waiting(
            None,
            Some(SECOND),
            FIRST,
            Some(FIRST)
        ));
    }
}
