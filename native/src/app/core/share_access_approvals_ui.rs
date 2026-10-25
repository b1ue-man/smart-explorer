//! The open requests on top of the tab „Befehle“ (PRM-M9, C5): every request
//! that waits on this host with the answers of the consent window („Diesmal
//! erlauben“, „Immer erlauben“, „Ablehnen“), also when no window could be
//! shown. The list is read from the worker every 2 seconds in the background
//! while the tab is shown. The answers that allow unlock 1.2 seconds after a
//! request appears, so a click meant for something else cannot allow it.
//! Nothing is shown while no request waits.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use super::programs_ui::heading;
use crate::app::theme;
use crate::share::access_approval::{ApprovalCommand, ApprovalDecision, ApprovalView};
use crate::share::consent_launch::{approval_context, approval_context_complete, capability_label, request_line};

/// How often the list is read while the tab is shown.
const POLL: Duration = Duration::from_secs(2);
/// Delay before the answers that allow are enabled (as the consent window).
pub(super) const ARM_DELAY: Duration = Duration::from_millis(1200);

#[derive(Default)]
struct Pending {
    requests: Vec<ApprovalView>,
    /// When each request was first listed.
    seen: BTreeMap<String, Instant>,
    error: Option<String>,
    reading: bool,
    read_at: Option<Instant>,
    sending: bool,
}

type Shared = Arc<Mutex<Pending>>;

fn shared(ctx: &egui::Context) -> Shared {
    let id = egui::Id::new("share_access_pending");
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(Pending::default())))
            .clone()
    })
}

fn lock(state: &Shared) -> MutexGuard<'_, Pending> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The block of the tab „Befehle“: reads the worker, shows the requests and
/// sends the answer clicked.
pub(super) fn ui_pending(ui: &mut egui::Ui) {
    let state = shared(ui.ctx());
    refresh(ui.ctx(), &state);
    let (requests, error, armed) = {
        let pending = lock(&state);
        let now = Instant::now();
        let armed: Vec<String> = pending
            .seen
            .iter()
            .filter(|(_, at)| now.duration_since(**at) >= ARM_DELAY && !pending.sending)
            .map(|(id, _)| id.clone())
            .collect();
        (pending.requests.clone(), pending.error.clone(), armed)
    };
    if armed.len() < requests.len() {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
    let answer = block(ui, &requests, &|id| armed.iter().any(|armed| armed == id));
    if let Some(error) = error {
        ui.colored_label(theme::danger(ui), error);
    }
    if let Some((id, decision)) = answer {
        send(ui.ctx(), &state, id, decision);
    }
}

/// The requests with their three answers; the answer clicked. `armed` says
/// whether the allowing answers of a request (by id) are enabled.
pub(super) fn block(
    ui: &mut egui::Ui,
    requests: &[ApprovalView],
    armed: &dyn Fn(&str) -> bool,
) -> Option<(String, ApprovalDecision)> {
    if requests.is_empty() {
        return None;
    }
    heading(ui, &format!("OFFENE RÜCKFRAGEN ({})", requests.len()));
    let mut answer = None;
    for request in requests {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(request_line(request)).strong());
            ui.label(facts(request));
            for detail in approval_context(request) {
                ui.add(egui::Label::new(detail).wrap());
            }
            if request.older_client_terminal {
                let note = format!(
                    "Älteres Smart Explorer auf {}: das Terminal wartet dort höchstens 60 Sekunden.",
                    request.device_name
                );
                ui.colored_label(theme::warning(ui), note);
            }
            let allow = armed(request.id.as_str()) && approval_context_complete(request);
            ui.horizontal_wrapped(|ui| {
                let answers = [
                    (ApprovalDecision::Once, "Diesmal erlauben"),
                    (ApprovalDecision::Always, "Immer erlauben"),
                    (ApprovalDecision::Deny, "Ablehnen"),
                ];
                for (decision, label) in answers {
                    let enabled = allow || decision == ApprovalDecision::Deny;
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        answer = Some((request.id.to_string(), decision));
                    }
                }
            });
        });
    }
    let hint = "„Immer erlauben“ setzt dieses Recht in dieser Beziehung auf „Erlauben“. \
                Die gespeicherten Zielbedingungen bleiben bestehen.";
    ui.label(RichText::new(hint).small().color(theme::muted(ui)));
    answer
}

/// Relation, capability, account and command of a request in one line.
fn facts(request: &ApprovalView) -> String {
    let relation = match (request.relation.as_str(), &request.room) {
        ("room", Some(room)) => format!("Raum „{room}“"),
        ("room", None) => "Raum".to_string(),
        _ => "direkt verbunden".to_string(),
    };
    let mut parts = vec![relation, capability_label(request.capability).to_string()];
    if let Some(account) = &request.account_label {
        parts.push(format!("als {account}"));
    }
    match &request.command {
        Some(ApprovalCommand::Argv { program, args }) => parts.push(
            format!("{program} {}", args.join(" "))
                .trim_end()
                .to_string(),
        ),
        Some(ApprovalCommand::Shell { text }) => parts.push(format!("Shell: {text}")),
        Some(ApprovalCommand::Login) => parts.push("Terminal (Login-Shell)".into()),
        None => {}
    }
    parts.join(" · ")
}

/// Reads the list in the background when it is due.
fn refresh(ctx: &egui::Context, state: &Shared) {
    let due = {
        let mut pending = lock(state);
        let due = !pending.reading && pending.read_at.is_none_or(|at| at.elapsed() >= POLL);
        pending.reading |= due;
        due
    };
    ctx.request_repaint_after(POLL);
    if !due {
        return;
    }
    let (thread_state, thread_ctx) = (state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("share-access-pending".into())
        .spawn(move || {
            let listed = crate::daemon::access_approvals();
            let mut pending = lock(&thread_state);
            pending.reading = false;
            pending.read_at = Some(Instant::now());
            match listed {
                Ok(requests) => {
                    let now = Instant::now();
                    pending
                        .seen
                        .retain(|id, _| requests.iter().any(|r| r.id.as_str() == id));
                    for request in &requests {
                        pending.seen.entry(request.id.to_string()).or_insert(now);
                    }
                    pending.requests = requests;
                    pending.error = None;
                }
                Err(error) => pending.error = Some(format!("Rückfragen nicht lesbar: {error}")),
            }
            thread_ctx.request_repaint();
        });
    if spawned.is_err() {
        lock(state).reading = false;
    }
}

/// Sends one answer in the background; the list is read again afterwards.
fn send(ctx: &egui::Context, state: &Shared, id: String, decision: ApprovalDecision) {
    {
        let mut pending = lock(state);
        if pending.sending {
            return;
        }
        pending.sending = true;
        pending.error = None;
    }
    let (thread_state, thread_ctx) = (state.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("share-access-answer".into())
        .spawn(move || {
            let result = crate::daemon::decide_access_approval(&id, decision);
            let mut pending = lock(&thread_state);
            pending.sending = false;
            pending.read_at = None;
            match result {
                Ok(()) => pending.requests.retain(|request| request.id.as_str() != id),
                Err(error) => pending.error = Some(format!("Antwort nicht übermittelt: {error}")),
            }
            thread_ctx.request_repaint();
        });
    if spawned.is_err() {
        lock(state).sending = false;
    }
}
