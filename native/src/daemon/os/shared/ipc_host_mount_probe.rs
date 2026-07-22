use std::time::{Duration, Instant};

use super::ShareHost;

impl ShareHost {
    /// Ask the actual remote PeerBackend for one root's mount guarantees.
    /// This is deliberately a no-lease inspection: only the mount host may
    /// acquire and retain a server-side lease through its PeerBackend.
    pub(crate) fn probe_share_mount_capabilities(
        &self,
        target: crate::share::PeerOpenTarget,
        root: &str,
    ) -> Result<crate::vfs::MountPathCapabilities, String> {
        let root = crate::mount::BackendRoot::parse(root)
            .map_err(|error| format!("Peer-Laufwerkswurzel: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(45);
        self.reload_now()?;
        self.drain_events();
        let service = {
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker gesperrt".to_string())?;
            state.service.clone()
        };
        let Some(service) = service else {
            return Err("Share-Server ist nicht konfiguriert oder Auto-Connect ist aus".into());
        };
        service.cmd(crate::share::ShareCmd::Refresh)?;
        loop {
            if Instant::now() >= deadline {
                return Err("Peer-Laufwerkspruefung hat das 45-Sekunden-Limit erreicht".into());
            }
            match service.probe_mount_capabilities_for_target_until(
                &target,
                root.as_str(),
                deadline,
            ) {
                Ok(capabilities) => return Ok(capabilities),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining <= Duration::from_secs(1) {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(750).min(remaining));
                }
            }
        }
    }
}
