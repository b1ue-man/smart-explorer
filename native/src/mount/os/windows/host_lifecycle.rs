//! Owner-loop notification and ordered filesystem teardown.
use super::*;

pub(super) fn run_until_stopped(
    filesystem: DokanyFileSystem,
    storage: &CallbackStorage,
    session: &MountHostSession,
) -> Result<(), String> {
    let drive = match storage.context.selected_drive() {
        Ok(drive) => drive,
        Err(_) => {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive lost its letter state"),
            );
        }
    };
    let initial_status = match storage.context.engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => MountStatus::Mounted { drive },
        Ok(entries) => {
            let (path, condition) = &entries[0];
            let detail = match condition {
                EntryCondition::Conflict(conflict) => conflict.detail.clone(),
                EntryCondition::Dirty => {
                    "a cached change still requires safe remote recovery".into()
                }
                EntryCondition::Clean => "mounted-drive recovery is incomplete".into(),
            };
            MountStatus::Conflict {
                drive,
                path: path.clone(),
                detail,
            }
        }
        Err(_) => {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive could not inspect its recovery state"),
            );
        }
    };
    if session.report_status(initial_status).is_err() {
        return close_and_finalize(
            filesystem,
            storage,
            session,
            Some("the mounted drive could not report its running state"),
        );
    }
    let mut notifications = super::super::notifications::HostNotifications::default();
    let mut notification_error_reported = false;
    loop {
        if session.wait_for_stop_timeout(CONTROL_POLL) {
            return close_and_finalize(filesystem, storage, session, None);
        }
        if storage.context.stop_requested() {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive stopped after a callback or control failure"),
            );
        }
        match filesystem.wait(0) {
            DokanyWaitOutcome::Timeout if filesystem.is_running() => {}
            DokanyWaitOutcome::Closed | DokanyWaitOutcome::Timeout => {
                return close_and_finalize(filesystem, storage, session,
                    Some("the mounted drive closed without a requested unmount"));
            }
            DokanyWaitOutcome::Failed { .. } | DokanyWaitOutcome::Unexpected(_) => {
                return close_and_finalize(
                    filesystem,
                    storage,
                    session,
                    Some("Dokany stopped the mounted drive unexpectedly"),
                );
            }
        }
        match notifications.deliver(&storage.context.engine, &filesystem, drive) {
            Ok(()) => notification_error_reported = false,
            Err(error) if !notification_error_reported => {
                use std::io::Write;
                let _ = writeln!(std::io::stderr().lock(), "mount change delivery deferred: {error}");
                notification_error_reported = true;
            }
            Err(_) => {}
        }
    }
}

pub(super) fn close_and_finalize(
    filesystem: DokanyFileSystem,
    storage: &CallbackStorage,
    session: &MountHostSession,
    prior_failure: Option<&str>,
) -> Result<(), String> {
    storage.request_metadata_refresh_stop();
    let shutdown_watchdog = super::super::shutdown_watchdog::ShutdownWatchdog::start("dokan-close");
    // DokanCloseHandle is the callback lifetime boundary. Only inspect the
    // engine once Dokany can no longer mutate its retryable journal state.
    filesystem.close();
    // Closing Dokany first prevents a slow background target from keeping the
    // drive visible. The worker observes cancellation between remote targets;
    // join still owns its engine before recovery inspection begins.
    if let Some(watchdog) = shutdown_watchdog.as_ref() {
        watchdog.set_phase("metadata-refresh");
    }
    storage.join_metadata_refresh();
    let watchdog_failed = shutdown_watchdog.is_some_and(|watchdog| !watchdog.finish());
    let prior_failure = prior_failure.or(watchdog_failed
        .then_some("the mounted drive shutdown watchdog stopped after an internal failure"));
    match storage.context.engine.dirty_entries() {
        Ok(entries) if !entries.is_empty() => {
            let detail = recovery_detail(&entries);
            Err(report_failure_with_recovery(
                session,
                &detail,
                MountRecovery::Required,
            ))
        }
        Err(_) => Err(report_failure_with_recovery(
            session,
            "the mounted drive closed, but its recovery journal could not be inspected; keep the mount and use Retry",
            MountRecovery::Unknown,
        )),
        Ok(_) => match prior_failure {
            Some(detail) => Err(report_failure_with_recovery(
                session,
                detail,
                MountRecovery::Clean,
            )),
            None => session
                .report_status_with_recovery(MountStatus::Unmounted, Some(MountRecovery::Clean))
                .map_err(|_| "the mounted drive closed, but its final status could not be recorded".to_string()),
        },
    }
}
