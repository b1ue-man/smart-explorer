//! The confirmation window `smart_explorer --access-consent <ID>` (PRM-M3,
//! C5, E28): the background worker starts it when a device asks for a
//! capability on „Fragen“. The window belongs to that device: it lists every
//! open request of the device with their count, and each request is answered on
//! its own. „Diesmal erlauben“ answers exactly that request; „Immer erlauben“
//! and „Ablehnen“ also answer the equal open requests of the device. Closing
//! the window without an answer refuses the requests it shows; one that came
//! after its last read stays open and the worker shows it again. A window whose
//! requests all ended closes. The answers that allow are enabled only shortly
//! after the listed requests last changed, so a click meant for another window
//! or another request cannot allow one. A new update-stop request closes the
//! window without an answer (V8). A window of a followed session (E46) reads
//! and answers through the channel its worker gave it and sends the
//! notification of its request itself, on its account's session bus.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::share::access_approval::{ApprovalCommand, ApprovalDecision, ApprovalId, ApprovalView};
use crate::share::consent_launch::{
    approval_context, approval_context_complete, capability_label, notify_from_window, request_line,
};

/// How often the window reads its requests.
const POLL: Duration = Duration::from_secs(1);
/// Delay before the answers that allow are enabled.
const ARM_DELAY: Duration = Duration::from_millis(1200);
/// Characters of the fingerprint the window shows.
const FINGERPRINT_CHARS: usize = 24;
/// Failed reads in a row after which a window channel counts as gone.
const CHANNEL_GONE_AFTER: u32 = 10;
/// What the answers mean beyond their own request (shown below the requests).
const SHARED_ANSWERS: &str = "„Immer erlauben“ setzt dieses Recht in dieser Beziehung auf \
     „Erlauben“. Die gespeicherten Zielbedingungen bleiben bestehen. „Immer erlauben“ und \
     „Ablehnen“ beantworten zusätzlich alle gleichen offenen Anfragen dieses Geräts. Schließen \
     lehnt die angezeigten Anfragen ab.";

/// Runs the window of the request `id` until its requests are answered or the window is closed.
pub fn run_access_consent(id: String) -> eframe::Result<()> {
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
    /// The open requests of the window's device, oldest first, as the last read listed them.
    views: Vec<ApprovalView>,
    /// The device of the window's own request, once that request was listed.
    device: Option<String>,
    /// The device was listed once: no open request left then means it ended.
    seen: bool,
    gone: bool,
    error: Option<String>,
    /// The request whose answer is being sent.
    sending: Option<String>,
}

/// The window; its own request is known only to the poll thread, which learns
/// the window's device from it.
struct ConsentWindow {
    state: Arc<Mutex<State>>,
    /// When the listed requests last changed; the allowing answers are armed after `ARM_DELAY`.
    shown_at: Option<Instant>,
    /// The requests the window listed at the previous frame, in their order.
    listed: Vec<ApprovalId>,
    update_stop: bool,
}

impl ConsentWindow {
    fn new(id: ApprovalId, ctx: &egui::Context) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (slot, poll_ctx) = (Arc::downgrade(&state), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("access-consent-poll".into())
            .spawn(move || poll(&id, &slot, &poll_ctx));
        Self {
            state,
            shown_at: None,
            listed: Vec::new(),
            update_stop: false,
        }
    }

    fn snapshot(&self) -> State {
        lock(&self.state).clone()
    }

    /// Sends the answer `decision` for the request `id`; one answer is sent at a time.
    fn answer(&self, ctx: &egui::Context, id: &str, decision: ApprovalDecision) {
        {
            let mut state = lock(&self.state);
            if state.sending.is_some() {
                return;
            }
            state.sending = Some(id.to_string());
            state.error = None;
        }
        let (id, state, ctx) = (id.to_string(), self.state.clone(), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("access-consent-answer".into())
            .spawn(move || {
                let result = crate::daemon::decide_access_approval(&id, decision);
                let mut state = lock(&state);
                state.sending = None;
                match result {
                    Ok(_) => state.views.retain(|view| view.id.as_str() != id.as_str()),
                    Err(error) => state.error = Some(error),
                }
                ctx.request_repaint();
            });
    }

    fn body(&self, ui: &mut egui::Ui, state: &State, armed: bool) {
        if state.views.is_empty() {
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
        }
        let count = state.views.len();
        if count > 1 {
            ui.heading(format!("{count} offene Anfragen von diesem Gerät"));
            ui.add_space(6.0);
        }
        for (index, view) in state.views.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            self.request_block(ui, state, index, view, armed);
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(SHARED_ANSWERS).wrap());
        if let Some(error) = &state.error {
            ui.colored_label(theme::danger(ui), error.as_str());
        }
    }

    /// One open request of the device with its own answers.
    fn request_block(
        &self,
        ui: &mut egui::Ui,
        state: &State,
        index: usize,
        view: &ApprovalView,
        armed: bool,
    ) {
        ui.heading(request_line(view));
        ui.add_space(6.0);
        egui::Grid::new(("access_consent_facts", index))
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
        ui.add_space(6.0);
        let id = view.id.as_str();
        let idle = state.sending.is_none();
        let allow = armed && idle && approval_context_complete(view);
        let sending = state.sending.as_deref() == Some(id);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(allow, egui::Button::new("Diesmal erlauben"))
                .clicked()
            {
                self.answer(ui.ctx(), id, ApprovalDecision::Once);
            }
            if ui
                .add_enabled(allow, egui::Button::new("Immer erlauben"))
                .clicked()
            {
                self.answer(ui.ctx(), id, ApprovalDecision::Always);
            }
            if ui
                .add_enabled(idle, egui::Button::new("Ablehnen"))
                .clicked()
            {
                self.answer(ui.ctx(), id, ApprovalDecision::Deny);
            }
            if sending {
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
        if state.gone && state.seen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let listed: Vec<ApprovalId> = state.views.iter().map(|view| view.id.clone()).collect();
        if listed != self.listed {
            // Every change moves the blocks: a request that appears, and one that
            // ends under the pointer, arm the allowing answers anew, so a second
            // click cannot allow the request that moved into place (C5).
            self.shown_at = Some(Instant::now());
            if listed.iter().any(|id| !self.listed.contains(id)) {
                let attention = egui::UserAttentionType::Critical;
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(attention));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            self.listed = listed;
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
        // Closing refuses the requests the window shows (C5). An update stop is
        // no answer. A request that came after the last read was never shown: it
        // stays open, and the worker shows it again when this window has exited
        // (`window_ended`). A window whose requests all ended refuses nothing.
        let state = self.snapshot();
        if self.update_stop || state.gone || state.sending.is_some() {
            return;
        }
        for view in &state.views {
            let _ = crate::daemon::decide_access_approval(view.id.as_str(), ApprovalDecision::Deny);
        }
    }
}

/// Reads the requests once a second until the window's device has none left.
/// A window channel (E46) that stays unreachable belongs to a worker that is
/// gone (update stop, exit) and its requests with it: the window closes then.
fn poll(id: &ApprovalId, slot: &Weak<Mutex<State>>, ctx: &egui::Context) {
    let channel = crate::share::consent_launch::in_window_channel();
    let mut failures = 0u32;
    while let Some(state) = slot.upgrade() {
        let listed = crate::daemon::access_approvals();
        let mut found = None;
        let gone = {
            let mut state = lock(&state);
            match listed {
                Ok(pending) => {
                    failures = 0;
                    if state.device.is_none() {
                        found = pending.iter().find(|view| &view.id == id).cloned();
                        state.device = found.as_ref().map(|view| view.device_id.clone());
                    }
                    match state.device.clone() {
                        Some(device) => {
                            state.views = pending
                                .into_iter()
                                .filter(|view| view.device_id == device)
                                .collect();
                            state.seen = true;
                            state.error = None;
                            state.gone = state.views.is_empty();
                        }
                        None => state.gone = true,
                    }
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    if channel && failures >= CHANNEL_GONE_AFTER {
                        state.seen = true;
                        state.gone = true;
                    }
                    state.error = Some(error);
                }
            }
            state.gone
        };
        // E46: the window's own request, listed for the first time.
        if let Some(view) = &found {
            notify_from_window(view);
        }
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
