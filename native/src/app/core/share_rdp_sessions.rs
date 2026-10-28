//! The Remote Desktop sessions this process owns for one device. A session
//! has no fixed end: it lives as long as its RDP program and the connection
//! (design 2.6, user rule r7 „keinen random ass default timeouts“); its
//! effect on the host's session is shown as a notice (design 2.7 point 5).
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};

/// „ohne feste Frist“ or the rest of an explicitly chosen lifetime.
pub(crate) fn lifetime_text(expires_at: Option<i64>) -> String {
    match expires_at {
        None => "ohne feste Frist (endet mit dem RDP-Programm)".into(),
        Some(at) => format!(
            "endet in {} s (ausdrücklich gewählte Frist)",
            at.saturating_sub(crate::share::core_now_secs()).max(0)
        ),
    }
}

pub(super) fn show(ui: &mut egui::Ui, nodes: &[&str]) {
    ui.label("Native RDP-Sitzungen dieses Geräts");
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(1));
    let id = ui.id().with("rdp-close-result");
    let result: Arc<Mutex<Option<Result<String, String>>>> = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(None)))
            .clone()
    });
    if let Some(value) = &*result.lock().unwrap_or_else(PoisonError::into_inner) {
        match value {
            Ok(message) => {
                ui.label(message);
            }
            Err(message) => {
                ui.colored_label(egui::Color32::RED, message);
            }
        }
    }
    let sessions: Vec<_> = crate::rdp::owned_sessions()
        .into_iter()
        .filter(|s| nodes.contains(&s.tunnel.remote_node.as_str()))
        .collect();
    if sessions.is_empty() {
        ui.label("Keine laufende Remote-Desktop-Sitzung dieses Fensters zu diesem Gerät.");
    }
    for session in sessions {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "{} · Client {} · Prozess {}",
                session.tunnel.id.as_str(),
                session.client.id,
                session.process_id
            ));
            ui.label(&session.session_notice);
            ui.label(format!(
                "RemoteApp: {} · Beziehung: {} / {}",
                session.remote_app.as_deref().unwrap_or("ganzer Desktop"),
                session.tunnel.relation_kind,
                session.tunnel.relation_id
            ));
            ui.label(format!(
                "Lokaler Zugang: {} · {} · {} Verbindungen",
                session
                    .tunnel
                    .listener
                    .map_or_else(|| "nicht vorhanden".into(), |a| a.to_string()),
                lifetime_text(session.tunnel.expires_at),
                session.tunnel.active_connections
            ));
            if session.tunnel.closing {
                ui.label("RDP-Programm und Verbindung werden geschlossen …");
            }
            if let Some(error) = &session.tunnel.last_error {
                ui.colored_label(egui::Color32::RED, error);
            }
            if ui
                .add_enabled(
                    !session.tunnel.closing || session.tunnel.last_error.is_some(),
                    egui::Button::new("RDP-Sitzung schließen"),
                )
                .clicked()
            {
                let (id, state, context) =
                    (session.tunnel.id.clone(), result.clone(), ui.ctx().clone());
                let retry = state.clone();
                if let Err(error) = std::thread::Builder::new()
                    .name("device-rdp-close".into())
                    .spawn(move || {
                        let closed = crate::rdp::close_owned_session(&id).map(|found| {
                            if found {
                                "RDP-Programm und Verbindung geschlossen.".into()
                            } else {
                                "Diese RDP-Sitzung ist bereits beendet.".into()
                            }
                        });
                        *state.lock().unwrap_or_else(PoisonError::into_inner) = Some(closed);
                        context.request_repaint();
                    })
                {
                    *retry.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(Err(error.to_string()));
                }
            }
        });
    }
}
