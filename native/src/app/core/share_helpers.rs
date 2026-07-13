use super::*;

pub(super) fn share_value_field(ui: &mut egui::Ui, value: &str) -> egui::Response {
    let mut text = value.to_string();
    let width = share_input_width(ui, 420.0);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::TextStyle::Monospace)
            .desired_width(width)
            .clip_text(true)
            .interactive(false),
    )
}

pub(super) fn share_input_width(ui: &egui::Ui, preferred: f32) -> f32 {
    let available = ui.available_width().max(64.0);
    preferred.min(available).max(64.0)
}

pub(super) fn selected_room_label(app: &App) -> String {
    app.share_profiles
        .rooms
        .iter()
        .find(|room| room.id == app.share_export_target_id)
        .map(|room| room.name.clone())
        .unwrap_or_else(|| "Raum waehlen".into())
}

pub(super) fn export_summary(cfg: &crate::share::ShareExportConfig) -> String {
    let mut parts = Vec::new();
    if cfg.roots.is_empty() {
        parts.push("keine Ordner".to_string());
    } else if cfg.roots.len() == 1 {
        parts.push(format!("1 Ordner ({})", cfg.roots[0].label));
    } else {
        parts.push(format!("{} Ordner/Laufwerke", cfg.roots.len()));
    }
    if cfg.include_connections {
        parts.push("gespeicherte Verbindungen".to_string());
    }
    parts.join(", ")
}

pub(super) fn share_open_result_is_current(
    opening: Option<&crate::share::PeerOpenTarget>,
    active: Option<&crate::share::PeerOpenTarget>,
    opening_origin: Option<&str>,
    current_origin: &str,
) -> bool {
    match (opening, active, opening_origin) {
        (Some(opening), Some(active), Some(origin)) => {
            opening == active && origin == current_origin
        }
        _ => false,
    }
}

pub(super) fn trim_share_diag_log(log: &mut String) {
    if log.len() <= SHARE_DIAG_MAX_BYTES {
        return;
    }
    let keep_from = log.len() - SHARE_DIAG_MAX_BYTES;
    let drain_to = log[keep_from..]
        .find('\n')
        .map(|idx| keep_from + idx + 1)
        .unwrap_or(keep_from);
    log.drain(..drain_to);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_share_open_result_is_rejected() {
        let target = crate::share::PeerOpenTarget::Direct {
            contact_id: "contact-a".to_string(),
        };
        let other = crate::share::PeerOpenTarget::Direct {
            contact_id: "contact-b".to_string(),
        };

        assert!(share_open_result_is_current(
            Some(&target),
            Some(&target),
            Some("local|C:/Work"),
            "local|C:/Work"
        ));
        assert!(!share_open_result_is_current(
            Some(&target),
            Some(&target),
            Some("local|C:/Work"),
            "local|C:/Other"
        ));
        assert!(!share_open_result_is_current(
            Some(&target),
            Some(&other),
            Some("local|C:/Work"),
            "local|C:/Work"
        ));
        assert!(!share_open_result_is_current(
            Some(&target),
            Some(&target),
            None,
            "local|C:/Work"
        ));
    }

    #[test]
    fn share_diag_log_is_bounded_on_line_boundary() {
        let mut log = String::new();
        for i in 0..3000 {
            log.push_str(&format!("line {i:04} {}\n", "x".repeat(40)));
            trim_share_diag_log(&mut log);
        }
        assert!(log.len() <= SHARE_DIAG_MAX_BYTES);
        assert!(!log.starts_with('x'));
        assert!(log.contains("line 2999"));
    }
}
