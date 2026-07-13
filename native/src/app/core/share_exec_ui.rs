use super::App;
use eframe::egui::{self, Color32, RichText};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecDeviceView {
    target: crate::share::ExecGrantTarget,
    target_key: String,
    relation: String,
    device_id: String,
    device_name: String,
    fingerprint: String,
    enabled: bool,
    policy_revision: u64,
    base_authorized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecWarning {
    full_access: String,
    elevated: Option<&'static str>,
}

pub(in crate::app) fn ui_exec_grants(app: &mut App, ui: &mut egui::Ui) {
    let views = exec_device_views(&app.share_profiles);
    ui.label(
        RichText::new("REMOTE-AUSFUEHRUNG PRO GERAET")
            .small()
            .color(Color32::from_gray(140)),
    );
    ui.small(
        "Dateifreigaben erlauben niemals automatisch Codeausfuehrung. Exec gilt nur fuer die angezeigte, exakt gepinnte Geraeteidentitaet.",
    );
    if views.is_empty() {
        ui.label("Keine exakten Direkt-Freigaben oder Raummitglieder vorhanden.");
        ui.separator();
        crate::app::share_exec_jobs_ui::ui_exec_jobs(ui);
        return;
    }

    let provider = cached_provider_status(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Lokale Shell: {}", provider.user_label));
        ui.label(format!("Kapselung: {}", provider.provider));
        if !provider.available {
            ui.colored_label(
                Color32::from_rgb(230, 100, 100),
                format!("nicht verfuegbar — {}", provider.detail),
            );
        }
    });

    let mut action = None;
    for view in &views {
        exec_device_card(ui, view, &provider, &mut action);
    }
    if let Some((target, enabled)) = action {
        apply_exec_grant(app, target, enabled);
    }
    ui.separator();
    crate::app::share_exec_jobs_ui::ui_exec_jobs(ui);
}

fn exec_device_card(
    ui: &mut egui::Ui,
    view: &ExecDeviceView,
    provider: &crate::share::ExecProviderStatus,
    action: &mut Option<(crate::share::ExecGrantTarget, bool)>,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&view.device_name).strong());
            ui.label(&view.relation);
        });
        ui.label(format!("Geraet-ID: {}", view.device_id));
        ui.horizontal_wrapped(|ui| {
            ui.label("Fingerprint:");
            ui.monospace(&view.fingerprint);
        });
        let status = if view.enabled {
            "AKTIVIERT — volle Remote-Codeausfuehrung erlaubt"
        } else {
            "DEAKTIVIERT — keine Remote-Codeausfuehrung"
        };
        let color = if view.enabled {
            Color32::from_rgb(235, 105, 95)
        } else {
            Color32::from_rgb(120, 205, 145)
        };
        ui.colored_label(color, RichText::new(status).strong());
        ui.small(format!("Exec-Policy-Revision: {}", view.policy_revision));

        if !view.base_authorized {
            ui.colored_label(
                Color32::from_rgb(255, 185, 120),
                "Basisfreigabe ist inaktiv; Exec kann nicht aktiviert werden.",
            );
        }
        if view.enabled {
            if ui.button("Exec sofort deaktivieren").clicked() {
                *action = Some((view.target.clone(), false));
                clear_confirmation(ui, view);
            }
        } else {
            activation_controls(ui, view, provider, action);
        }
    });
    ui.add_space(4.0);
}

fn activation_controls(
    ui: &mut egui::Ui,
    view: &ExecDeviceView,
    provider: &crate::share::ExecProviderStatus,
    action: &mut Option<(crate::share::ExecGrantTarget, bool)>,
) {
    let armed_id = confirmation_id(view, "armed");
    let armed = ui.data_mut(|data| data.get_temp::<bool>(armed_id).unwrap_or_default());
    if !armed {
        let can_prepare = view.base_authorized;
        if ui
            .add_enabled(
                can_prepare,
                egui::Button::new("Remote-Ausfuehrung aktivieren …"),
            )
            .clicked()
        {
            ui.data_mut(|data| data.insert_temp(armed_id, true));
        }
        return;
    }

    let warning = exec_warning(provider);
    ui.colored_label(
        Color32::from_rgb(245, 95, 85),
        RichText::new(&warning.full_access).strong(),
    );
    if let Some(elevated) = warning.elevated {
        ui.colored_label(
            Color32::from_rgb(245, 70, 70),
            RichText::new(elevated).strong(),
        );
    }
    ui.label(
        "Dieses Geraet kann danach ohne Befehlsfilter beliebige Programme und Shell-Befehle mit den Rechten des Smart-Explorer-Prozesses starten.",
    );

    let understood_id = confirmation_id(view, "understood");
    let mut understood =
        ui.data_mut(|data| data.get_temp::<bool>(understood_id).unwrap_or_default());
    if ui
        .checkbox(
            &mut understood,
            "Ich bestaetige diese uneingeschraenkte Codeausfuehrung bewusst",
        )
        .changed()
    {
        ui.data_mut(|data| data.insert_temp(understood_id, understood));
    }
    ui.horizontal_wrapped(|ui| {
        let can_enable = activation_ready(understood, provider, view.base_authorized);
        if ui
            .add_enabled(can_enable, egui::Button::new("Exec jetzt aktivieren"))
            .clicked()
        {
            *action = Some((view.target.clone(), true));
            clear_confirmation(ui, view);
        }
        if ui.button("Abbrechen").clicked() {
            clear_confirmation(ui, view);
        }
    });
}

fn activation_ready(
    understood: bool,
    provider: &crate::share::ExecProviderStatus,
    base_authorized: bool,
) -> bool {
    understood && provider.available && base_authorized
}

fn cached_provider_status(ui: &mut egui::Ui) -> crate::share::ExecProviderStatus {
    let id = egui::Id::new("share_exec_provider_status");
    if let Some(status) = ui.data_mut(|data| data.get_temp::<crate::share::ExecProviderStatus>(id))
    {
        return status;
    }
    let status = crate::share::exec_provider_status();
    ui.data_mut(|data| data.insert_temp(id, status.clone()));
    status
}

fn confirmation_id(view: &ExecDeviceView, part: &'static str) -> egui::Id {
    egui::Id::new((
        "share_exec_confirmation",
        &view.target_key,
        view.policy_revision,
        part,
    ))
}

fn clear_confirmation(ui: &mut egui::Ui, view: &ExecDeviceView) {
    for part in ["armed", "understood"] {
        ui.data_mut(|data| data.remove::<bool>(confirmation_id(view, part)));
    }
}

fn apply_exec_grant(app: &mut App, target: crate::share::ExecGrantTarget, enabled: bool) {
    match crate::daemon::mutate_exec_grant(target, enabled) {
        Ok(result) if result.persisted && result.applied && result.error.is_none() => {
            let state = if enabled { "aktiviert" } else { "deaktiviert" };
            app.notice = Some((
                format!(
                    "Exec {state}; Policy-Revision {} dauerhaft gespeichert und im Worker angewendet",
                    result.revision
                ),
                std::time::Instant::now(),
            ));
            app.share_next_poll_at = std::time::Instant::now();
        }
        Ok(result) => {
            let detail = result
                .error
                .unwrap_or_else(|| "Worker-Anwendung steht noch aus".into());
            app.error_msg = Some(format!(
                "Exec-Aenderung nicht vollstaendig (gespeichert={}, angewendet={}): {detail}",
                result.persisted, result.applied
            ));
            app.share_next_poll_at = std::time::Instant::now();
        }
        Err(error) => {
            app.error_msg = Some(format!("Exec-Aenderung fehlgeschlagen: {error}"));
        }
    }
}

fn exec_device_views(profiles: &crate::share::ShareProfiles) -> Vec<ExecDeviceView> {
    let mut views = Vec::new();
    for grant in &profiles.direct_grants {
        let target = crate::share::ExecGrantTarget::Direct {
            device_id: grant.device_id.clone(),
            public_key: grant.public_key.clone(),
            fingerprint: grant.fingerprint.clone(),
            node_id: grant.node_id.clone(),
        };
        views.push(ExecDeviceView {
            target_key: format!("direct/{}/{}", grant.device_id, grant.fingerprint),
            target,
            relation: "Direktgeraet".into(),
            device_id: grant.device_id.clone(),
            device_name: display_name(&grant.device_name, &grant.device_id),
            fingerprint: grant.fingerprint.clone(),
            enabled: grant.exec.enabled,
            policy_revision: grant.exec.policy_revision,
            base_authorized: grant.state == crate::share::DirectGrantState::Accepted,
        });
    }
    for room in &profiles.rooms {
        for member in &room.members {
            let target = crate::share::ExecGrantTarget::RoomMember {
                room_id: room.room_id.clone(),
                device_id: member.device_id.clone(),
                public_key: member.public_key.clone(),
                fingerprint: member.fingerprint.clone(),
                node_id: member.node_id.clone(),
            };
            views.push(ExecDeviceView {
                target_key: format!(
                    "room/{}/{}/{}",
                    room.room_id, member.device_id, member.fingerprint
                ),
                target,
                relation: format!("Raum: {}", room.name),
                device_id: member.device_id.clone(),
                device_name: display_name(&member.device_name, &member.device_id),
                fingerprint: member.fingerprint.clone(),
                enabled: member.exec.enabled,
                policy_revision: member.exec.policy_revision,
                base_authorized: room.auto_join && !member.blocked,
            });
        }
    }
    views.sort_by(|left, right| {
        left.device_name
            .cmp(&right.device_name)
            .then_with(|| left.target_key.cmp(&right.target_key))
    });
    views
}

fn display_name(name: &str, device_id: &str) -> String {
    if name.trim().is_empty() {
        format!("Geraet {}", &device_id[..device_id.len().min(8)])
    } else {
        name.to_string()
    }
}

fn exec_warning(provider: &crate::share::ExecProviderStatus) -> ExecWarning {
    ExecWarning {
        full_access: format!("FULL {} CODE EXECUTION", provider.user_label),
        elevated: provider.elevated.then_some(
            if provider.user_label.eq_ignore_ascii_case("root") {
                "REMOTE ROOT SHELL"
            } else {
                "REMOTE ADMINISTRATOR SHELL"
            },
        ),
    }
}

#[cfg(test)]
#[path = "share_exec_ui_tests.rs"]
mod tests;
