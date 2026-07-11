const WORKER_UNREACHABLE_PREFIX: &str = "Share-Worker nicht erreichbar:";
const POLL_START_FAILED_PREFIX: &str = "Share-Worker Poll konnte nicht starten:";

/// Reconciles a stale UI polling error after authenticated daemon IPC succeeds.
/// Domain statuses are left to the Share event stream and remain unchanged.
pub(super) fn after_successful_snapshot(
    current: &str,
    running: bool,
    connected: bool,
) -> Option<&'static str> {
    if !current.starts_with(WORKER_UNREACHABLE_PREFIX)
        && !current.starts_with(POLL_START_FAILED_PREFIX)
    {
        return None;
    }
    Some(if connected {
        "Share-Server verbunden"
    } else if running {
        "Share-Worker erreichbar (Share-Server nicht verbunden)"
    } else {
        "Share-Worker erreichbar (inaktiv)"
    })
}

#[cfg(test)]
mod tests {
    use super::after_successful_snapshot;

    #[test]
    fn connected_snapshot_replaces_worker_unreachable_status() {
        assert_eq!(
            after_successful_snapshot(
                "Share-Worker nicht erreichbar: connection refused",
                true,
                true,
            ),
            Some("Share-Server verbunden")
        );
    }

    #[test]
    fn running_snapshot_replaces_poll_start_failure() {
        assert_eq!(
            after_successful_snapshot(
                "Share-Worker Poll konnte nicht starten: thread limit",
                true,
                false,
            ),
            Some("Share-Worker erreichbar (Share-Server nicht verbunden)")
        );
    }

    #[test]
    fn inactive_snapshot_still_reports_reachable_daemon() {
        assert_eq!(
            after_successful_snapshot(
                "Share-Worker nicht erreichbar: stale endpoint",
                false,
                false,
            ),
            Some("Share-Worker erreichbar (inaktiv)")
        );
    }

    #[test]
    fn unrelated_and_domain_errors_are_preserved() {
        for status in [
            "Fehler: Iroh identity conflict",
            "Share-Server nicht erreichbar: DNS lookup failed",
            "Share-Worker konnte nicht aktiviert werden: profile invalid",
            "Share-Identitaet nicht verfuegbar: secret missing",
        ] {
            assert_eq!(after_successful_snapshot(status, true, true), None);
        }
    }
}
