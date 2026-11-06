//! Access between this device and its paired devices on the desktop (plan
//! 2026-11-06, R1/R3/R5/R9): the files rights of a grant together with the
//! folders it sees, the rejected sessions the worker reports
//! (`ShareRouteSnapshot::access`) and the gaps the profiles show
//! (`lifecycle_view::access_gaps`), each with the action that allows it.
use super::*;
use crate::app::device_page_layout as layout;
use crate::app::share_rights_choice_ui as rights_ui;
use crate::share::connection_rights::{FolderChoice, RightsTarget};
use crate::share::lifecycle_view::access_gaps::{
    access_gaps, reconfirm_all_direct_grants, AccessGap, AccessGapKind,
};

/// Files (authorization, folders, write right), then screen and commands.
pub(super) fn grant_rights(app: &mut App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) {
    ui.label(RichText::new("Dateien").strong());
    super::super::lifecycle_ui::ui_authorized_device(app, ui, &grant.device_id);
    folders_line(app, ui, grant);
    let mut write = grant.write;
    let editable = grant.write || grant.state == crate::share::DirectGrantState::Accepted;
    if ui
        .add_enabled(
            editable,
            egui::Checkbox::new(&mut write, "Darf in meine Freigaben schreiben"),
        )
        .changed()
    {
        let peer = crate::share::DirectPeerIdentity {
            device_id: grant.device_id.clone(),
            device_name: grant.device_name.clone(),
            public_key: grant.public_key.clone(),
            node_id: grant.node_id.clone(),
            fingerprint: grant.fingerprint.clone(),
        };
        match crate::share::set_direct_peer_write(home(), &peer, write) {
            Ok(change) => {
                app.share_profiles = change.profiles;
                let _ = app.configure_share_service();
            }
            Err(error) => app.error_msg = Some(format!("Schreibrecht: {error}")),
        }
    }
    if !editable {
        ui.label("Freigabe inaktiv; zuerst ausdrücklich wieder erlauben.");
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Bildschirm und Befehle").strong());
    crate::app::device_rights_ui::ui(app, ui, &grant.device_id, &grant.public_key);
}

/// R5: which of my folders the device sees, next to the write right, with
/// the way to change it. „Darf schreiben“ without a folder gives nothing.
fn folders_line(app: &mut App, ui: &mut egui::Ui, grant: &crate::share::DirectGrant) {
    let exports = &app.share_profiles.default_direct_exports;
    let choice = exports.folder_choice_of(&grant.device_id, &grant.public_key);
    let (text, empty) = match &choice {
        FolderChoice::All if exports.roots.is_empty() => {
            ("Sieht: alle Ordner (du gibst noch keinen Ordner frei)".to_string(), true)
        }
        FolderChoice::All => ("Sieht: alle meine Direkt-Ordner".to_string(), false),
        FolderChoice::None => (
            "Sieht: keinen Ordner (Rechte offen oder „Keine Ordner“) – das Gerät sieht einen leeren Ordner".to_string(),
            true,
        ),
        FolderChoice::Selected { paths, connections } => (
            format!(
                "Sieht: {} Ordner{}",
                paths.len(),
                if *connections { " und meine Verbindungen" } else { "" }
            ),
            false,
        ),
    };
    ui.horizontal_wrapped(|ui| {
        if empty {
            ui.colored_label(theme::warning(ui), text);
        } else {
            ui.label(text);
        }
        if ui.button("Ordner und Rechte festlegen …").clicked() {
            let target = RightsTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
            };
            rights_ui::open_for_target(app, ui.ctx(), &target);
        }
    });
}

/// One rejected session as the worker reports it.
struct Notice {
    direction: String,
    device_id: String,
    device_name: Option<String>,
    message: String,
    remedy: String,
    count: u64,
    retry_in_secs: Option<u64>,
}

fn reported(app: &App) -> Vec<Notice> {
    app.share_routes
        .as_ref()
        .map(|routes| {
            routes
                .access
                .iter()
                .map(|notice| Notice {
                    direction: notice.direction.clone(),
                    device_id: notice.device_id.clone(),
                    device_name: notice.device_name.clone(),
                    message: notice.message.clone(),
                    remedy: notice.remedy.clone(),
                    count: notice.count,
                    retry_in_secs: notice.retry_in_secs,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The rejections and gaps of one device, on its page.
pub(super) fn device_notices(app: &mut App, ui: &mut egui::Ui, device_id: &str) {
    let notices: Vec<Notice> = reported(app)
        .into_iter()
        .filter(|notice| notice.device_id == device_id)
        .collect();
    let gaps: Vec<AccessGap> = access_gaps(&app.share_profiles)
        .into_iter()
        .filter(|gap| gap.device_id == device_id)
        .collect();
    if notices.is_empty() && gaps.is_empty() {
        return;
    }
    layout::card(ui, "Zugriff", None, |ui| {
        for notice in &notices {
            notice_line(ui, notice);
        }
        for gap in &gaps {
            gap_line(app, ui, gap);
        }
    });
}

fn notice_line(ui: &mut egui::Ui, notice: &Notice) {
    let title = if notice.direction == "outgoing" {
        "Abgelehnt vom anderen Gerät"
    } else {
        "Hier abgelehnt"
    };
    ui.colored_label(theme::warning(ui), format!("{title}: {}", notice.message));
    let mut detail = format!("{} ({}×)", notice.remedy, notice.count);
    if let Some(secs) = notice.retry_in_secs {
        detail.push_str(&format!(" · nächster Versuch in {secs} s"));
    }
    ui.label(RichText::new(detail).small().color(theme::muted(ui)));
}

fn gap_line(app: &mut App, ui: &mut egui::Ui, gap: &AccessGap) {
    ui.horizontal_wrapped(|ui| {
        ui.label(&gap.message);
        if ui.button(&gap.action).clicked() {
            gap_action(app, ui, gap);
        }
    });
}

fn gap_action(app: &mut App, ui: &mut egui::Ui, gap: &AccessGap) {
    let result = match gap.kind {
        AccessGapKind::NotShared => match &gap.contact_id {
            Some(contact) => crate::share::set_direct_share_back(home(), contact, true),
            None => return,
        },
        AccessGapKind::Reconfirm => crate::share::allow_direct_peer_again(home(), &gap.device_id),
        AccessGapKind::RightsPending => {
            if let Some(grant) = app.share_profiles.grant_for(&gap.device_id) {
                let target = RightsTarget::Direct {
                    device_id: grant.device_id.clone(),
                    public_key: grant.public_key.clone(),
                };
                rights_ui::open_for_target(app, ui.ctx(), &target);
            }
            return;
        }
    };
    match result {
        Ok(change) => {
            app.share_profiles = change.profiles;
            let _ = app.configure_share_service();
        }
        Err(error) => app.error_msg = Some(error),
    }
}

/// The overview line: devices rejected here or rejecting this device and
/// the grants that wait for re-confirmation, with „Alle neu bestätigen“
/// (R9).
pub(super) fn overview_banner(app: &mut App, ui: &mut egui::Ui) {
    let gaps = access_gaps(&app.share_profiles);
    let reconfirm = gaps
        .iter()
        .filter(|gap| gap.kind == AccessGapKind::Reconfirm)
        .count();
    let rejected = reported(app);
    if reconfirm == 0 && rejected.is_empty() {
        return;
    }
    layout::card(ui, "Zugriff", None, |ui| {
        if reconfirm > 0 {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    theme::warning(ui),
                    format!(
                        "{reconfirm} Gerät(e) warten auf die erneute Bestätigung (Direkt-Code erneuert oder Identität repariert) und erreichen deine Freigaben bis dahin nicht."
                    ),
                );
                if ui.button("Alle neu bestätigen").clicked() {
                    match reconfirm_all_direct_grants(home()) {
                        Ok(change) => {
                            app.share_profiles = change.profiles;
                            let _ = app.configure_share_service();
                        }
                        Err(error) => app.error_msg = Some(error),
                    }
                }
            });
        }
        for notice in &rejected {
            let name = notice
                .device_name
                .clone()
                .unwrap_or_else(|| notice.device_id.clone());
            ui.label(RichText::new(name).strong());
            notice_line(ui, notice);
        }
    });
}

fn home() -> Option<String> {
    Some(dirs_home().to_string_lossy().replace('\\', "/"))
}
