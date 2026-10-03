//! Volume arrivals are bound to a configured local endpoint and volume
//! identity. A changed drive letter cannot authorize a different disk.
use std::collections::{HashSet, HashMap};
use crate::syncjobs::{ConnectMark, PendingKind, SyncJob, Trigger};

pub(super) struct Connections { seen: HashSet<String>, first: bool, owed: HashMap<String, String> }
impl Connections {
    pub(super) fn new() -> Self { Self { seen: HashSet::new(), first: true, owed: HashMap::new() } }
    pub(super) fn poll(&mut self, jobs: &[SyncJob], now: i64) {
        let Some(snapshot) = super::platform::drive_snapshot() else { return; };
        let drives: HashSet<String> = snapshot.into_iter().filter_map(|drive|
            serde_json::to_string(&(drive.letter, drive.label, drive.serial)).ok()).collect();
        let session = super::platform::session_marker();
        for descriptor in drives.difference(&self.seen) {
            for job in jobs.iter().filter(|job| job.enabled && job.trigger == Trigger::OnConnect) {
                if !matches(job, descriptor) { continue; }
                if self.first && crate::syncjobs::load_job_state(&job.id).ok().is_some_and(|state|
                    state.last_connect.is_some_and(|mark| mark.volume == *descriptor && mark.session == session)) { continue; }
                self.owed.insert(job.id.clone(), descriptor.clone());
            }
        }
        self.owed.retain(|id, volume| {
            if !jobs.iter().any(|job| &job.id == id && job.enabled) { return false; }
            let mut waits_for_confirmation = false;
            match crate::syncjobs::update_job_state(id, |state| {
                if state.pending_trigger.as_ref().is_some_and(|pending| pending.kind == PendingKind::Confirmed) {
                    // A confirmation belongs to its original volume. A later
                    // arrival cannot silently change that authorization.
                    waits_for_confirmation = true;
                    return;
                } else {
                    let since = state.running.as_ref().map_or(now, |mark| mark.started.saturating_add(1).max(now));
                    state.pending_trigger = Some(crate::syncjobs::PendingTrigger {
                        kind: PendingKind::Connect, since, volume: Some(volume.clone()),
                    });
                }
                state.last_connect = Some(ConnectMark { volume: volume.clone(), seen: now, session: session.clone() });
            }) {
                Ok(_) => waits_for_confirmation,
                Err(error) => { super::state::log(&format!("volume trigger {id}: {error}")); true }
            }
        });
        self.seen = drives; self.first = false;
    }
}
pub(super) fn matches(job: &SyncJob, descriptor: &str) -> bool {
    if !super::schedule::drive_matches(&job.connect_match, descriptor) { return false; }
    let Some(root) = super::schedule::drive_root(descriptor) else { return false; };
    let root = super::platform::normalize_local_backend_path(&root);
    [&job.source, &job.target].iter().any(|endpoint| {
        let Ok(Some(path)) = crate::connect::local_endpoint_path(endpoint) else { return false; };
        let path = super::platform::normalize_local_backend_path(&path);
        if super::platform::watch_case_fold() {
            let root = root.to_lowercase(); let path = path.to_lowercase();
            path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
        } else { std::path::Path::new(path.as_ref()).starts_with(root.as_ref()) }
    })
}
pub(super) fn still_present(job: &SyncJob, volume: &str) -> bool {
    super::schedule::current_drives().contains(volume) && matches(job, volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_connect_matches_the_endpoint_on_the_arrived_volume() {
        let job = SyncJob::new("x".into(), "/media/a/project".into(), "sftp://peer/x".into());
        assert!(matches(&job, "/media/a|Disk|UUID-A"));
        assert!(!matches(&job, "/media/ab|Disk|UUID-B"));
        let remote = SyncJob::new("x".into(), "sftp://peer/media/a".into(), "drive://account/x".into());
        assert!(!matches(&remote, "/media/a|Disk|UUID-A"));
    }
}
