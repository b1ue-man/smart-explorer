use eframe::egui::{self, Color32, RichText};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct ExecJobsUiState {
    snapshot: Option<crate::daemon::ExecJobsSnapshot>,
    loading: bool,
    last_load: Option<Instant>,
    cancelling: Vec<String>,
    error: Option<String>,
    notice: Option<String>,
}

struct ActiveJobControls<'a> {
    direction: crate::daemon::ExecJobDirection,
    cancelling: &'a [String],
    cache: &'a Arc<Mutex<ExecJobsUiState>>,
}

pub(in crate::app) fn ui_exec_jobs(ui: &mut egui::Ui) {
    let cache = jobs_cache(ui);
    request_refresh(ui.ctx().clone(), cache.clone(), false);
    let (snapshot, loading, cancelling, error, notice) = cache
        .lock()
        .map(|state| {
            (
                state.snapshot.clone(),
                state.loading,
                state.cancelling.clone(),
                state.error.clone(),
                state.notice.clone(),
            )
        })
        .unwrap_or_default();

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("AKTIVE UND LETZTE EXEC-JOBS")
                .small()
                .color(Color32::from_gray(140)),
        );
        if ui
            .add_enabled(!loading, egui::Button::new("Aktualisieren"))
            .clicked()
        {
            request_refresh(ui.ctx().clone(), cache.clone(), true);
        }
        if loading {
            ui.spinner();
        }
    });
    if let Some(error) = error {
        ui.colored_label(
            Color32::from_rgb(230, 100, 100),
            format!("Exec-Status: {error}"),
        );
    }
    if let Some(notice) = notice {
        ui.label(notice);
    }
    let Some(snapshot) = snapshot else {
        ui.small("Exec-Jobs werden vom Background-Worker geladen.");
        return;
    };

    active_jobs(
        ui,
        "EINGEHEND — laeuft auf diesem Geraet",
        &snapshot.incoming_active,
        crate::daemon::ExecJobDirection::Incoming,
        &cancelling,
        &cache,
    );
    active_jobs(
        ui,
        "AUSGEHEND — laeuft auf dem Peer",
        &snapshot.outgoing_active,
        crate::daemon::ExecJobDirection::Outgoing,
        &cancelling,
        &cache,
    );

    let history_count = snapshot.incoming_history.len() + snapshot.outgoing_history.len();
    egui::CollapsingHeader::new(format!("LETZTE EXEC-JOBS ({history_count})"))
        .default_open(false)
        .show(ui, |ui| {
            if history_count == 0 {
                ui.label("Noch keine abgeschlossenen Exec-Jobs.");
            }
            history_jobs(ui, "Eingehend", &snapshot.incoming_history);
            history_jobs(ui, "Ausgehend", &snapshot.outgoing_history);
        });
}

fn active_jobs(
    ui: &mut egui::Ui,
    heading: &str,
    jobs: &[crate::share::ExecJobView],
    direction: crate::daemon::ExecJobDirection,
    cancelling: &[String],
    cache: &Arc<Mutex<ExecJobsUiState>>,
) {
    ui.label(RichText::new(heading).small().strong());
    if jobs.is_empty() {
        ui.label("Keine aktiven Jobs.");
        return;
    }
    for job in jobs {
        job_card(
            ui,
            job,
            Some(ActiveJobControls {
                direction,
                cancelling,
                cache,
            }),
        );
    }
}

fn history_jobs(ui: &mut egui::Ui, heading: &str, jobs: &[crate::share::ExecJobView]) {
    if jobs.is_empty() {
        return;
    }
    ui.label(RichText::new(heading).small().strong());
    for job in jobs.iter().rev() {
        job_card(ui, job, None);
    }
}

fn job_card(
    ui: &mut egui::Ui,
    job: &crate::share::ExecJobView,
    active: Option<ActiveJobControls<'_>>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(peer_label(job)).strong());
            ui.label(lifecycle_label(&job.state));
        });
        ui.label(format!("Exec-ID: {}", job.exec_id));
        ui.label(format!("Programm: {}", job.program));
        ui.small(format!("Command-Digest: {}", job.command_digest));
        ui.small(format!("Policy-Revision: {}", job.policy_revision));
        if let Some(started) = job.started_at {
            ui.small(format!("Gestartet: {}", timestamp(started)));
        }
        if let Some(terminal) = &job.terminal {
            ui.small(format!(
                "Ergebnis: {:?}, Exit={:?}, stdout={} B, stderr={} B{}",
                terminal.kind,
                terminal.exit_code,
                terminal.stdout_bytes,
                terminal.stderr_bytes,
                if terminal.output_truncated {
                    ", Ausgabe gekuerzt"
                } else {
                    ""
                }
            ));
        }
        if let Some(active) = active {
            let key = format!("{:?}:{}", active.direction, job.exec_id);
            let pending = active.cancelling.contains(&key);
            if ui
                .add_enabled(!pending, egui::Button::new("Job abbrechen"))
                .clicked()
            {
                request_cancel(
                    ui.ctx().clone(),
                    active.cache.clone(),
                    cancel_target(active.direction, job),
                );
            }
            if pending {
                ui.spinner();
            }
        }
    });
    ui.add_space(3.0);
}

fn cancel_target(
    direction: crate::daemon::ExecJobDirection,
    job: &crate::share::ExecJobView,
) -> crate::daemon::ExecCancelTarget {
    crate::daemon::ExecCancelTarget {
        direction,
        exec_id: job.exec_id.clone(),
        peer_device_id: job.peer_device_id.clone(),
    }
}

fn jobs_cache(ui: &mut egui::Ui) -> Arc<Mutex<ExecJobsUiState>> {
    let id = egui::Id::new("share_exec_jobs_cache");
    if let Some(cache) = ui.data_mut(|data| data.get_temp::<Arc<Mutex<ExecJobsUiState>>>(id)) {
        return cache;
    }
    let cache = Arc::new(Mutex::new(ExecJobsUiState::default()));
    ui.data_mut(|data| data.insert_temp(id, cache.clone()));
    cache
}

fn request_refresh(ctx: egui::Context, cache: Arc<Mutex<ExecJobsUiState>>, force: bool) {
    let should_start = cache
        .lock()
        .map(|mut state| {
            let stale = state
                .last_load
                .is_none_or(|last| last.elapsed() >= REFRESH_INTERVAL);
            if state.loading || (!force && !stale) {
                false
            } else {
                state.loading = true;
                true
            }
        })
        .unwrap_or(false);
    if !should_start {
        return;
    }
    std::thread::spawn(move || {
        let result = crate::daemon::exec_jobs();
        if let Ok(mut state) = cache.lock() {
            state.loading = false;
            state.last_load = Some(Instant::now());
            match result {
                Ok(snapshot) => {
                    state.snapshot = Some(snapshot);
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
        }
        ctx.request_repaint();
        ctx.request_repaint_after(REFRESH_INTERVAL);
    });
}

fn request_cancel(
    ctx: egui::Context,
    cache: Arc<Mutex<ExecJobsUiState>>,
    target: crate::daemon::ExecCancelTarget,
) {
    let exec_id = target.exec_id.clone();
    let key = format!("{:?}:{exec_id}", target.direction);
    if let Ok(mut state) = cache.lock() {
        if state.cancelling.contains(&key) {
            return;
        }
        state.cancelling.push(key.clone());
        state.notice = Some(format!("Abbruch fuer {exec_id} wird gesendet …"));
    }
    std::thread::spawn(move || {
        let result = crate::daemon::cancel_exec(target);
        let snapshot = result.as_ref().ok().and_then(|found| {
            if *found {
                crate::daemon::exec_jobs().ok()
            } else {
                None
            }
        });
        if let Ok(mut state) = cache.lock() {
            state.cancelling.retain(|pending| pending != &key);
            state.last_load = Some(Instant::now());
            if let Some(snapshot) = snapshot {
                state.snapshot = Some(snapshot);
            }
            match result {
                Ok(true) => {
                    state.notice = Some(format!("Abbruch fuer {exec_id} angefordert"));
                    state.error = None;
                }
                Ok(false) => {
                    state.error = Some(format!("Exec-Job nicht mehr aktiv: {exec_id}"));
                    state.notice = None;
                }
                Err(error) => {
                    state.error = Some(format!("Exec-Abbruch fehlgeschlagen: {error}"));
                    state.notice = None;
                }
            }
        }
        ctx.request_repaint();
        ctx.request_repaint_after(REFRESH_INTERVAL);
    });
}

fn peer_label(job: &crate::share::ExecJobView) -> String {
    if job.peer_device_name.trim().is_empty() {
        format!("Geraet {}", job.peer_device_id)
    } else {
        format!("{} ({})", job.peer_device_name, job.peer_device_id)
    }
}

fn lifecycle_label(state: &crate::share::ExecLifecycleState) -> &'static str {
    use crate::share::ExecLifecycleState::*;
    match state {
        QueuedLocal => "queued-local",
        Connecting => "connecting",
        Authenticating => "authenticating",
        Authorized => "authorized",
        Starting => "starting",
        Running => "running",
        Cancelling => "cancelling",
        Exited => "exited",
        Failed => "failed",
        TimedOut => "timed-out",
        Cancelled => "cancelled",
        Revoked => "revoked",
        Disconnected => "disconnected",
    }
}

fn timestamp(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| format!("{seconds} (Unix)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_labels_cover_active_and_terminal_states() {
        assert_eq!(
            lifecycle_label(&crate::share::ExecLifecycleState::Running),
            "running"
        );
        assert_eq!(
            lifecycle_label(&crate::share::ExecLifecycleState::Revoked),
            "revoked"
        );
    }

    #[test]
    fn cancellation_keeps_direction_id_and_peer_identity() {
        let job = crate::share::ExecJobView {
            exec_id: crate::share::ExecId::parse("11".repeat(16)).unwrap(),
            peer_device_id: "peer-a".into(),
            peer_device_name: "Peer A".into(),
            program: "<shell>".into(),
            command_digest: "digest".into(),
            state: crate::share::ExecLifecycleState::Running,
            policy_revision: 3,
            started_at: Some(1),
            finished_at: None,
            terminal: None,
        };
        let target = cancel_target(crate::daemon::ExecJobDirection::Incoming, &job);
        assert_eq!(target.direction, crate::daemon::ExecJobDirection::Incoming);
        assert_eq!(target.exec_id, job.exec_id);
        assert_eq!(target.peer_device_id, "peer-a");
    }
}
