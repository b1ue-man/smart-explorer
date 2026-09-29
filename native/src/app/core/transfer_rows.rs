//! Texts of the transfer list, the status chips and the completion notices,
//! computed from the progress values alone so they can be checked without a
//! window.
use super::transfer_center::{ExternalEntry, FinishedEntry};
use crate::format::format_bytes;
use crate::transfer::{ExternalSnapshot, TransferJob, TransferKind, TransferProgress};

pub(in crate::app) fn kind_icon(kind: TransferKind) -> &'static str {
    match kind {
        TransferKind::Upload => "⬆",
        TransferKind::Download => "⬇",
        TransferKind::RemoteCopy => "⇄",
        TransferKind::Local | TransferKind::Move => "📋",
    }
}

/// "Quelle → Ziel"; the job's labels until the engine reports its own.
pub(in crate::app) fn transfer_title(
    progress: &TransferProgress,
    job: Option<&TransferJob>,
) -> String {
    let pick = |reported: &str, planned: Option<&str>| {
        if reported.trim().is_empty() {
            planned.unwrap_or_default().trim().to_string()
        } else {
            reported.trim().to_string()
        }
    };
    let source = pick(
        progress.source.as_str(),
        job.map(|job| job.source_label.as_str()),
    );
    let target = pick(
        progress.target.as_str(),
        job.map(|job| job.target_label.as_str()),
    );
    match (source.is_empty(), target.is_empty()) {
        (false, false) => format!("{source} → {target}"),
        (true, false) => format!("→ {target}"),
        (false, true) => source,
        (true, true) if progress.label.trim().is_empty() => progress.kind.label().to_string(),
        (true, true) => progress.label.clone(),
    }
}

/// State of a running transfer.
pub(in crate::app) fn running_state(progress: &TransferProgress, canceling: bool) -> String {
    if canceling {
        "wird abgebrochen…".to_string()
    } else if progress.discovering {
        format!("sucht… {} gefunden", group_digits(progress.files_total))
    } else if progress.parallel == 0 && progress.files_done == 0 && progress.bytes_done == 0 {
        "wartet".to_string()
    } else {
        "läuft".to_string()
    }
}

/// "1.234 / 5.678 Dateien · 1.20 GB / 3.40 GB"
pub(in crate::app) fn counts_line(progress: &TransferProgress) -> String {
    let files = if progress.files_total > 0 {
        format!(
            "{} / {} Dateien",
            group_digits(progress.files_done),
            group_digits(progress.files_total)
        )
    } else {
        format!("{} Dateien", group_digits(progress.files_done))
    };
    let bytes = if progress.bytes_total > 0 {
        format!(
            "{} / {}",
            format_bytes(progress.bytes_done),
            format_bytes(progress.bytes_total)
        )
    } else {
        format_bytes(progress.bytes_done)
    };
    format!("{files} · {bytes}")
}

/// Remaining seconds at the current rate; unknown while files are still
/// being found (the totals keep growing) or nothing moves.
pub(in crate::app) fn eta_secs(progress: &TransferProgress) -> Option<u64> {
    if progress.discovering || progress.rate_bps == 0 || progress.bytes_total <= progress.bytes_done
    {
        return None;
    }
    Some((progress.bytes_total - progress.bytes_done).div_ceil(progress.rate_bps))
}

/// "12.3 MB/s · noch 1:23": the rate over the last seconds (reported by the
/// engine), the remaining time once every file was found.
pub(in crate::app) fn rate_line(progress: &TransferProgress) -> Option<String> {
    if progress.rate_bps == 0 {
        return None;
    }
    let rate = format!("{}/s", format_bytes(progress.rate_bps));
    Some(match eta_secs(progress) {
        Some(secs) => format!("{rate} · noch {}", format_duration(secs)),
        None => rate,
    })
}

/// "läuft: a.txt, b.txt · 8 parallel"
pub(in crate::app) fn activity_line(progress: &TransferProgress) -> Option<String> {
    let names: Vec<&str> = progress
        .active
        .iter()
        .take(3)
        .map(|path| file_name(path))
        .filter(|name| !name.is_empty())
        .collect();
    let parallel = (progress.parallel > 1).then(|| format!("{} parallel", progress.parallel));
    match (names.is_empty(), parallel) {
        (true, None) => None,
        (true, Some(parallel)) => Some(parallel),
        (false, None) => Some(format!("läuft: {}", names.join(", "))),
        (false, Some(parallel)) => Some(format!("läuft: {} · {parallel}", names.join(", "))),
    }
}

/// "übersprungen: 3 · ausgelassen: 1 · Fehler: 2"
pub(in crate::app) fn tally_line(progress: &TransferProgress) -> Option<String> {
    let parts: Vec<String> = [
        ("übersprungen", progress.skipped),
        ("ausgelassen", progress.omitted),
        ("Fehler", progress.errors),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(label, count)| format!("{label}: {}", group_digits(count)))
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(in crate::app) fn finished_state(entry: &FinishedEntry) -> &'static str {
    if entry.failure.is_some() {
        "fehlgeschlagen"
    } else if entry.canceled {
        "abgebrochen"
    } else if issue_count(entry) > 0 {
        "mit Fehlern"
    } else {
        "fertig"
    }
}

/// Errors of a finished transfer: the engine's count, at least the listed ones.
pub(in crate::app) fn issue_count(entry: &FinishedEntry) -> u64 {
    let listed = entry.issues.len().max(entry.errors.len()) as u64;
    entry.progress.errors.max(listed) + u64::from(entry.failure.is_some())
}

/// The message when a transfer ends (Spec B).
pub(in crate::app) fn completion_notice(entry: &FinishedEntry) -> String {
    let progress = &entry.progress;
    if let Some(failure) = &entry.failure {
        return format!("⚠ Übertragung fehlgeschlagen: {failure}");
    }
    let amount = format!(
        "{} Datei(en) ({})",
        group_digits(progress.files_done),
        format_bytes(progress.bytes_done)
    );
    let skipped = if progress.skipped > 0 {
        format!(", {} übersprungen", group_digits(progress.skipped))
    } else {
        String::new()
    };
    if entry.canceled {
        return format!(
            "⚠ Übertragung abgebrochen · {amount} übertragen{skipped} – „Fehlende übertragen“ unter Übertragungen"
        );
    }
    let took = format_duration(progress.elapsed_ms / 1000);
    match issue_count(entry) {
        0 => format!("✓ {amount} übertragen in {took}{skipped}"),
        errors => format!(
            "⚠ {amount} übertragen in {took}{skipped} mit {} Fehler(n) – Details in Übertragungen",
            group_digits(errors)
        ),
    }
}

/// The whole error list of a finished transfer, for copying and the app's
/// error log (the log file holds every issue when there are more).
pub(in crate::app) fn issues_text(entry: &FinishedEntry) -> String {
    let mut lines = Vec::new();
    if let Some(failure) = &entry.failure {
        lines.push(failure.clone());
    }
    for issue in &entry.issues {
        if issue.path.is_empty() {
            lines.push(issue.message.clone());
        } else {
            lines.push(format!("{}: {}", issue.path, issue.message));
        }
    }
    if entry.issues.is_empty() {
        lines.extend(entry.errors.iter().cloned());
    }
    let listed = entry.issues.len().max(entry.errors.len()) as u64;
    if entry.progress.errors > listed {
        lines.push(format!(
            "… {} weitere Fehler (vollständig im Protokoll)",
            group_digits(entry.progress.errors - listed)
        ));
    }
    if let Some(log) = &entry.progress.log_path {
        lines.push(format!("Protokoll: {log}"));
    }
    lines.join("\n")
}

/// The entries another program could not receive, one per line, for copying
/// and the app's error log.
pub(in crate::app) fn external_issues_text(snapshot: &ExternalSnapshot) -> String {
    let mut lines: Vec<String> = snapshot
        .issues
        .iter()
        .map(|issue| {
            if issue.path.is_empty() {
                issue.message.clone()
            } else {
                format!("{}: {}", issue.path, issue.message)
            }
        })
        .collect();
    let listed = snapshot.issues.len() as u64;
    if snapshot.errors > listed {
        lines.push(format!(
            "… {} weitere Fehler",
            group_digits(snapshot.errors - listed)
        ));
    }
    lines.join("\n")
}

pub(in crate::app) fn external_state(entry: &ExternalEntry) -> &'static str {
    match (entry.snapshot.finished, entry.released) {
        (true, _) if entry.snapshot.errors > 0 => "mit Fehlern",
        (true, _) => "fertig",
        (false, true) => "beendet",
        (false, false) => "läuft",
    }
}

/// "12 / 40 Dateien · 1.20 GB · Fehler: 1"
pub(in crate::app) fn external_counts(snapshot: &ExternalSnapshot) -> String {
    let files = if snapshot.files_total > 0 {
        format!(
            "{} / {} Dateien",
            group_digits(snapshot.files_done),
            group_digits(snapshot.files_total)
        )
    } else {
        format!("{} Dateien", group_digits(snapshot.files_done))
    };
    let errors = if snapshot.errors > 0 {
        format!(" · Fehler: {}", group_digits(snapshot.errors))
    } else {
        String::new()
    };
    format!("{files} · {}{errors}", format_bytes(snapshot.bytes_done))
}

/// "1:23" below an hour, "1:02:03" above.
pub(in crate::app) fn format_duration(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// "1.234.567" (German digit grouping).
pub(in crate::app) fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    grouped
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

#[cfg(test)]
#[path = "transfer_rows_tests.rs"]
mod tests;
