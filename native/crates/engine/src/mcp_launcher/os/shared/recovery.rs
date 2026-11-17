//! Resumes an update transaction its owner left before Commit or Rollback:
//! the owner was ended, crashed, or its own rollback failed. The installed
//! regular files decide the direction (`core::recovery::resolve`): this
//! rolls back, reports a completion for the caller's start proof, or refuses
//! with each file's content. The failure of the chosen direction is the
//! error; a check of the other direction never replaces it (before this, a
//! failed rollback was reported as the `runtime_sha256_mismatch` of the
//! completion check against the still previous files).
#[cfg(all(test, any(windows, target_os = "linux")))]
#[path = "recovery_task_tests.rs"]
mod recovery_task_tests;
use crate::mcp_launcher::core::lifecycle::{Phase, UpdateState};
use crate::mcp_launcher::core::recovery::{found, refusal, resolve, FoundTarget, Resolution};
use crate::mcp_launcher::installation::handle_sha256;
use crate::mcp_launcher::{os, Installation};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long an updater waits for the process of an owner that already gave
/// up the owner lock: it writes its failure report and exits right after.
pub const OWNER_EXIT_WAIT: Duration = Duration::from_secs(30);

/// What `resume_interrupted_update` found and did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resumed {
    /// No unfinished transaction.
    Nothing,
    /// Its owner holds it: another update runs right now.
    Running,
    /// The unfinished transaction was rolled back; a new one may start.
    RolledBack,
    /// The installed files are the target version: the caller proves that it
    /// starts and then completes it with `recover_update(root, true)`.
    CompletionPending,
}

/// An unfinished transaction and the direction its files allow.
#[derive(Clone, Debug)]
pub struct InterruptedUpdate {
    pub state: UpdateState,
    pub resolution: Resolution,
    /// Its owner holds it: another update runs right now.
    pub running: bool,
}

/// The unfinished transaction of the installation at `root`, without
/// changing anything (`se update --check`, the app's update page).
pub fn inspect_interrupted_update(root: &Path) -> Result<Option<InterruptedUpdate>, String> {
    let installation = Installation::load(root)?;
    let Some(state) = installation.state()? else {
        return Ok(None);
    };
    if !unfinished(&installation, &state) {
        return Ok(None);
    }
    let running = installation.owner_alive(&state);
    let resolution = resolve(&state, &observe(&installation.root, &state)?);
    Ok(Some(InterruptedUpdate {
        state,
        resolution,
        running,
    }))
}

/// Brings an unfinished transaction to an end before a new update starts:
/// previous files roll back (the checked `recover_update(root, false)`),
/// target files are reported for completion, anything else is refused. An
/// owner process that gave up the owner lock gets `owner_wait` to exit.
pub fn resume_interrupted_update(root: &Path, owner_wait: Duration) -> Result<Resumed, String> {
    let installation = Installation::load(root)?;
    let Some(state) = installation.state()? else {
        return Ok(Resumed::Nothing);
    };
    if !unfinished(&installation, &state) {
        return Ok(Resumed::Nothing);
    }
    if installation.owner_alive(&state) {
        return Ok(Resumed::Running);
    }
    wait_for_owner_exit(&state, owner_wait)?;
    match resolve(&state, &observe(&installation.root, &state)?) {
        // recover_update proves every file again under the owner and control locks.
        Resolution::RollBack => crate::mcp_launcher::recover_update(root, false)
            .map(|()| Resumed::RolledBack)
            .map_err(|error| format!("update_rollback_failed: {error}")),
        Resolution::Complete => Ok(Resumed::CompletionPending),
        Resolution::Refuse(entries) => Err(refusal(&state, &entries)),
    }
}

/// For `Resumed::CompletionPending` in an update to another release than the
/// recorded target (`cli_sha256` differs): the installed files are that
/// target, its own runtime proves itself (contract probe) and the
/// transaction completes here, so this update starts from it as usual;
/// `Ok(true)`. `Ok(false)`: the same release or a retirement, whose
/// completion the caller proves by starting it (as before).
pub fn complete_other_release(root: &Path, cli_sha256: &str) -> Result<bool, String> {
    let installation = Installation::load(root)?;
    let state = installation
        .state()?
        .ok_or("update_recovery_state_missing")?;
    if state.retire_managed || state.target.runtime_sha256 == cli_sha256 {
        return Ok(false);
    }
    crate::mcp_launcher::verify_recovery_targets(root, true)?;
    crate::mcp_launcher::verify_installed_product(
        root,
        &installation.installed_cli(),
        &state.target.runtime_sha256,
        &state.target.product_version,
    )?;
    crate::mcp_launcher::recover_update(root, true)?;
    Ok(true)
}

/// The record blocks work (`Installation::status` "update_*"): a prepared or
/// stopping transaction, or a retirement whose pointer is still the previous.
fn unfinished(installation: &Installation, state: &UpdateState) -> bool {
    state.blocks_work()
        || (state.retire_managed
            && state.phase == Phase::Committed
            && installation.manifest == state.previous)
}

fn wait_for_owner_exit(state: &UpdateState, wait: Duration) -> Result<(), String> {
    let deadline = Instant::now() + wait;
    while os::process_matches(&state.owner) {
        if Instant::now() >= deadline {
            return Err(format!(
                "update_owner_still_alive: der Updater der unterbrochenen Aktualisierung auf \
                 v{} (PID {}, {}) läuft noch",
                state.target.product_version, state.owner.pid, state.owner.image
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Each recorded regular file with its current SHA-256. `state.validate`
/// allows only plain names, so every path stays directly in `root`.
fn observe(root: &Path, state: &UpdateState) -> Result<Vec<FoundTarget>, String> {
    state
        .installation_targets
        .iter()
        .map(|target| {
            let path = root.join(&target.file);
            let sha256 = match std::fs::File::open(&path) {
                Ok(mut file) => {
                    Some(handle_sha256(&mut file).map_err(|e| format!("{}: {e}", path.display()))?)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("{}: {error}", path.display())),
            };
            Ok(found(target, sha256))
        })
        .collect()
}
