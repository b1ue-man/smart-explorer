//! Tab „Befehle“ and the screen/command card of a device (PRM-M9): the open
//! requests on top, one line per device with its access and „Bildschirm- und
//! Befehlsrechte …“, then the jobs, this host's access log and „Konten
//! dieses Geräts“ (`share_access_accounts_ui.rs`); the card of one device
//! with its access, „Bildschirm- und Befehlsrechte ändern …“ (the dialog,
//! `share_access_dialog_ui.rs`) and „Zugriff entziehen“. Every change goes
//! to the worker through `Access::MutateGrant` at the revision shown.
use super::App;
use crate::app::theme;
use crate::share::access_grant::{AccessGrant, AccessGrantChange, Capability, CapabilityDecision};
use eframe::egui::{self, RichText};

#[path = "share_access_dialog_ui.rs"]
mod access_dialog;
#[path = "share_access_room_ui.rs"]
mod access_room;
#[path = "share_access_accounts_ui.rs"]
mod accounts_ui;
#[path = "share_access_approvals_ui.rs"]
mod approvals_ui;
#[path = "share_exec_log_ui.rs"]
mod log_ui;
#[path = "share_exec_programs_ui.rs"]
mod programs_ui;

#[cfg(test)]
use access_dialog::{activation_ready, activation_warning, exec_warning};
use access_dialog::{AccessDraft, DialogAction};
pub(in crate::app) use access_room::ui_room_access;

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
    programs: Vec<String>,
    /// The access right: account, decisions and end (C1).
    access: AccessGrant,
    /// Wire room id of a room member.
    room_id: Option<String>,
}

/// Tab „Befehle“: the open requests, one line per device with its access,
/// the jobs and the access log; „Einstellungen“ returns the target key whose
/// card the caller opens.
pub(in crate::app) fn ui_exec_overview(app: &mut App, ui: &mut egui::Ui) -> Option<String> {
    approvals_ui::ui_pending(ui);
    let views = exec_device_views(&app.share_profiles);
    intro(ui);
    let provider = cached_provider_status(ui);
    provider_line(ui, &provider);
    let now = crate::share::core_now_secs();
    let mut open = None;
    if views.is_empty() {
        ui.label(
            "Noch kein gekoppeltes Gerät und kein Raummitglied, dem Zugriff gegeben werden kann.",
        );
    }
    for view in &views {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&view.device_name).strong());
            ui.label(
                RichText::new(&view.relation)
                    .small()
                    .color(theme::muted(ui)),
            );
            let text = access_text(view, &provider.user_label, now);
            ui.colored_label(state_color(ui, view, now), text);
            if ui.button("Bildschirm- und Befehlsrechte …").clicked() {
                open = Some(view.target_key.clone());
            }
        });
    }
    run_dialog(app, ui.ctx(), &views, &provider);
    ui.separator();
    crate::app::share_exec_jobs_ui::ui_exec_jobs(ui);
    ui.separator();
    log_ui::ui_exec_log(ui);
    ui.separator();
    // Account administration belongs to „Dieses Gerät“.
    open
}

/// The access section of one device (device page and tab „Befehle“);
/// `false` when the key names no current device.
pub(in crate::app) fn ui_exec_target(app: &mut App, ui: &mut egui::Ui, target_key: &str) -> bool {
    ui_exec_card(app, ui, target_key, false)
}

/// The access card of one target; `compact` (device page) leaves out the
/// device name, the identity and the rights list the page already shows.
pub(in crate::app) fn ui_exec_card(
    app: &mut App,
    ui: &mut egui::Ui,
    target_key: &str,
    compact: bool,
) -> bool {
    let views = exec_device_views(&app.share_profiles);
    let Some(view) = views
        .iter()
        .find(|view| view.target_key == target_key)
        .cloned()
    else {
        return false;
    };
    let provider = cached_provider_status(ui);
    provider_line(ui, &provider);
    let now = crate::share::core_now_secs();
    if !compact {
        identity_and_rights(ui, &view, now);
    }
    let account = view.access.account.label(&provider.user_label);
    ui.label(format!("Ausführen als: {account}"));
    if !view.programs.is_empty() {
        let programs = view.programs.join(", ");
        ui.label(format!(
            "Nur diese Programme (keine Shell, kein Terminal): {programs}"
        ));
    }
    ui.label(match view.access.valid_until {
        Some(end) if view.access.expired_at(now) => format!("Abgelaufen am {}", end_text(end)),
        Some(end) => format!("Gültig bis {}", end_text(end)),
        None => "Gültig: unbegrenzt".into(),
    });

    if !view.base_authorized {
        let text = "Die Freigabe dieses Geräts ist inaktiv; Zugriff kann erst nach der Freigabe eingestellt werden.";
        ui.colored_label(theme::warning(ui), text);
    }
    let (mut edit, mut revoke) = (false, false);
    ui.horizontal_wrapped(|ui| {
        edit = ui
            .add_enabled(
                view.base_authorized,
                egui::Button::new("Bildschirm- und Befehlsrechte ändern …"),
            )
            .clicked();
        if view.access.grants_any() {
            revoke = ui
                .button("Zugriff entziehen")
                .on_hover_text(access_dialog::NARROWING)
                .clicked();
        }
    });
    if edit {
        open_dialog(ui.ctx(), &app.share_profiles, &view);
    }
    if revoke {
        let capabilities = Capability::ALL.map(|capability| (capability, CapabilityDecision::Off));
        let change = AccessGrantChange {
            capabilities: capabilities.into_iter().collect(),
            ..AccessGrantChange::default()
        };
        apply_access(app, &view, change, view.policy_revision);
    }
    run_dialog(app, ui.ctx(), &views, &provider);
    true
}

/// Key of the Exec target of a direct authorization (see `exec_targets.rs`).
pub(in crate::app) fn direct_target_key(device_id: &str, fingerprint: &str) -> String {
    format!("direct/{device_id}/{fingerprint}")
}

/// Key of the Exec target of a room member.
pub(in crate::app) fn room_target_key(room_id: &str, device_id: &str, fingerprint: &str) -> String {
    format!("room/{room_id}/{device_id}/{fingerprint}")
}

/// The access of a target as one line, for device pages.
pub(in crate::app) fn exec_summary(
    app: &App,
    ui: &mut egui::Ui,
    target_key: &str,
) -> Option<String> {
    let own = cached_provider_status(ui).user_label;
    let now = crate::share::core_now_secs();
    exec_device_views(&app.share_profiles)
        .into_iter()
        .find(|view| view.target_key == target_key)
        .map(|view| access_text(&view, &own, now))
}

fn summary(view: &ExecDeviceView) -> String {
    match (view.enabled, view.programs.len()) {
        (false, _) => "aus".into(),
        (true, 0) => "volle Shell".into(),
        (true, 1) => "1 Programm".into(),
        (true, count) => format!("{count} Programme"),
    }
}

fn decision_word(decision: CapabilityDecision) -> &'static str {
    match decision {
        CapabilityDecision::Off => "aus",
        CapabilityDecision::Ask => "mit Rückfrage",
        CapabilityDecision::Allow => "erlaubt",
    }
}

/// „Befehle: erlaubt · Terminal: erlaubt · als {Konto}“ (C6), or „aus“.
fn access_text(view: &ExecDeviceView, own_label: &str, now: i64) -> String {
    let access = &view.access;
    if !access.grants_exec() || access.expired_at(now) {
        let ended = access.grants_exec() && access.expired_at(now);
        return if ended {
            "aus (abgelaufen)".into()
        } else {
            "aus".into()
        };
    }
    let mut commands = decision_word(access.decision(Capability::ExecCommands)).to_string();
    if !view.programs.is_empty() && view.enabled {
        commands = format!("{commands} ({})", summary(view));
    }
    let terminal = decision_word(access.decision(Capability::ExecTerminal));
    let account = access.account.label(own_label);
    let end = access
        .valid_until
        .map(|end| format!(" · bis {}", end_text(end)))
        .unwrap_or_default();
    format!("Befehle: {commands} · Terminal: {terminal} · als {account}{end}")
}

fn state_color(ui: &egui::Ui, view: &ExecDeviceView, now: i64) -> egui::Color32 {
    let access = &view.access;
    let on = |capability| access.decision_at(capability, now);
    match (on(Capability::ExecCommands), on(Capability::ExecTerminal)) {
        (CapabilityDecision::Off, CapabilityDecision::Off) => theme::success(ui),
        (CapabilityDecision::Allow, _) | (_, CapabilityDecision::Allow)
            if view.programs.is_empty() =>
        {
            theme::danger(ui)
        }
        _ => theme::warning(ui),
    }
}

/// Local date and time of an end, e.g. `17.10.2026 18:00`.
fn end_text(end: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(end, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Zeitpunkt nicht darstellbar".into())
}

fn intro(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("ZUGRIFF PRO GERÄT")
            .small()
            .color(theme::muted(ui)),
    );
    ui.small(
        "Dateifreigaben erlauben nie Befehle. Zugriff gilt nur für die angezeigte, exakt gepinnte Geräteidentität; Befehle laufen mit den Rechten des gewählten Kontos.",
    );
}

fn provider_line(ui: &mut egui::Ui, provider: &crate::share::ExecProviderStatus) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Mein Konto: {}", provider.user_label));
        egui::CollapsingHeader::new("Kontodiagnose")
            .id_salt("exec_provider_diagnostics")
            .show(ui, |ui| {
                ui.monospace(format!("Kapselung: {}", provider.provider));
            });
        if !provider.available {
            ui.colored_label(
                theme::danger(ui),
                format!("nicht verfügbar — {}", provider.detail),
            );
        }
    });
}

/// Opens the access dialog of `view` with the accounts this host offers.
fn open_dialog(ctx: &egui::Context, profiles: &crate::share::ShareProfiles, view: &ExecDeviceView) {
    let right = (view.access.clone(), view.programs.clone());
    let accounts = programs_ui::account_options(&exec_device_views(profiles));
    let draft = AccessDraft::of(
        &view.target_key,
        &view.device_name,
        view.policy_revision,
        right,
        accounts,
    );
    access_dialog::open(ctx, draft);
}

fn run_dialog(
    app: &mut App,
    ctx: &egui::Context,
    views: &[ExecDeviceView],
    provider: &crate::share::ExecProviderStatus,
) {
    let now = crate::share::core_now_secs();
    let Some(DialogAction::Apply {
        key,
        change,
        revision,
    }) = access_dialog::show(ctx, provider, views, now)
    else {
        return;
    };
    if let Some(view) = views.iter().find(|view| view.target_key == key) {
        if apply_access(app, view, change, revision) {
            access_dialog::close(ctx);
        } else if let Some(mut draft) = access_dialog::current(ctx) {
            draft.save_error = app.error_msg.clone();
            access_dialog::open(ctx, draft);
        }
    }
}

/// One change at the revision the surface showed; the answer as notice or
/// error with the next step (C6).
fn apply_access(
    app: &mut App,
    view: &ExecDeviceView,
    change: AccessGrantChange,
    revision: u64,
) -> bool {
    let mut saved = false;
    let name = &view.device_name;
    match crate::daemon::mutate_access_grant(view.target.clone(), change, Some(revision)) {
        Ok((result, _)) if result.persisted && result.applied && result.error.is_none() => {
            let text = format!("Zugriff für {name} ({}) gespeichert", view.relation);
            app.notice = Some((text, std::time::Instant::now()));
            saved = true;
        }
        Ok((result, _)) => {
            let detail = result
                .error
                .unwrap_or_else(|| "der Worker wendet sie noch an".into());
            app.error_msg = Some(format!(
                "Zugriff für {name} nicht vollständig übernommen (gespeichert={}, angewendet={}): {detail}",
                result.persisted, result.applied
            ));
        }
        Err(error) => {
            let next = if error.contains("changed concurrently") {
                ". Nächster Schritt: „Ändern…“ erneut öffnen; der Dialog zeigt den aktuellen Stand."
            } else if error.contains("another account") {
                ". Nächster Schritt: „Mein Konto“ wählen."
            } else {
                ""
            };
            app.error_msg = Some(format!("Zugriff für {name} nicht geändert: {error}{next}"));
        }
    }
    app.share_next_poll_at = std::time::Instant::now();
    saved
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

fn exec_device_views(profiles: &crate::share::ShareProfiles) -> Vec<ExecDeviceView> {
    crate::share::exec_target_views(profiles)
        .into_iter()
        .map(|view| {
            let no_name = match &view.target {
                crate::share::ExecGrantTarget::Direct {
                    device_id,
                    public_key,
                    ..
                } => profiles
                    .direct_grants
                    .iter()
                    .find(|grant| &grant.device_id == device_id && &grant.public_key == public_key)
                    .is_some_and(|grant| grant.device_name.trim().is_empty()),
                crate::share::ExecGrantTarget::RoomMember {
                    room_id,
                    device_id,
                    public_key,
                    ..
                } => profiles
                    .rooms
                    .iter()
                    .filter(|room| &room.room_id == room_id)
                    .flat_map(|room| &room.members)
                    .find(|member| {
                        &member.device_id == device_id && &member.public_key == public_key
                    })
                    .is_some_and(|member| member.device_name.trim().is_empty()),
            };
            let mut view = device_view(view);
            if no_name {
                view.device_name = "Gerät ohne Namen".into();
            }
            view
        })
        .collect()
}

fn device_view(view: crate::share::ExecTargetView) -> ExecDeviceView {
    let relation = match view.relation {
        crate::share::ExecTargetRelation::Direct => "Direktgerät".to_string(),
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
        programs: view.programs,
        access: view.access,
        room_id: view.room_id,
    }
}

/// Device name, relation, identity and the four rights of one access card.
fn identity_and_rights(ui: &mut egui::Ui, view: &ExecDeviceView, now: i64) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&view.device_name).strong());
        ui.label(&view.relation);
    });
    egui::CollapsingHeader::new("Identität abgleichen")
        .id_salt(("access_identity", &view.target_key))
        .show(ui, |ui| {
            ui.label("Fingerabdruck");
            ui.monospace(&view.fingerprint);
            if ui.button("Fingerabdruck kopieren").clicked() {
                ui.ctx().copy_text(view.fingerprint.clone());
            }
        });
    for (capability, label) in [
        (Capability::ScreenView, "Bildschirm ansehen"),
        (Capability::ScreenControl, "Bildschirm steuern"),
        (Capability::ExecCommands, "Befehle"),
        (Capability::ExecTerminal, "Terminal"),
    ] {
        let decision = view.access.decision_at(capability, now);
        ui.label(format!("{label}: {}", decision_word(decision)));
    }
}

#[cfg(test)]
#[path = "share_access_lists_tests.rs"]
mod access_lists_tests;
#[cfg(test)]
#[path = "share_access_ui_tests.rs"]
mod access_ui_tests;
#[cfg(test)]
#[path = "share_access_accounts_ui_tests.rs"]
mod accounts_ui_tests;
#[cfg(test)]
#[path = "share_exec_ui_tests.rs"]
mod tests;

/// Account management is owned by the local device settings.
pub(in crate::app) fn ui_host_accounts(ui: &mut egui::Ui) {
    accounts_ui::ui_accounts(ui);
}
