//! The window „Tresor entsperren“ and the question „Tresor sperren?“ (U29-5).
//!
//! Unlocking runs on the thread `vault-unlock` (`daemon::unlock_vault`; the
//! key derivation takes as long as the vault's scrypt needs, so there is no
//! time limit, only a spinner). The password stays in a masked field, is
//! taken out of it when sent and wiped after the request. Locking asks the
//! worker; with unsaved changes the worker answers `vault_unsaved_changes`
//! and the window offers „Sperren, Änderungen verschlüsselt aufbewahren“.
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;
use zeroize::Zeroize;

use super::vault_feed::{guard, poke, poll, spawn, Poll};
use super::vault_models::{lock_call, lock_step, unlock_outcome, Failure, LockStep, UnlockDone};
use super::vault_sidebar_ui::Effect;

type UnlockAnswer = Result<UnlockDone, Failure>;
type LockAnswer = Result<Vec<String>, Failure>;

struct UnlockDialog {
    site: String,
    label: String,
    password: String,
    remember: bool,
    /// The site has a remembered password.
    saved: bool,
    error: Option<String>,
    rx: Option<Receiver<UnlockAnswer>>,
    /// Open the vault as a tab after the unlock.
    then_open: bool,
    focus: bool,
}

struct KeepDialog {
    site: Option<String>,
    label: String,
}

struct LockJob {
    rx: Option<Receiver<LockAnswer>>,
    site: Option<String>,
    label: String,
    keep_unsaved: bool,
}

#[derive(Default)]
struct State {
    unlock: Option<UnlockDialog>,
    keep: Option<KeepDialog>,
    locks: Vec<LockJob>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

fn failure(error: crate::daemon::VaultIpcError) -> Failure {
    Failure {
        code: error.code,
        message: error.message,
    }
}

/// Closes the windows of this module (the render test of the harness).
#[cfg(test)]
pub(crate) fn reset_for_test() {
    *guard(state()) = State::default();
}

/// Opens the unlock window for `site`.
pub(crate) fn ask(site: &str, label: &str, saved: bool, remember: bool, then_open: bool) {
    guard(state()).unlock = Some(UnlockDialog {
        site: site.to_string(),
        label: label.to_string(),
        password: String::new(),
        remember,
        saved,
        error: None,
        rx: None,
        then_open,
        focus: true,
    });
}

/// Locks `site` (or every site with `None`); unsaved changes lead to the
/// question of the window.
pub(crate) fn lock(
    ctx: &egui::Context,
    site: Option<&str>,
    label: &str,
    keep_unsaved: bool,
) -> Vec<Effect> {
    let call = lock_call(site, keep_unsaved);
    let work = call.clone();
    let rx = spawn(ctx, "vault-lock", move || -> LockAnswer {
        crate::daemon::lock_vaults(work.site.as_deref(), work.keep_unsaved).map_err(failure)
    });
    match rx {
        Ok(rx) => {
            guard(state()).locks.push(LockJob {
                rx: Some(rx),
                site: call.site,
                label: label.to_string(),
                keep_unsaved,
            });
            Vec::new()
        }
        Err(error) => vec![Effect::Error(error)],
    }
}

/// Draws the windows and collects the answers of the threads.
pub(crate) fn show(ctx: &egui::Context) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut held = guard(state());
    let state: &mut State = &mut held;
    // Answers of locks.
    let mut index = 0;
    while index < state.locks.len() {
        let job = &mut state.locks[index];
        match poll(&mut job.rx) {
            Poll::Pending => {
                index += 1;
                continue;
            }
            Poll::Gone => effects.push(Effect::Error(
                "Das Sperren endete ohne Antwort des Dienstes.".to_string(),
            )),
            Poll::Ready(answer) => match lock_step(&answer) {
                LockStep::Done(sites) => {
                    effects.push(Effect::Locked(sites));
                }
                LockStep::AskKeep if !job.keep_unsaved => {
                    let ask = KeepDialog {
                        site: job.site.clone(),
                        label: job.label.clone(),
                    };
                    state.keep = Some(ask);
                }
                LockStep::AskKeep => effects.push(Effect::Error(
                    "Das Sperren mit aufbewahrten Änderungen wurde abgelehnt.".to_string(),
                )),
                LockStep::Failed(text) => effects.push(Effect::Error(text)),
            },
        }
        state.locks.remove(index);
        poke();
    }
    show_keep(ctx, state, &mut effects);
    show_unlock(ctx, state, &mut effects);
    effects
}

fn show_keep(ctx: &egui::Context, state: &mut State, effects: &mut Vec<Effect>) {
    let Some(keep) = state.keep.as_ref() else {
        return;
    };
    let (site, label) = (keep.site.clone(), keep.label.clone());
    let mut choice: Option<bool> = None;
    egui::Window::new("Tresor sperren?")
        .id(egui::Id::new("vault_lock_keep"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(&label).strong());
            ui.label("Es gibt noch nicht gespeicherte Änderungen in diesem Tresor.");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Sperren, Änderungen verschlüsselt aufbewahren")
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui.button("Abbrechen").clicked() {
                    choice = Some(false);
                }
            });
        });
    match choice {
        Some(true) => {
            state.keep = None;
            effects.extend(lock_with(ctx, state, site.as_deref(), &label));
        }
        Some(false) => state.keep = None,
        None => {}
    }
}

/// Starts the lock that keeps unsaved changes (the state is held by the
/// caller, so the job is added to it directly).
fn lock_with(
    ctx: &egui::Context,
    state: &mut State,
    site: Option<&str>,
    label: &str,
) -> Vec<Effect> {
    let call = lock_call(site, true);
    let work = call.clone();
    match spawn(ctx, "vault-lock", move || -> LockAnswer {
        crate::daemon::lock_vaults(work.site.as_deref(), work.keep_unsaved).map_err(failure)
    }) {
        Ok(rx) => {
            state.locks.push(LockJob {
                rx: Some(rx),
                site: call.site,
                label: label.to_string(),
                keep_unsaved: true,
            });
            Vec::new()
        }
        Err(error) => vec![Effect::Error(error)],
    }
}

fn show_unlock(ctx: &egui::Context, state: &mut State, effects: &mut Vec<Effect>) {
    let Some(dialog) = state.unlock.as_mut() else {
        return;
    };
    // The answer of the thread.
    let mut close = false;
    match poll(&mut dialog.rx) {
        Poll::Pending => {}
        Poll::Gone => {
            dialog.error = Some("Das Entsperren endete ohne Antwort des Dienstes.".to_string());
            dialog.focus = true;
        }
        Poll::Ready(answer) => {
            let view = unlock_outcome(&answer);
            if view.clear_password {
                dialog.password.zeroize();
            }
            if let Some(notice) = view.notice {
                effects.push(Effect::Notice(notice));
            }
            if view.keep_open {
                dialog.error = view.error_line;
                dialog.focus = true;
            } else {
                effects.push(Effect::Unlocked {
                    site: dialog.site.clone(),
                    then_open: dialog.then_open,
                });
                close = true;
            }
        }
    }
    if close {
        state.unlock = None;
        poke();
        return;
    }
    let busy = dialog.rx.is_some();
    let mut submit = false;
    let mut use_saved = false;
    let mut cancel = false;
    egui::Window::new("Tresor entsperren")
        .id(egui::Id::new("vault_unlock"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(&dialog.label).strong());
            ui.add_space(4.0);
            if dialog.saved {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new("Mit gespeichertem Passwort entsperren"),
                    )
                    .clicked()
                {
                    use_saved = true;
                }
                ui.separator();
            }
            ui.label("Passwort");
            let edit = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut dialog.password)
                    .password(true)
                    .desired_width(280.0),
            );
            if dialog.focus && !busy {
                edit.request_focus();
                dialog.focus = false;
            }
            let enter = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut dialog.remember, "Passwort merken"),
            );
            if let Some(error) = &dialog.error {
                ui.colored_label(crate::app::theme::danger(ui), error);
            }
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        "Wird entsperrt … (das Passwort wird geprüft, das dauert einen Moment)",
                    );
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let ready = !busy && !dialog.password.is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Entsperren"))
                    .clicked()
                    || (enter && ready)
                {
                    submit = true;
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Abbrechen"))
                    .clicked()
                {
                    cancel = true;
                }
            });
        });
    if cancel {
        dialog.password.zeroize();
        state.unlock = None;
        return;
    }
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    if submit || use_saved {
        dialog.error = None;
        let site = dialog.site.clone();
        let remember = dialog.remember;
        // The password leaves the field here; the thread wipes its copy.
        let mut password = std::mem::take(&mut dialog.password);
        let started = spawn(ctx, "vault-unlock", move || -> UnlockAnswer {
            let answer = if use_saved {
                crate::daemon::unlock_vault_saved(&site)
            } else {
                crate::daemon::unlock_vault(&site, &password, Some(remember))
            };
            password.zeroize();
            answer
                .map(|unlocked| UnlockDone {
                    remember_error: unlocked.remember_error,
                })
                .map_err(failure)
        });
        match started {
            Ok(rx) => dialog.rx = Some(rx),
            Err(error) => dialog.error = Some(error),
        }
    }
}
