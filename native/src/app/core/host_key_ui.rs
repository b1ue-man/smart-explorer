//! Desktop confirmation of an unknown SSH host key. A connection that met an
//! unknown key fails with the key's fingerprint (`sftp::UnknownHostKey`); the
//! status bar then offers "Hostschlüssel prüfen …", a window shows type and
//! fingerprint, and "Vertrauen" pins the key only when the server presents
//! exactly that fingerprint again. The person then connects again.
use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText};

use super::App;
use crate::app::theme;

type TrustResult = Arc<Mutex<Option<Result<(), String>>>>;

#[derive(Clone)]
struct Pending {
    key: crate::sftp::UnknownHostKey,
    result: Option<TrustResult>,
}

fn state_id() -> egui::Id {
    egui::Id::new("ssh_host_key_confirmation")
}

/// Status-bar button next to an error that names an unknown host key.
pub(in crate::app) fn status_button(app: &App, ui: &mut egui::Ui) {
    let Some(key) = app
        .error_msg
        .as_deref()
        .and_then(crate::sftp::unknown_host_key)
    else {
        return;
    };
    if ui
        .small_button("Hostschlüssel prüfen …")
        .on_hover_text("Fingerabdruck des Servers anzeigen und nach Vergleich speichern")
        .clicked()
    {
        ui.data_mut(|data| data.insert_temp(state_id(), Pending { key, result: None }));
    }
}

/// The confirmation window while one is open.
pub(in crate::app) fn window(app: &mut App, ctx: &egui::Context) {
    let Some(mut pending) = ctx.data_mut(|data| data.get_temp::<Pending>(state_id())) else {
        return;
    };
    if let Some(result) = pending
        .result
        .as_ref()
        .and_then(|result| result.lock().ok().and_then(|mut slot| slot.take()))
    {
        ctx.data_mut(|data| data.remove::<Pending>(state_id()));
        match result {
            Ok(()) => {
                app.error_msg = None;
                app.notice = Some((
                    format!(
                        "Hostschlüssel von {}:{} gespeichert – bitte erneut verbinden",
                        pending.key.host, pending.key.port
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => app.error_msg = Some(error),
        }
        return;
    }
    let mut open = true;
    let mut close = false;
    egui::Window::new("SSH-Hostschlüssel prüfen")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let key = &pending.key;
            ui.label(format!(
                "Der Server {}:{} ist noch nicht bekannt.",
                key.host, key.port
            ));
            ui.label(format!("Schlüsseltyp: {}", key.algorithm));
            ui.horizontal_wrapped(|ui| {
                ui.label("Fingerabdruck:");
                ui.monospace(&key.fingerprint);
            });
            ui.colored_label(
                theme::warning(ui),
                "Nur vertrauen, wenn der Fingerabdruck mit dem des Servers übereinstimmt \
                 (z. B. `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` auf dem Server).",
            );
            if pending.result.is_some() {
                ui.label(RichText::new("Prüfe den Server erneut …").italics());
                return;
            }
            ui.horizontal(|ui| {
                if ui.button("Vertrauen und speichern").clicked() {
                    let result: TrustResult = Arc::new(Mutex::new(None));
                    let slot = result.clone();
                    let key = key.clone();
                    std::thread::spawn(move || {
                        let outcome = crate::sftp::trust_host_key(
                            &key.host,
                            key.port,
                            &key.fingerprint,
                            false,
                        )
                        .map(|_| ())
                        .map_err(|error| format!("Hostschlüssel nicht gespeichert: {error}"));
                        if let Ok(mut slot) = slot.lock() {
                            *slot = Some(outcome);
                        }
                    });
                    pending.result = Some(result);
                }
                if ui.button("Abbrechen").clicked() {
                    close = true;
                }
            });
        });
    if !open || close {
        ctx.data_mut(|data| data.remove::<Pending>(state_id()));
    } else {
        ctx.data_mut(|data| data.insert_temp(state_id(), pending));
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}
