use super::App;
use crate::app::theme;
use eframe::egui::{self, RichText};

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
    run_as: crate::share::ExecRunAs,
    programs: Vec<String>,
}

/// A grant change from the cards: target, enabled, and new run-as settings
/// (`None` keeps the grant's own).
type ExecAction = (
    crate::share::ExecGrantTarget,
    bool,
    Option<crate::share::ExecGrantSettings>,
);

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
            .color(theme::muted(ui)),
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
                theme::danger(ui),
                format!("nicht verfuegbar — {}", provider.detail),
            );
        }
    });

    let mut action = None;
    for view in &views {
        exec_device_card(ui, view, &provider, &mut action);
    }
    if let Some((target, enabled, settings)) = action {
        apply_exec_grant(app, target, enabled, settings);
    }
    ui.separator();
    crate::app::share_exec_jobs_ui::ui_exec_jobs(ui);
}

fn exec_device_card(
    ui: &mut egui::Ui,
    view: &ExecDeviceView,
    provider: &crate::share::ExecProviderStatus,
    action: &mut Option<ExecAction>,
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
        let status = match (view.enabled, view.programs.is_empty()) {
            (true, true) => "AKTIVIERT — volle Remote-Codeausfuehrung erlaubt",
            (true, false) => "AKTIVIERT — nur die gelisteten Programme, ohne Shell und Terminal",
            (false, _) => "DEAKTIVIERT — keine Remote-Codeausfuehrung",
        };
        let color = match (view.enabled, view.programs.is_empty()) {
            (true, true) => theme::danger(ui),
            (true, false) => theme::warning(ui),
            (false, _) => theme::success(ui),
        };
        ui.colored_label(color, RichText::new(status).strong());
        ui.small(format!("Exec-Policy-Revision: {}", view.policy_revision));
        ui.label(format!(
            "Ausfuehren als: {}",
            run_as_label(&view.run_as, provider)
        ));
        if !view.programs.is_empty() {
            ui.label(format!(
                "Nur diese Programme (keine Shell, kein Terminal): {}",
                view.programs.join(", ")
            ));
        }

        if !view.base_authorized {
            ui.colored_label(
                theme::warning(ui),
                "Basisfreigabe ist inaktiv; Exec kann nicht aktiviert werden.",
            );
        }
        if view.enabled {
            if ui.button("Exec sofort deaktivieren").clicked() {
                *action = Some((view.target.clone(), false, None));
                clear_confirmation(ui, view);
            }
            if crate::share::exec_run_as_supported() {
                account_controls(ui, view, action);
            }
        } else {
            activation_controls(ui, view, provider, action);
        }
    });
    ui.add_space(4.0);
}

/// "Standardnutzer" is the worker's own account, the account every grant
/// made before run-as existed keeps.
fn run_as_label(
    run_as: &crate::share::ExecRunAs,
    provider: &crate::share::ExecProviderStatus,
) -> String {
    match run_as.account_name() {
        None => format!("Standardnutzer ({})", provider.user_label),
        Some(name) => format!("Konto {name}"),
    }
}

/// Account field of a root Linux host: empty means the Standardnutzer.
fn account_field(ui: &mut egui::Ui, view: &ExecDeviceView) -> String {
    let id = confirmation_id(view, "account");
    let mut account = ui.data_mut(|data| {
        data.get_temp::<String>(id)
            .unwrap_or_else(|| view.run_as.account_name().unwrap_or_default().to_string())
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Konto (leer = Standardnutzer):");
        if ui.text_edit_singleline(&mut account).changed() {
            ui.data_mut(|data| data.insert_temp(id, account.clone()));
        }
    });
    account.trim().to_string()
}

fn settings_for(view: &ExecDeviceView, account: &str) -> crate::share::ExecGrantSettings {
    crate::share::ExecGrantSettings {
        run_as: if account.is_empty() {
            crate::share::ExecRunAs::Standard
        } else {
            crate::share::ExecRunAs::Account {
                name: account.to_string(),
            }
        },
        programs: view.programs.clone(),
    }
}

fn account_controls(ui: &mut egui::Ui, view: &ExecDeviceView, action: &mut Option<ExecAction>) {
    let account = account_field(ui, view);
    let changed = account.as_str() != view.run_as.account_name().unwrap_or_default();
    if ui
        .add_enabled(changed, egui::Button::new("Konto uebernehmen"))
        .on_hover_text("Laufende Befehle dieses Geraets enden wie bei jedem Rechtewechsel.")
        .clicked()
    {
        *action = Some((
            view.target.clone(),
            true,
            Some(settings_for(view, &account)),
        ));
        ui.data_mut(|data| data.remove::<String>(confirmation_id(view, "account")));
    }
}

fn activation_controls(
    ui: &mut egui::Ui,
    view: &ExecDeviceView,
    provider: &crate::share::ExecProviderStatus,
    action: &mut Option<ExecAction>,
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
        theme::danger(ui),
        RichText::new(&warning.full_access).strong(),
    );
    if let Some(elevated) = warning.elevated {
        ui.colored_label(theme::danger(ui), RichText::new(elevated).strong());
    }
    ui.label(if view.programs.is_empty() {
        "Dieses Geraet kann danach ohne Befehlsfilter beliebige Programme und Shell-Befehle mit den Rechten des gewaehlten Kontos starten (Standardnutzer: das Konto des Smart-Explorer-Prozesses). Eine Programmliste setzt `se share grants exec enable --allow-program`."
    } else {
        "Dieses Geraet kann danach nur die gelisteten Programme mit freien Argumenten und den Rechten des gewaehlten Kontos starten, ohne Shell und Terminal; nur LANG, LC_* und TERM gelten als Umgebung. Enthaelt die Liste eine Shell, einen Interpreter oder ein Programm, das Befehle ausfuehrt, ist das eine volle Shell."
    });

    let account = if crate::share::exec_run_as_supported() {
        Some(account_field(ui, view))
    } else {
        None
    };

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
            let settings = account
                .as_deref()
                .map(|account| settings_for(view, account));
            *action = Some((view.target.clone(), true, settings));
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
    ui.data_mut(|data| data.remove::<String>(confirmation_id(view, "account")));
}

fn apply_exec_grant(
    app: &mut App,
    target: crate::share::ExecGrantTarget,
    enabled: bool,
    settings: Option<crate::share::ExecGrantSettings>,
) {
    match crate::daemon::mutate_exec_grant(target, enabled, settings) {
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
    crate::share::exec_target_views(profiles)
        .into_iter()
        .map(device_view)
        .collect()
}

fn device_view(view: crate::share::ExecTargetView) -> ExecDeviceView {
    let relation = match view.relation {
        crate::share::ExecTargetRelation::Direct => "Direktgeraet".to_string(),
        crate::share::ExecTargetRelation::Room => {
            format!("Raum: {}", view.room_name.as_deref().unwrap_or_default())
        }
    };
    ExecDeviceView {
        target: view.target,
        target_key: view.target_key,
        relation,
        device_id: view.device_id,
        device_name: view.device_name,
        fingerprint: view.fingerprint,
        enabled: view.enabled,
        policy_revision: view.policy_revision,
        base_authorized: view.base_authorized,
        run_as: view.run_as,
        programs: view.programs,
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
