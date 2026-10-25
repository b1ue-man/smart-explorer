//! Authenticated metadata only: no backend calls and no recovery transitions.
use super::MountManager;

impl MountManager {
    pub(in crate::daemon) fn update_capacity(
        &self,
        id: &crate::mount::MountId,
        session_token: &str,
        capacity: crate::mount::MountCapacityReport,
    ) -> Result<(), String> {
        capacity.validate().map_err(|error| error.to_string())?;
        let mut state = self.state_guard()?;
        let entry = super::host::authenticated_entry(&mut state, id, session_token)?;
        if matches!(entry.status, crate::mount::MountStatus::Unmounted) {
            return Err("Laufwerk-Host ist beendet".into());
        }
        entry.capacity = Some(capacity.with_freshness(crate::vfs::observed_now_ms()));
        Ok(())
    }
}
