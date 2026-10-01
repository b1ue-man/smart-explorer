use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use super::{default_home, log, ShareHostState};

/// Shortest distance between periodic reloads (desktop: every reload).
const RELOAD_INTERVAL: Duration = Duration::from_secs(5);
/// Embedded worker: longest distance between periodic reloads.
const RELOAD_SAFETY_NET: Duration = Duration::from_secs(60);

/// Why the embedded worker may skip a periodic reload. On Android every
/// in-process writer of a Share input reloads explicitly (facade
/// `reconfigure`/`RefreshShare`) or updates the host state itself (event
/// drain, exec grants), so the periodic reload is a safety net: it runs when
/// the inputs changed, while a state waits for another attempt, and at least
/// once a minute. The desktop worker keeps its 5 s reload.
#[derive(Debug, Default)]
pub(super) struct ReloadGate {
    /// Input fingerprint taken right before the last reload read the inputs.
    inputs: Option<u64>,
    /// When unchanged inputs were last confirmed without a reload.
    checked_at: Option<Instant>,
    /// The last reload failed.
    failed: bool,
    /// The last reload ran while a reciprocal repair held the configuration.
    repair_deferred: bool,
    /// The last configuration asked for a running service.
    service_wanted: bool,
}

impl ReloadGate {
    pub(super) fn reloaded(&mut self, inputs: Option<u64>, failed: bool) {
        self.inputs = inputs;
        self.checked_at = None;
        self.failed = failed;
    }

    pub(super) fn set_repair_deferred(&mut self, deferred: bool) {
        self.repair_deferred = deferred;
    }

    /// Records a fingerprint check; true when the inputs changed.
    pub(super) fn inputs_changed(&mut self, current: u64, now: Instant) -> bool {
        self.checked_at = Some(now);
        self.inputs != Some(current)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReloadDecision {
    Wait,
    Reload,
    /// Reload only if `input_fingerprint` differs from the last reload's.
    IfInputsChanged,
}

pub(super) fn periodic_reload_decision(
    state: &ShareHostState,
    embedded: bool,
    now: Instant,
) -> ReloadDecision {
    decide_reload(
        now.saturating_duration_since(state.last_reload),
        state
            .reload_gate
            .checked_at
            .map(|at| now.saturating_duration_since(at)),
        embedded,
        embedded && needs_attempt(state),
    )
}

fn decide_reload(
    since_reload: Duration,
    since_check: Option<Duration>,
    embedded: bool,
    needs_attempt: bool,
) -> ReloadDecision {
    if since_reload < RELOAD_INTERVAL {
        return ReloadDecision::Wait;
    }
    if !embedded || needs_attempt || since_reload >= RELOAD_SAFETY_NET {
        return ReloadDecision::Reload;
    }
    if since_check.is_some_and(|since| since < RELOAD_INTERVAL) {
        return ReloadDecision::Wait;
    }
    ReloadDecision::IfInputsChanged
}

/// A failed, waiting or retrying state that only another reload resolves.
fn needs_attempt(state: &ShareHostState) -> bool {
    let gate = &state.reload_gate;
    gate.failed
        || gate.repair_deferred
        || (gate.service_wanted && state.service.is_none())
        || state
            .service
            .as_ref()
            .is_some_and(crate::share::ShareService::reciprocal_repair_in_flight)
        || state.pending_profiles_base.is_some()
        || state.identity_error.is_some()
        || state.profiles_error.is_some()
        || state.exec_retry.is_some()
        // Legacy requests expire over time; the reload refreshes them.
        || !state.profiles.legacy_direct_requests.is_empty()
}

/// Fingerprint of the Share inputs: the app data directory's direct entries
/// (name, kind, size, modification time). `share_server.txt` is one of them
/// and the exec-grant journal lives in its `sync` folder, whose time changes
/// with every atomic replacement there.
pub(super) fn input_fingerprint() -> u64 {
    directory_fingerprint(&crate::support_dirs::app_data_dir())
}

fn directory_fingerprint(directory: &std::path::Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_entry(&mut hasher, directory);
    match std::fs::read_dir(directory) {
        Ok(entries) => {
            let mut names: Vec<std::ffi::OsString> = entries
                .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
                .collect();
            names.sort();
            for name in names {
                name.hash(&mut hasher);
                hash_entry(&mut hasher, &directory.join(&name));
            }
        }
        Err(error) => error.kind().hash(&mut hasher),
    }
    hasher.finish()
}

fn hash_entry(hasher: &mut impl Hasher, path: &std::path::Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            metadata.is_dir().hash(hasher);
            metadata.len().hash(hasher);
            metadata.modified().ok().hash(hasher);
        }
        Err(error) => error.kind().hash(hasher),
    }
}

pub(super) fn configure_or_restart_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(error) = &state.identity_error {
        return Err(format!("Share-Identitaet nicht verfuegbar: {error}"));
    }
    if let Some(error) = &state.profiles_error {
        return Err(format!("Share-Profile nicht verfuegbar: {error}"));
    }
    let identity = state
        .identity
        .clone()
        .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
    let lan_presence = crate::share::LanSettings::load()
        .map(|settings| settings.presence_enabled)
        .unwrap_or(false);
    let has_direct_peers = state
        .profiles
        .direct_contacts
        .iter()
        .any(|contact| contact.access_state == crate::share::DirectAccessState::Accepted);
    let wanted = share_service_requested(
        state.suspended,
        &state.server,
        state.profiles.auto_connect,
        lan_presence && has_direct_peers,
    );
    state.reload_gate.service_wanted = wanted;
    if !wanted {
        if let Some(service) = state.service.take() {
            super::end_tracked_offers(state);
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        return Ok(());
    }
    if state
        .service
        .as_ref()
        .is_some_and(crate::share::ShareService::reciprocal_repair_in_flight)
    {
        return Ok(());
    }
    let needs_restart = state
        .service
        .as_ref()
        .map(|service| {
            service.identity.node_id != identity.node_id
                || service.identity.device_id != identity.device_id
                || service.identity.device_name != identity.device_name
                || service.identity.direct_lookup_id != identity.direct_lookup_id
                || service.identity.direct_secret() != identity.direct_secret()
                || state.running_server != state.server
        })
        .unwrap_or(true);
    if needs_restart {
        if let Some(service) = state.service.take() {
            super::end_tracked_offers(state);
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        match crate::share::ShareService::start_with_profile_home(
            state.server.clone(),
            identity,
            state.profiles.clone(),
            Some(default_home()),
        ) {
            Ok(service) => {
                log("share worker started");
                configure_service(&service, &state.profiles)?;
                state.running_server = state.server.clone();
                state.service = Some(service);
            }
            Err(error) => return Err(format!("Share-Worker Start: {error}")),
        }
    } else if let Some(service) = &state.service {
        configure_service(service, &state.profiles)?;
    }
    Ok(())
}

/// The service runs for a configured server, or without one when local
/// presence can still reach accepted Direct peers.
fn share_service_requested(
    suspended: bool,
    server: &str,
    auto_connect: bool,
    lan_only_possible: bool,
) -> bool {
    !suspended && auto_connect && (!server.trim().is_empty() || lan_only_possible)
}

pub(in crate::daemon) fn stop_service_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(service) = state.service.take() {
        super::end_tracked_offers(state);
        service.cmd(crate::share::ShareCmd::Stop)?;
    }
    state.running_server.clear();
    state.signal_connected = false;
    Ok(())
}

pub(in crate::daemon) fn configure_service(
    service: &crate::share::ShareService,
    profiles: &crate::share::ShareProfiles,
) -> Result<(), String> {
    if service.reciprocal_repair_in_flight() {
        return Ok(());
    }
    service
        .cmd(crate::share::ShareCmd::ConfigureProfiles {
            profiles: Box::new(profiles.clone()),
        })
        .map(|_| ())
}

pub(in crate::daemon) fn reload_committed_profiles(
    state: &mut ShareHostState,
    previous: &crate::share::ShareProfiles,
    preserve_worker_updates: bool,
) -> Result<(), String> {
    let mut canonical = crate::share::ShareProfiles::load_checked(Some(default_home()))?;
    if preserve_worker_updates {
        super::profile_merge::merge_worker_updates(&mut canonical, previous, &state.profiles);
    }
    state.profiles = canonical;
    state.profiles_error = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        decide_reload, directory_fingerprint, needs_attempt, share_service_requested,
        ReloadDecision, ShareHostState,
    };
    use std::time::Duration;

    const fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn android_background_task_reload_gate_keeps_desktop_cadence() {
        assert_eq!(
            decide_reload(secs(3), None, false, false),
            ReloadDecision::Wait
        );
        assert_eq!(
            decide_reload(secs(5), Some(secs(1)), false, false),
            ReloadDecision::Reload
        );
    }

    #[test]
    fn android_background_task_reload_gate_embedded_needs_change_or_safety_net() {
        assert_eq!(
            decide_reload(secs(4), None, true, true),
            ReloadDecision::Wait
        );
        assert_eq!(
            decide_reload(secs(6), None, true, false),
            ReloadDecision::IfInputsChanged
        );
        // Unchanged inputs were confirmed moments ago: no new check yet.
        assert_eq!(
            decide_reload(secs(20), Some(secs(2)), true, false),
            ReloadDecision::Wait
        );
        assert_eq!(
            decide_reload(secs(20), Some(secs(5)), true, false),
            ReloadDecision::IfInputsChanged
        );
        assert_eq!(
            decide_reload(secs(60), Some(secs(1)), true, false),
            ReloadDecision::Reload
        );
        assert_eq!(
            decide_reload(secs(6), Some(secs(1)), true, true),
            ReloadDecision::Reload
        );
    }

    #[test]
    fn android_background_task_reload_gate_retries_failed_and_waiting_states() {
        let mut state = ShareHostState::new();
        assert!(!needs_attempt(&state));
        state.reload_gate.reloaded(Some(1), true);
        assert!(needs_attempt(&state));
        state.reload_gate.reloaded(Some(1), false);
        assert!(!needs_attempt(&state));
        // The service should run but does not (a start failed).
        state.reload_gate.service_wanted = true;
        assert!(needs_attempt(&state));
        state.reload_gate.service_wanted = false;
        state.reload_gate.set_repair_deferred(true);
        assert!(needs_attempt(&state));
        state.reload_gate.set_repair_deferred(false);
        state.pending_profiles_base = Some(crate::share::ShareProfiles::default());
        assert!(needs_attempt(&state));
        state.pending_profiles_base = None;
        state.identity_error = Some("locked".into());
        assert!(needs_attempt(&state));
        state.identity_error = None;
        state.profiles_error = Some("unreadable".into());
        assert!(needs_attempt(&state));
        state.profiles_error = None;
        assert!(!needs_attempt(&state));

        let now = std::time::Instant::now();
        assert!(state.reload_gate.inputs_changed(2, now));
        state.reload_gate.reloaded(Some(2), false);
        assert!(!state.reload_gate.inputs_changed(2, now));
        assert!(state.reload_gate.inputs_changed(3, now));
    }

    #[test]
    fn android_background_task_input_fingerprint_follows_file_changes() {
        // Inode times have jiffy granularity; pause between observed changes.
        let pause = || std::thread::sleep(Duration::from_millis(30));
        let root = tempfile::tempdir().unwrap();
        let sync = root.path().join("sync");
        std::fs::create_dir(&sync).unwrap();
        std::fs::write(root.path().join("share_server.txt"), "a").unwrap();
        pause();
        let initial = directory_fingerprint(root.path());
        assert_eq!(directory_fingerprint(root.path()), initial);

        std::fs::write(root.path().join("share_server.txt"), "ab").unwrap();
        let rewritten = directory_fingerprint(root.path());
        assert_ne!(rewritten, initial);

        pause();
        let staged = sync.join(".journal.tmp");
        std::fs::write(&staged, "{}").unwrap();
        std::fs::rename(&staged, sync.join("exec-grant.journal")).unwrap();
        let journaled = directory_fingerprint(root.path());
        assert_ne!(journaled, rewritten);

        pause();
        std::fs::write(root.path().join("profiles.json"), "{}").unwrap();
        assert_ne!(directory_fingerprint(root.path()), journaled);
    }

    #[test]
    fn explicit_stop_barrier_blocks_periodic_auto_connect_reload() {
        assert!(share_service_requested(false, "127.0.0.1:9", true, false));
        assert!(!share_service_requested(true, "127.0.0.1:9", true, false));
        assert!(!share_service_requested(false, "", true, false));
        assert!(!share_service_requested(false, "127.0.0.1:9", false, false));
    }

    #[test]
    fn lan_cleanup_task_lan_only_operation_needs_no_server() {
        assert!(share_service_requested(false, "", true, true));
        assert!(!share_service_requested(true, "", true, true));
        assert!(!share_service_requested(false, "", false, true));
    }
}
