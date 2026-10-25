//! The confirmation window `smart_explorer --access-consent <ID>` (PRM-M3,
//! C5): the background worker starts it when a device asks for a capability
//! on „Fragen“. It reads its request from the worker once a second and shows
//! device, relation, fingerprint, capability, account and command; the
//! worker cleaned every text of the other device. „Diesmal erlauben“, „Immer
//! erlauben“ and „Ablehnen“ answer exactly this request. Closing the window
//! without an answer refuses; a request answered elsewhere or withdrawn
//! closes it. The answers that allow are enabled only shortly after the
//! request appears, so a click meant for another window cannot allow it.
//! A new update-stop request closes the window without an answer (V8).
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::share::access_approval::{ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalView};
use crate::share::consent_launch::{approval_context, approval_context_complete, capability_label, request_line};

/// How often the window reads its request.
const POLL: Duration = Duration::from_secs(1);
/// Delay before the answers that allow are enabled.
const ARM_DELAY: Duration = Duration::from_millis(1200);
/// Characters of the fingerprint the window shows.
const FINGERPRINT_CHARS: usize = 24;

/// Runs the window of the request `id` until it is answered or closed.
pub(crate) fn run_access_consent(id: String) -> eframe::Result<()> {
    crate::app::update_stop_ui::arm_window();
    let id = match ApprovalId::parse(&id) {
        Ok(id) => id,
        Err(error) => {
            eprintln!("Smart Explorer: --access-consent: {error}");
            return Ok(());
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Zugriff bestätigen – Smart Explorer")
            .with_inner_size([520.0, 400.0])
            .with_min_inner_size([420.0, 320.0])
            .with_always_on_top()
            .with_active(true)
            .with_icon(crate::window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Smart Explorer Zugriff",
        options,
        Box::new(move |cc| Ok(Box::new(ConsentWindow::new(id, &cc.egui_ctx)))),
    )
}

#[derive(Clone, Default)]
struct State {
    view: Option<ApprovalView>,
    /// The request was listed once; its absence then means it ended.
    seen: bool,
    gone: bool,
    error: Option<String>,
    sending: bool,
    answered: bool,
}

struct ConsentWindow {
    id: ApprovalId,
    state: Arc<Mutex<State>>,
    shown_at: Option<Instant>,
    update_stop: bool,
}

impl ConsentWindow {
    fn new(id: ApprovalId, ctx: &egui::Context) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (poll_id, slot, poll_ctx) = (id.clone(), Arc::downgrade(&state), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("access-consent-poll".into())
            .spawn(move || poll(&poll_id, &slot, &poll_ctx));
        Self {
            id,
            state,
            shown_at: None,
            update_stop: false,
        }
    }

    fn snapshot(&self) -> State {
        lock(&self.state).clone()
    }

    fn answer(&self, ctx: &egui::Context, decision: ApprovalDecision) {
        {
            let mut state = lock(&self.state);
            if state.sending || state.answered {
                return;
            }
            state.sending = true;
            state.error = None;
        }
        let (id, state, ctx) = (self.id.clone(), self.state.clone(), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("access-consent-answer".into())
            .spawn(move || {
                let result = crate::daemon::decide_access_approval(id.as_str(), decision);
                let mut state = lock(&state);
                state.sending = false;
                match result {
                    Ok(()) => state.answered = true,
                    Err(error) => state.error = Some(error),
                }
                ctx.request_repaint();
            });
    }

    fn body(&self, ui: &mut egui::Ui, state: &State, armed: bool) {
        let Some(view) = &state.view else {
            if state.gone {
                ui.label("Diese Rückfrage ist nicht mehr offen.");
                if ui.button("Schließen").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else if let Some(error) = &state.error {
                let text = format!("Rückfrage nicht lesbar: {error}");
                ui.colored_label(theme::danger(ui), text);
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Lade Rückfrage …");
                });
            }
            return;
        };
        ui.heading(request_line(view));
        ui.add_space(6.0);
        egui::Grid::new("access_consent_facts")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                row(
                    ui,
                    "Gerät",
                    &format!("{} ({})", view.device_name, view.device_id),
                );
                row(ui, "Beziehung", &relation(view));
                row(ui, "Fingerabdruck", &fingerprint(&view.fingerprint));
                row(ui, "Recht", capability_label(view.capability));
                if let Some(account) = &view.account_label {
                    row(ui, "Konto", account);
                }
                if let Some(command) = &view.command {
                    row(ui, "Befehl", &command_text(command));
                }
            });
        for detail in approval_context(view) {
            ui.add(egui::Label::new(detail).wrap());
        }
        if view.older_client_terminal {
            ui.add_space(4.0);
            let note = format!(
                "Älteres Smart Explorer auf {}: das Terminal wartet dort höchstens 60 Sekunden.",
                view.device_name
            );
            ui.colored_label(theme::warning(ui), note);
        }
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                "„Immer erlauben“ setzt dieses Recht in dieser Beziehung auf „Erlauben“. \
                 Die gespeicherten Zielbedingungen bleiben bestehen. Schließen lehnt ab.",
            )
            .wrap(),
        );
        if let Some(error) = &state.error {
            ui.colored_label(theme::danger(ui), error.as_str());
        }
        ui.add_space(6.0);
        let allow = armed && !state.sending && approval_context_complete(view);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(allow, egui::Button::new("Diesmal erlauben"))
                .clicked()
            {
                self.answer(ui.ctx(), ApprovalDecision::Once);
            }
            if ui
                .add_enabled(allow, egui::Button::new("Immer erlauben"))
                .clicked()
            {
                self.answer(ui.ctx(), ApprovalDecision::Always);
            }
            if ui
                .add_enabled(!state.sending, egui::Button::new("Ablehnen"))
                .clicked()
            {
                self.answer(ui.ctx(), ApprovalDecision::Deny);
            }
            if state.sending {
                ui.spinner();
            }
        });
    }
}

impl eframe::App for ConsentWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // V8: a new update-stop request closes the window, without an answer.
        if crate::app::update_stop_ui::close_requested(ctx) {
            self.update_stop = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let state = self.snapshot();
        if state.answered || (state.gone && state.seen) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if state.view.is_some() && self.shown_at.is_none() {
            self.shown_at = Some(Instant::now());
            let attention = egui::UserAttentionType::Critical;
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(attention));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let armed = self.shown_at.is_some_and(|at| at.elapsed() >= ARM_DELAY);
        if !armed {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.body(ui, &state, armed));
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Closing without an answer refuses (C5); an update stop is no answer.
        let state = self.snapshot();
        let open = state.view.is_some() && !state.gone && !state.answered && !state.sending;
        if open && !self.update_stop {
            let _ = crate::daemon::decide_access_approval(self.id.as_str(), ApprovalDecision::Deny);
        }
    }
}

/// Reads the request `id` once a second until the window goes or the
/// request ended.
fn poll(id: &ApprovalId, slot: &Weak<Mutex<State>>, ctx: &egui::Context) {
    while let Some(state) = slot.upgrade() {
        let listed = crate::daemon::access_approvals();
        let gone = {
            let mut state = lock(&state);
            match listed {
                Ok(pending) => match pending.into_iter().find(|view| &view.id == id) {
                    Some(view) => {
                        state.view = Some(view);
                        state.seen = true;
                        state.error = None;
                    }
                    None => state.gone = true,
                },
                Err(error) => state.error = Some(error),
            }
            state.gone
        };
        ctx.request_repaint();
        if gone {
            return;
        }
        drop(state);
        std::thread::sleep(POLL);
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).strong());
    ui.add(egui::Label::new(value).wrap());
    ui.end_row();
}

fn relation(view: &ApprovalView) -> String {
    match (view.relation.as_str(), &view.room) {
        ("room", Some(room)) => format!("Raum „{room}“"),
        ("room", None) => "Raum".into(),
        _ => "Direkt verbunden".into(),
    }
}

fn fingerprint(full: &str) -> String {
    if full.chars().count() > FINGERPRINT_CHARS {
        let shown: String = full.chars().take(FINGERPRINT_CHARS).collect();
        format!("{shown}…")
    } else {
        full.to_string()
    }
}

fn command_text(command: &ApprovalCommand) -> String {
    match command {
        ApprovalCommand::Argv { program, args } if args.is_empty() => program.clone(),
        ApprovalCommand::Argv { program, args } => format!("{program} {}", args.join(" ")),
        ApprovalCommand::Shell { text } => format!("Shell: {text}"),
        ApprovalCommand::Login => "Terminal (Login-Shell)".into(),
    }
}
