//! What a running analysis shows. A local walk: "N Ordner · Ordner". A
//! remote one runs on the other device (`analytics::scan_remote`): the
//! phases of its worker (waiting for a worker, preparing, walking,
//! assembling), the transfer of the finished result with its own progress
//! bar, the check of the received result, and how long the other device has
//! been silent.
use std::time::Duration;

use crate::analytics::{thousands, walk_status, ScanPhase, ScanSnapshot};

/// A host report older than this is named in the status.
const QUIET_AFTER: Duration = Duration::from_secs(5);

/// `(done_bytes, total_bytes)` of the task: the result transfer while it
/// runs (a determinate bar), else the measured bytes without a total.
pub(super) fn task_bytes(snapshot: &ScanSnapshot) -> (u64, u64) {
    match snapshot.phase {
        ScanPhase::Transferring | ScanPhase::Verifying if snapshot.transfer_total > 0 => (
            snapshot.transferred.min(snapshot.transfer_total),
            snapshot.transfer_total,
        ),
        _ => (snapshot.bytes, 0),
    }
}

/// The task message: one line per fact (the app shows each line apart).
pub(super) fn status(remote: bool, snapshot: &ScanSnapshot, quiet: Option<Duration>) -> String {
    if !remote {
        return walk_status(snapshot.dirs, &snapshot.current);
    }
    let mut lines: Vec<String> = Vec::with_capacity(3);
    match snapshot.phase {
        ScanPhase::Preparing => lines.push("Verbindung zur Gegenstelle wird vorbereitet …".into()),
        ScanPhase::Queued => {
            lines.push("Wartet auf einen freien Analyse-Worker der Gegenstelle".into())
        }
        ScanPhase::Scanning => {
            lines.push(if snapshot.directories_unreported {
                "Gegenstelle durchsucht".to_string()
            } else {
                format!(
                    "Gegenstelle durchsucht · {} Ordner",
                    thousands(snapshot.dirs)
                )
            });
            push_current(&mut lines, &snapshot.current);
        }
        ScanPhase::Legacy => {
            lines.push("Älterer Analysepfad der Gegenstelle: nur Datei- und Bytezähler".into());
            push_current(&mut lines, &snapshot.current);
        }
        ScanPhase::Assembling => lines.push("Gegenstelle stellt das Ergebnis zusammen".into()),
        ScanPhase::Transferring => {
            lines.push("Ergebnis wird übertragen".into());
            if snapshot.transfer_total > 0 {
                let done = snapshot.transferred.min(snapshot.transfer_total);
                let percent = u128::from(done) * 100 / u128::from(snapshot.transfer_total);
                lines.push(format!(
                    "{} von {} · {percent} %",
                    size_text(done),
                    size_text(snapshot.transfer_total)
                ));
            }
        }
        ScanPhase::Verifying => lines.push("Empfangenes Ergebnis wird geprüft".into()),
    }
    if let Some(quiet) = quiet.filter(|quiet| *quiet >= QUIET_AFTER) {
        lines.push(format!(
            "Letzte Meldung der Gegenstelle vor {} s",
            thousands(quiet.as_secs())
        ));
    }
    lines.join("\n")
}

fn push_current(lines: &mut Vec<String>, current: &str) {
    if !current.is_empty() {
        lines.push(current.to_string());
    }
}

/// `12,3 MB` (German decimal comma).
fn size_text(bytes: u64) -> String {
    crate::format::format_bytes(bytes).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: ScanPhase) -> ScanSnapshot {
        ScanSnapshot {
            files: 10,
            dirs: 1234,
            bytes: 5000,
            phase,
            current: "/Daten/Fotos".into(),
            ..ScanSnapshot::default()
        }
    }

    #[test]
    fn review_task_remote_status_names_every_phase() {
        let local = status(false, &snapshot(ScanPhase::Scanning), None);
        assert_eq!(local, "1.234 Ordner · /Daten/Fotos");
        let scanning = status(true, &snapshot(ScanPhase::Scanning), None);
        assert_eq!(
            scanning,
            "Gegenstelle durchsucht · 1.234 Ordner\n/Daten/Fotos"
        );
        let queued = status(
            true,
            &snapshot(ScanPhase::Queued),
            Some(Duration::from_secs(7)),
        );
        assert_eq!(
            queued,
            "Wartet auf einen freien Analyse-Worker der Gegenstelle\nLetzte Meldung der Gegenstelle vor 7 s"
        );
        let mut transfer = snapshot(ScanPhase::Transferring);
        transfer.transferred = 512 * 1024;
        transfer.transfer_total = 2 * 1024 * 1024;
        assert_eq!(
            status(true, &transfer, Some(Duration::from_secs(1))),
            "Ergebnis wird übertragen\n512 KB von 2,00 MB · 25 %"
        );
        assert_eq!(task_bytes(&transfer), (512 * 1024, 2 * 1024 * 1024));
        assert_eq!(task_bytes(&snapshot(ScanPhase::Scanning)), (5000, 0));
        let mut legacy = snapshot(ScanPhase::Legacy);
        legacy.directories_unreported = true;
        assert!(status(true, &legacy, None).starts_with("Älterer Analysepfad"));
        for phase in [
            ScanPhase::Preparing,
            ScanPhase::Assembling,
            ScanPhase::Verifying,
        ] {
            assert!(!status(true, &snapshot(phase), None).is_empty());
        }
    }
}
