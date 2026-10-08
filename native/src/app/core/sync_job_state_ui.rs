//! Shared cached JobState display for setup manager and landing tiles.
use super::*;
use crate::syncjobs::{BlockKind, ChangeDetection, JobState, PendingKind, Runner, SyncJob};
use eframe::egui;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Cache {
    states: Arc<BTreeMap<String, JobState>>,
    ids: Vec<String>,
    next: Instant,
    loading: bool,
}
pub(in crate::app) fn states(
    ctx: &egui::Context,
    jobs: &[SyncJob],
) -> Arc<BTreeMap<String, JobState>> {
    let key = egui::Id::new("desktop-job-state-cache");
    let shared = ctx.data_mut(|data| {
        if let Some(cache) = data.get_temp::<Arc<Mutex<Cache>>>(key) {
            cache
        } else {
            let cache = Arc::new(Mutex::new(Cache {
                states: Arc::new(BTreeMap::new()),
                ids: Vec::new(),
                next: Instant::now(),
                loading: false,
            }));
            data.insert_temp(key, cache.clone());
            cache
        }
    });
    let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
    let ids: Vec<_> = jobs.iter().map(|job| job.id.clone()).collect();
    if !cache.loading && (cache.ids != ids || Instant::now() >= cache.next) {
        cache.loading = true;
        cache.ids = ids;
        cache.next = Instant::now() + Duration::from_secs(1);
        let jobs = jobs.to_vec();
        let worker_cache = shared.clone();
        let repaint = ctx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("desktop-job-state".into())
            .spawn(move || {
                let states = crate::syncjobs::load_job_states(&jobs);
                let mut cache = worker_cache.lock().unwrap_or_else(|e| e.into_inner());
                cache.states = Arc::new(states);
                cache.loading = false;
                drop(cache);
                repaint.request_repaint();
            })
        {
            let errors = jobs_error(cache.ids.clone(), error.to_string());
            cache.states = Arc::new(errors);
            cache.loading = false;
        }
    }
    ctx.request_repaint_after(Duration::from_secs(1));
    cache.states.clone()
}
fn jobs_error(ids: Vec<String>, error: String) -> BTreeMap<String, JobState> {
    ids.into_iter()
        .map(|id| {
            (
                id,
                JobState {
                    load_error: Some(error.clone()),
                    ..Default::default()
                },
            )
        })
        .collect()
}
fn time(value: Option<i64>) -> String {
    value
        .map(|value| super::fmt_ms(value.saturating_mul(1000)))
        .unwrap_or_else(|| "noch nie".into())
}
pub(in crate::app) fn summary(state: Option<&JobState>) -> (String, bool) {
    let Some(state) = state else {
        return ("Laufzustand wird geladen…".into(), false);
    };
    if let Some(error) = &state.load_error {
        return (format!("Laufzustand nicht lesbar: {error}"), true);
    }
    let detail = if let Some(mark) = state.running_now(now_secs_i64()) {
        format!(
            "{} läuft seit {}",
            runner(mark.runner),
            time(Some(mark.started))
        )
    } else if state.blocked.is_some() {
        "Sicherheitsstopp · Bestätigung erforderlich".into()
    } else if let Some(error) = &state.last_error {
        let next = if !error.kind.needs_user() {
            ""
        } else if state
            .recheck
            .as_ref()
            .is_some_and(|recheck| recheck.pending)
        {
            " · neuer Versuch vorgemerkt"
        } else {
            " · wartet auf erneute Anmeldung oder geänderte Einstellungen"
        };
        format!(
            "{} Fehler in Folge, zuletzt {}: {}{next}",
            state.consecutive_failures,
            time(state.last_attempt),
            error.message
        )
    } else if let Some(result) = &state.last_result {
        format!("{} · {} Konflikte", result.note, result.conflicts)
    } else {
        "kein Ergebnis gespeichert".into()
    };
    let warn = state.problem().is_some()
        || state.last_error.is_some()
        || state
            .last_result
            .as_ref()
            .is_some_and(|r| r.errors > 0 || r.conflicts > 0);
    (
        format!(
            "Versuch: {} · Erfolg: {} · {detail}",
            time(state.last_attempt),
            time(state.last_success)
        ),
        warn,
    )
}
fn runner(runner: Runner) -> &'static str {
    match runner {
        Runner::Daemon => "Hintergrunddienst",
        Runner::Desktop => "Fenster",
        Runner::Android => "Android",
        Runner::Cli => "Terminal",
        Runner::Other => "Sync",
    }
}
pub(in crate::app) fn render(ui: &mut egui::Ui, state: Option<&JobState>) {
    let (summary, warn) = summary(state);
    ui.colored_label(
        if warn {
            theme::warning(ui)
        } else {
            theme::muted(ui)
        },
        summary,
    );
    let Some(state) = state else {
        return;
    };
    if let Some(block) = &state.blocked {
        ui.colored_label(theme::warning(ui), &block.detail);
    }
    if let Some(result) = &state.last_result {
        ui.label(format!(
            "Ergebnis vom {}: {} →, {} ←, {} gelöscht · {} Konflikte · {} Fehler · {}",
            time(Some(result.when)),
            result.a_to_b,
            result.b_to_a,
            result.deleted,
            result.conflicts,
            result.errors,
            result.note
        ));
    }
    let running = state.running_now(now_secs_i64()).is_some();
    if let Some(mark) = state.running_now(now_secs_i64()) {
        if let Some(since) = mark.stalled_since {
            ui.colored_label(
                theme::warning(ui),
                format!(
                    "Keine Lauf-Aktivität seit {}; der letzte Schritt steht im Protokoll.",
                    time(Some(since))
                ),
            );
        }
    } else if let Some(mark) = &state.running {
        ui.colored_label(
            theme::warning(ui),
            format!(
                "Lauf von {} ({}) meldet sich seit {} nicht mehr; er wird als unterbrochen vermerkt.",
                time(Some(mark.started)),
                runner(mark.runner),
                time(Some(mark.alive))
            ),
        );
    }
    if let Some(interrupted) = &state.interrupted {
        ui.colored_label(
            theme::warning(ui),
            format!(
                "Lauf von {} ({}) endete ohne Ergebnis (letztes Lebenszeichen {}); ein Kontrolllauf ist vorgemerkt.",
                time(Some(interrupted.started)),
                runner(interrupted.runner),
                time(Some(interrupted.alive))
            ),
        );
    }
    if let Some(recheck) = state.recheck.as_ref().filter(|recheck| recheck.pending) {
        ui.label(format!(
            "Neuer Versuch vorgemerkt: {} ({})",
            recheck.reason,
            time(Some(recheck.evidence))
        ));
    }
    if let Some(retry) = state.retry_at {
        ui.label(format!(
            "Automatischer Wiederholungsversuch ab {}",
            time(Some(retry))
        ));
    }
    if let Some(pending) = &state.pending_trigger {
        let what = match pending.kind {
            PendingKind::Change => "Änderung erkannt",
            PendingKind::Verify => "Kontrolllauf fällig",
            PendingKind::Startup => "Startlauf offen",
            PendingKind::Connect => "Laufwerk angeschlossen",
            PendingKind::Confirmed => "Bestätigter Lauf offen",
            PendingKind::Other => "Geplanter Lauf offen",
        };
        let when = if running {
            "wird nach dem laufenden Lauf übernommen"
        } else {
            "wird im nächsten Lauf übernommen"
        };
        ui.label(format!(
            "{what} seit {}; {when}.",
            time(Some(pending.since))
        ));
    }
    if let Some(watch) = &state.watch {
        let detection = match &watch.detection {
            ChangeDetection::Starting => "Überwachung wird eingerichtet".into(),
            ChangeDetection::Events => "Änderungsereignisse mit Kontrollläufen".into(),
            ChangeDetection::EventsAndPoll { poll_secs } => {
                format!("Änderungsereignisse + Vergleich alle {poll_secs} s")
            }
            ChangeDetection::Poll { poll_secs } => format!("Vergleich alle {poll_secs} s"),
            ChangeDetection::Other => "Erkennung unbekannt".into(),
        };
        ui.label(format!(
            "Zuletzt gemeldete Erkennung: {detection} (seit {})",
            time(Some(watch.since))
        ));
        if let Some(note) = &watch.note {
            ui.label(note);
        }
    }
}

#[derive(Clone)]
pub(in crate::app) struct BlockReview {
    pub id: String,
    pub kind: BlockKind,
    pub detail: String,
    pub source: String,
    pub target: String,
}
pub(in crate::app) fn confirmation(
    ctx: &egui::Context,
    requested: Option<BlockReview>,
) -> Option<BlockReview> {
    let key = egui::Id::new("desktop-block-confirmation");
    if let Some(review) = requested {
        ctx.data_mut(|data| data.insert_temp(key, review));
    }
    let review = ctx.data(|data| data.get_temp::<BlockReview>(key))?;
    let mut open = true;
    let mut approved = false;
    egui::Window::new("Sicherheitsstopp bestätigen").open(&mut open).collapsible(false).show(ctx, |ui| {
        ui.label(format!("Quelle: {}\nZiel: {}",review.source,review.target));
        ui.label(&review.detail);
        ui.label("Bitte Quelle, Ziel und Laufwerk prüfen. Die Bestätigung gilt einmal für genau diesen Stopp; neue oder größere Änderungen bleiben geschützt.");
        if ui.button("Trotzdem ausführen").clicked() { approved = true; }
    });
    if !open || approved {
        ctx.data_mut(|data| data.remove::<BlockReview>(key));
    }
    if approved {
        Some(review)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_job_state_failed_attempt_does_not_hide_prior_success() {
        let state = JobState {
            last_attempt: Some(2000),
            last_success: Some(1000),
            consecutive_failures: 3,
            last_error: Some(crate::syncjobs::JobError {
                kind: crate::syncjobs::FailureKind::Auth,
                message: "Anmeldung erforderlich".into(),
            }),
            ..Default::default()
        };
        let (text, warn) = summary(Some(&state));
        assert!(warn);
        assert!(text.contains(&time(Some(2000))));
        assert!(text.contains(&time(Some(1000))));
        assert!(text.contains("3 Fehler in Folge"));
        assert!(text.contains("Anmeldung erforderlich"));
    }
    #[test]
    fn sync_transparency_task_job_line_dates_errors_and_names_the_way_out() {
        let mut state = JobState {
            last_attempt: Some(2000),
            consecutive_failures: 7,
            last_error: Some(crate::syncjobs::JobError {
                kind: crate::syncjobs::FailureKind::Auth,
                message: "HTTP 400: invalid_grant".into(),
            }),
            ..Default::default()
        };
        let (text, warn) = summary(Some(&state));
        assert!(warn);
        assert!(text.contains(&format!("7 Fehler in Folge, zuletzt {}", time(Some(2000)))));
        assert!(text.contains("wartet auf erneute Anmeldung"));
        state.recheck = Some(crate::syncjobs::Recheck {
            evidence: 3000,
            reason: "Anmeldedaten wurden geändert".into(),
            pending: true,
        });
        let (text, _) = summary(Some(&state));
        assert!(text.contains("neuer Versuch vorgemerkt"));
    }

    #[test]
    fn desktop_job_state_block_takes_precedence_over_old_success_result() {
        let mut state = JobState::default();
        state.last_success = Some(1000);
        state.last_result = Some(crate::syncjobs::JobResult {
            when: 1000,
            a_to_b: 0,
            b_to_a: 0,
            deleted: 0,
            conflicts: 0,
            errors: 0,
            note: "ok".into(),
        });
        state.blocked = Some(crate::syncjobs::Blocked {
            kind: BlockKind::SideEmpty {
                side: crate::syncjobs::JobSide::B,
                previous: 3,
            },
            detail: "Laufwerk fehlt".into(),
            since: 2000,
            confirmed: false,
        });
        let (text, warn) = summary(Some(&state));
        assert!(warn);
        assert!(text.contains("Bestätigung erforderlich"));
    }
}
