//! Shared cached JobState display for setup manager and landing tiles.
use std::{collections::BTreeMap, sync::{Arc, Mutex}, time::{Duration, Instant}};
use crate::syncjobs::{BlockKind, ChangeDetection, JobState, Runner, SyncJob};
use super::*;

struct Cache {
    states: Arc<BTreeMap<String, JobState>>, ids:Vec<String>, next:Instant, loading:bool,
}
pub(in crate::app) fn states(ctx: &egui::Context, jobs: &[SyncJob]) -> Arc<BTreeMap<String,JobState>> {
    let key = egui::Id::new("desktop-job-state-cache");
    let shared = ctx.data_mut(|data| {
        if let Some(cache) = data.get_temp::<Arc<Mutex<Cache>>>(key) { cache }
        else {
            let cache = Arc::new(Mutex::new(Cache { states:Arc::new(BTreeMap::new()), ids:Vec::new(), next:Instant::now(), loading:false }));
            data.insert_temp(key, cache.clone()); cache
        }
    });
    let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
    let ids:Vec<_> = jobs.iter().map(|job| job.id.clone()).collect();
    if !cache.loading && (cache.ids != ids || Instant::now() >= cache.next) {
        cache.loading = true; cache.ids = ids; cache.next = Instant::now() + Duration::from_secs(1);
        let jobs = jobs.to_vec(); let worker_cache = shared.clone(); let repaint = ctx.clone();
        if let Err(error) = std::thread::Builder::new().name("desktop-job-state".into()).spawn(move || {
            let states = crate::syncjobs::load_job_states(&jobs);
            let mut cache = worker_cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.states = Arc::new(states); cache.loading = false;
            drop(cache); repaint.request_repaint();
        }) {
            let errors = jobs_error(cache.ids.clone(), error.to_string());
            cache.states = Arc::new(errors);
            cache.loading = false;
        }
    }
    ctx.request_repaint_after(Duration::from_secs(1));
    cache.states.clone()
}
fn jobs_error(ids: Vec<String>, error: String) -> BTreeMap<String,JobState> {
    ids.into_iter().map(|id| (id, JobState { load_error:Some(error.clone()), ..Default::default() })).collect()
}
fn time(value: Option<i64>) -> String {
    value.map(|value| super::fmt_ms(value.saturating_mul(1000))).unwrap_or_else(|| "noch nie".into())
}
pub(in crate::app) fn summary(state: Option<&JobState>) -> (String, bool) {
    let Some(state) = state else { return ("Laufzustand wird geladen…".into(), false); };
    if let Some(error) = &state.load_error { return (format!("Laufzustand nicht lesbar: {error}"), true); }
    let detail = if let Some(mark) = state.running_now(now_secs_i64()) {
        format!("{} läuft", runner(mark.runner))
    } else if state.blocked.is_some() { "Sicherheitsstopp · Bestätigung erforderlich".into() }
    else if let Some(error) = &state.last_error { format!("{} Fehler in Folge: {}", state.consecutive_failures, error.message) }
    else if let Some(result) = &state.last_result { format!("{} · {} Konflikte", result.note, result.conflicts) }
    else { "kein Ergebnis gespeichert".into() };
    let warn = state.problem().is_some() || state.last_error.is_some()
        || state.last_result.as_ref().is_some_and(|r| r.errors > 0 || r.conflicts > 0);
    (format!("Versuch: {} · Erfolg: {} · {detail}", time(state.last_attempt), time(state.last_success)), warn)
}
fn runner(runner: Runner) -> &'static str {
    match runner { Runner::Daemon => "Hintergrunddienst", Runner::Desktop => "Fenster",
        Runner::Android => "Android", Runner::Cli => "Terminal", Runner::Other => "Sync" }
}
pub(in crate::app) fn render(ui: &mut egui::Ui, state: Option<&JobState>) {
    let (summary, warn) = summary(state);
    ui.colored_label(if warn { theme::warning(ui) } else { theme::muted(ui) }, summary);
    let Some(state) = state else { return; };
    if let Some(block) = &state.blocked { ui.colored_label(theme::warning(ui), &block.detail); }
    if let Some(result) = &state.last_result {
        ui.label(format!("Ergebnis vom {}: {} →, {} ←, {} gelöscht · {} Konflikte · {} Fehler · {}",
            time(Some(result.when)), result.a_to_b, result.b_to_a, result.deleted, result.conflicts, result.errors, result.note));
    }
    if let Some(mark) = state.running_now(now_secs_i64()) {
        if mark.stalled_since.is_some() { ui.colored_label(theme::warning(ui), "Lauf ohne Fortschritt; Dienst prüft Wiederanlauf."); }
    } else if state.running.is_some() { ui.colored_label(theme::warning(ui), "Letzter Läufer meldet sich nicht mehr; Ergebnis prüfen."); }
    if let Some(retry) = state.retry_at { ui.label(format!("Automatischer Wiederholungsversuch ab {}", time(Some(retry)))); }
    if state.pending_trigger.is_some() { ui.label("Ausstehender Auslöser bleibt vorgemerkt."); }
    if let Some(watch) = &state.watch {
        let detection = match &watch.detection {
            ChangeDetection::Starting => "Überwachung wird eingerichtet".into(),
            ChangeDetection::Events => "Änderungsereignisse mit Kontrollläufen".into(),
            ChangeDetection::EventsAndPoll { poll_secs } => format!("Änderungsereignisse + Vergleich alle {poll_secs} s"),
            ChangeDetection::Poll { poll_secs } => format!("Vergleich alle {poll_secs} s"),
            ChangeDetection::Other => "Erkennung unbekannt".into(),
        };
        ui.label(format!("Zuletzt gemeldete Erkennung: {detection} (seit {})",time(Some(watch.since))));
        if let Some(note) = &watch.note { ui.label(note); }
    }
}

#[derive(Clone)]
pub(in crate::app) struct BlockReview {
    pub id:String, pub kind:BlockKind, pub detail:String, pub source:String, pub target:String,
}
pub(in crate::app) fn confirmation(ctx: &egui::Context, requested:Option<BlockReview>) -> Option<BlockReview> {
    let key = egui::Id::new("desktop-block-confirmation");
    if let Some(review) = requested { ctx.data_mut(|data| data.insert_temp(key, review)); }
    let review = ctx.data(|data| data.get_temp::<BlockReview>(key))?;
    let mut open = true; let mut approved = false;
    egui::Window::new("Sicherheitsstopp bestätigen").open(&mut open).collapsible(false).show(ctx, |ui| {
        ui.label(format!("Quelle: {}\nZiel: {}",review.source,review.target));
        ui.label(&review.detail);
        ui.label("Bitte Quelle, Ziel und Laufwerk prüfen. Die Bestätigung gilt einmal für genau diesen Stopp; neue oder größere Änderungen bleiben geschützt.");
        if ui.button("Trotzdem ausführen").clicked() { approved = true; }
    });
    if !open || approved { ctx.data_mut(|data| data.remove::<BlockReview>(key)); }
    if approved { Some(review) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_job_state_failed_attempt_does_not_hide_prior_success() {
        let state = JobState { last_attempt:Some(2000),last_success:Some(1000),consecutive_failures:3,
            last_error:Some(crate::syncjobs::JobError { kind:crate::syncjobs::FailureKind::Auth,message:"Anmeldung erforderlich".into() }),
            ..Default::default() };
        let (text,warn) = summary(Some(&state));
        assert!(warn); assert!(text.contains(&time(Some(2000)))); assert!(text.contains(&time(Some(1000))));
        assert!(text.contains("3 Fehler in Folge")); assert!(text.contains("Anmeldung erforderlich"));
    }
    #[test]
    fn desktop_job_state_block_takes_precedence_over_old_success_result() {
        let mut state = JobState::default();
        state.last_success = Some(1000);
        state.last_result = Some(crate::syncjobs::JobResult { when:1000,a_to_b:0,b_to_a:0,deleted:0,conflicts:0,errors:0,note:"ok".into() });
        state.blocked = Some(crate::syncjobs::Blocked { kind:BlockKind::SideEmpty { side:crate::syncjobs::JobSide::B,previous:3 },
            detail:"Laufwerk fehlt".into(),since:2000,confirmed:false });
        let (text,warn) = summary(Some(&state));
        assert!(warn); assert!(text.contains("Bestätigung erforderlich"));
    }
}
