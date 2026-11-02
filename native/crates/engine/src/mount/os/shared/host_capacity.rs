//! Copies cached provider metadata; never waits on a provider query.
use super::HostControl;
use crate::mount::{CacheSpaceProbe, MountCapacityReport, MountFilesystem};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Reporter {
    last: Option<Instant>,
}
impl Reporter {
    pub(super) fn turn(
        &mut self,
        control: &dyn HostControl,
        engine: &dyn MountFilesystem,
        space: &dyn CacheSpaceProbe,
    ) {
        if self
            .last
            .is_some_and(|last| last.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.last = Some(Instant::now());
        let mut remote = engine.capacity_snapshot();
        remote.read_only |= engine.volume_facts().read_only;
        let budget = space
            .available_bytes()
            .map(|bytes| engine.cache_capacity_budget(bytes))
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::PermissionDenied => crate::vfs::CapacityFailure::Authorization,
                _ => crate::vfs::CapacityFailure::Temporary,
            });
        let mut report = MountCapacityReport::from_observation(remote, budget);
        // G4 ML1: pending background uploads travel with this report.
        report.uploads = engine.upload_status();
        // Missing statistics must never stop a drive or alter recovery ownership.
        let _ = control.report_capacity(report);
    }
}
