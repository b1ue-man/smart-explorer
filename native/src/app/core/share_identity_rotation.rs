use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct WorkerState {
    daemon_running: bool,
    share_running: bool,
}

pub(super) fn rotate(app: &mut App) {
    let worker = stop_worker();
    let rotation = worker.map(|worker| {
        app.share_worker_running = false;
        let result = app
            .share_identity
            .as_mut()
            .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())
            .and_then(|identity| identity.regenerate_direct_code());
        (worker, result)
    });
    match rotation {
        Ok((worker, Ok(outcome))) => finish_success(app, worker, outcome),
        Ok((worker, Err(error))) => {
            let restore = restore_worker(worker);
            app.error_msg = Some(match restore {
                Ok(()) => format!("Direkt-Code nicht erneuert: {error}"),
                Err(restore) => format!(
                    "Direkt-Code nicht erneuert: {error}; Worker-Wiederherstellung: {restore}"
                ),
            });
        }
        Err(error) => {
            app.error_msg = Some(format!(
                "Direkt-Code nicht erneuert; Worker konnte nicht sicher gestoppt werden: {error}"
            ));
        }
    }
}

fn finish_success(app: &mut App, worker: WorkerState, outcome: crate::share::DirectCodeRotation) {
    app.share_regenerate_direct_confirm = false;
    let cleanup = app
        .share_identity
        .clone()
        .ok_or_else(|| "Share-Identitaet nach Rotation nicht verfuegbar".to_string())
        .and_then(|identity| {
            identity
                .complete_pending_cleanup(Some(dirs_home().to_string_lossy().replace('\\', "/")))
                .map(|profiles| {
                    app.share_profiles = profiles;
                    app.share_profiles_error = None;
                })
        });
    if let Err(error) = require_cleanup_before_restart(cleanup) {
        app.error_msg = Some(format!(
            "Direkt-Code erneuert, aber Share bleibt aus Sicherheitsgruenden gestoppt: {error}"
        ));
        return;
    }
    if let Err(error) = restore_worker(worker) {
        app.error_msg = Some(format!(
            "Direkt-Code erneuert, aber Share-Worker blieb gestoppt: {error}"
        ));
        return;
    }
    app.notice = Some((
        "Direkt-Code dauerhaft erneuert".into(),
        std::time::Instant::now(),
    ));
    if let Some(warning) = outcome.cleanup_warning {
        app.error_msg = Some(warning);
    }
}

pub(super) fn stop_worker() -> Result<WorkerState, String> {
    if !crate::daemon::is_running() {
        return Ok(WorkerState::default());
    }
    let snapshot = crate::daemon::drain_share_worker_events()
        .map_err(|error| format!("Share-Worker vor Code-Rotation pruefen: {error}"))?;
    let state = WorkerState {
        daemon_running: true,
        share_running: snapshot.running,
    };
    if !snapshot.running {
        return Ok(state);
    }
    crate::daemon::send_share_command(crate::share::ShareCmd::Stop)
        .map_err(|error| format!("Share-Worker vor Code-Rotation stoppen: {error}"))?;
    let stopped = crate::daemon::drain_share_worker_events()
        .map_err(|error| format!("Share-Worker-Stopp vor Code-Rotation pruefen: {error}"))?;
    if stopped.running {
        return Err("Share-Worker lief nach dem Stopp weiter".into());
    }
    Ok(state)
}

pub(super) fn restore_worker(state: WorkerState) -> Result<(), String> {
    if !state.daemon_running || !state.share_running {
        return Ok(());
    }
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => Ok(()),
        Ok(false) => Err("Share-Worker wurde nicht wieder aktiv".into()),
        Err(error) => Err(format!("Share-Worker neu laden: {error}")),
    }
}

pub(super) fn require_cleanup_before_restart(cleanup: Result<(), String>) -> Result<(), String> {
    cleanup.map_err(|error| format!("Legacy-Freigaben nicht sicher bereinigt: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleanup_failure_blocks_worker_restart_gate() {
        let result = super::require_cleanup_before_restart(Err("disk full".into()));
        assert!(result.unwrap_err().contains("nicht sicher bereinigt"));
    }
}
