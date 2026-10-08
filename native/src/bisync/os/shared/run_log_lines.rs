//! Text of the job log's comparison and action lines (German, one line per
//! entry). The planner stays untouched: comparisons are written after
//! planning from the plan, both sides and the baseline.
use std::collections::BTreeSet;

use super::completion::{CompletedAction, CompletedKind, DirAction};
use super::plan_types::PairPlan;
use super::run_log::RunLog;
use super::snapshot_types::SideSnapshot;
use super::types::{Action, Baseline, PairSide, Sig};

pub(super) fn side(side: PairSide) -> &'static str {
    match side {
        PairSide::A => "Quelle",
        PairSide::B => "Ziel",
    }
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn time(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| ms.to_string())
}

/// `12.3 KB, 2026-10-08 14:35:12` or `fehlt`.
pub(super) fn sig(sig: Option<Sig>) -> String {
    match sig {
        Some(sig) if sig.hash != 0 => format!(
            "{}, {}, Inhalt {:016x}",
            size(sig.size),
            time(sig.mtime_ms),
            sig.hash
        ),
        Some(sig) => format!("{}, {}", size(sig.size), time(sig.mtime_ms)),
        None => "fehlt".into(),
    }
}

fn action_parts(action: &Action) -> (&str, &'static str) {
    match action {
        Action::CopyAtoB(rel) => (rel, "Quelle → Ziel kopieren"),
        Action::CopyBtoA(rel) => (rel, "Ziel → Quelle kopieren"),
        Action::FinalizeMoveAtoB(rel) => (rel, "Verschieben abschließen (Quelle entfernen)"),
        Action::FinalizeMoveBtoA(rel) => (rel, "Verschieben abschließen (Ziel entfernen)"),
        Action::DeleteA(rel) => (rel, "in der Quelle löschen"),
        Action::DeleteB(rel) => (rel, "im Ziel löschen"),
        Action::KeepBothAtoB(rel) => (rel, "beide behalten, Quelle gewinnt"),
        Action::KeepBothBtoA(rel) => (rel, "beide behalten, Ziel gewinnt"),
    }
}

pub(super) fn completed_text(action: &CompletedAction) -> String {
    let what = match action.kind {
        CompletedKind::Copied { from } => format!("kopiert aus {}", side(from)),
        CompletedKind::Moved { from } => format!("verschoben aus {}", side(from)),
        CompletedKind::Deleted { side: at } => format!("gelöscht in {}", side(at)),
        CompletedKind::DirCreated { side: at } => format!("Ordner angelegt in {}", side(at)),
        CompletedKind::DirRemoved { side: at } => format!("leerer Ordner entfernt in {}", side(at)),
    };
    match action.dst_sig {
        Some(dst) => format!("{}: {what} ({})", action.rel, sig(Some(dst))),
        None => format!("{}: {what}", action.rel),
    }
}

fn compared(rel: &str, a: &SideSnapshot, b: &SideSnapshot, base: &Baseline) -> String {
    let (base_a, base_b) = base.get(rel).copied().unwrap_or((None, None));
    format!(
        "{rel}: Quelle {} · Ziel {} · letzter Stand Quelle {} / Ziel {}",
        sig(a.tree.get(rel).copied()),
        sig(b.tree.get(rel).copied()),
        sig(base_a),
        sig(base_b)
    )
}

/// One line per compared entry with its decision; unchanged entries one by
/// one only in verbose mode, otherwise as a count.
pub(super) fn plan_lines(
    log: &RunLog,
    plan: &PairPlan,
    a: &SideSnapshot,
    b: &SideSnapshot,
    base: &Baseline,
) {
    let mut decided = BTreeSet::new();
    for action in &plan.actions {
        let (rel, what) = action_parts(action);
        decided.insert(rel.to_string());
        log.line(
            "Vergleich",
            &format!("{} → {what}", compared(rel, a, b, base)),
        );
    }
    for conflict in &plan.conflicts {
        decided.insert(conflict.rel.clone());
        log.line(
            "Vergleich",
            &format!(
                "{} → Konflikt: beide Seiten geändert, nichts überschrieben",
                compared(&conflict.rel, a, b, base)
            ),
        );
    }
    for rel in &plan.verify {
        decided.insert(rel.clone());
        log.line(
            "Vergleich",
            &format!(
                "{} → gleiche Größe, andere Zeit: Inhalte werden gelesen",
                compared(rel, a, b, base)
            ),
        );
    }
    for (rel, _) in &plan.records {
        decided.insert(rel.clone());
        log.line(
            "Vergleich",
            &format!("{} → gleich, Stand vermerkt", compared(rel, a, b, base)),
        );
    }
    for rel in &plan.forget {
        decided.insert(rel.clone());
        log.line(
            "Vergleich",
            &format!("{rel}: auf beiden Seiten entfernt, Stand gelöscht"),
        );
    }
    for dir in &plan.dirs {
        let what = match dir {
            DirAction::Create { side: at, .. } => format!("Ordner in {} anlegen", side(*at)),
            DirAction::Remove { side: at, .. } => {
                format!("leeren Ordner in {} entfernen", side(*at))
            }
        };
        log.line("Vergleich", &format!("{}/ → {what}", dir.rel()));
    }
    let mut unchanged = 0u64;
    for rel in a.tree.keys().chain(b.tree.keys()) {
        if decided.insert(rel.clone()) {
            unchanged += 1;
            if log.verbose() {
                log.line(
                    "Vergleich",
                    &format!("{} → unverändert", compared(rel, a, b, base)),
                );
            }
        }
    }
    log.line(
        "Plan",
        &format!(
            "{} Aktionen, {} Konflikte, {} Ordneränderungen, {} unverändert verglichen",
            plan.actions.len(),
            plan.conflicts.len(),
            plan.dirs.len(),
            unchanged
        ),
    );
    if let Some(summary) = plan.omissions.summary() {
        log.line("Ausgelassen", &summary);
    }
}

/// The incremental mirror path: the actions planned from the change feed or
/// the stored index.
pub(super) fn incremental_lines(log: &RunLog, changes: usize, actions: &[Action]) {
    log.line(
        "Plan",
        &format!(
            "Inkrementeller Lauf: {changes} gemeldete Änderungen, {} Aktionen",
            actions.len()
        ),
    );
    for action in actions {
        let (rel, what) = action_parts(action);
        log.line("Vergleich", &format!("{rel}: geändert gemeldet → {what}"));
    }
}

/// First line of a run: both roots and the options that shape decisions.
pub(super) fn start_line(log: &RunLog, request: &super::orchestration::RunRequest<'_>) {
    let opts = &request.opts;
    log.line(
        if opts.dry_run { "Vorschau" } else { "Start" },
        &format!(
            "Quelle {} · Ziel {} · Richtung {:?} · Konflikte {:?} · Löschen {:?} · Vergleich {:?} · Tiefe {:?}{}",
            request.root_a,
            request.root_b,
            opts.direction,
            opts.conflict,
            opts.delete,
            opts.compare,
            request.settings.depth,
            if opts.dry_run { " · ändert nichts" } else { "" }
        ),
    );
}

/// Last lines of a run: result counts, every error, stop or block.
pub(super) fn outcome_lines(
    log: &RunLog,
    out: &super::orchestration::Outcome,
    elapsed: std::time::Duration,
) {
    for (path, error) in &out.errors {
        log.line("Fehler", &format!("{path}: {error}"));
    }
    for (rel, reason) in &out.deferred {
        log.line("Verschoben", &format!("{rel}: {reason}"));
    }
    if let Some(block) = &out.blocked {
        log.line(
            "Stopp",
            &format!("Sicherheitsstopp, wartet auf Bestätigung: {block:?}"),
        );
    }
    if let Some(stop) = &out.stopped {
        log.line("Gestoppt", &format!("Lauf vorzeitig beendet: {stop:?}"));
    }
    let status = if out.busy {
        "Paar belegt (anderer Lauf, Konfliktlösung oder Wiederherstellung); nichts getan"
    } else if out.canceled {
        "abgebrochen"
    } else if out.blocked.is_some() {
        "angehalten"
    } else if out.stats.errors > 0 {
        "mit Fehlern beendet"
    } else {
        "erfolgreich beendet"
    };
    let stats = &out.stats;
    log.line(
        "Ende",
        &format!(
            "{status} nach {:.1} s: {} Quelle→Ziel, {} Ziel→Quelle, {} gelöscht, {} Konflikte, {} Fehler, {} übertragen",
            elapsed.as_secs_f64(),
            stats.a_to_b,
            stats.b_to_a,
            stats.deleted,
            stats.conflicts,
            stats.errors,
            size(stats.bytes)
        ),
    );
}
