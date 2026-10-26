//! Presentation of exact policy sources. This module makes no access decision.
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::access_relation_view::{AccessRelationView, DeviceAccessView};
use crate::share::ExecGrantTarget;

pub(super) fn target_key(target: &ExecGrantTarget) -> String {
    match target {
        ExecGrantTarget::Direct {
            device_id,
            fingerprint,
            ..
        } => format!("direct/{device_id}/{fingerprint}"),
        ExecGrantTarget::RoomMember {
            room_id,
            device_id,
            fingerprint,
            ..
        } => format!("room/{room_id}/{device_id}/{fingerprint}"),
    }
}
pub(super) fn capability_label(capability: Capability) -> &'static str {
    match capability {
        Capability::ExecCommands => "Befehle",
        Capability::ExecTerminal => "Terminal",
        Capability::ScreenView => "Bildschirm ansehen",
        Capability::ScreenControl => "Bildschirm steuern",
        Capability::AppLaunch => "Apps starten",
        Capability::LoginAccess => "Anmeldezugang",
        Capability::TcpForward => "TCP weiterleiten",
    }
}
pub(super) fn decision_label(decision: CapabilityDecision) -> &'static str {
    match decision {
        CapabilityDecision::Off => "Aus",
        CapabilityDecision::Ask => "Mit Rückfrage",
        CapabilityDecision::Allow => "Erlaubt",
    }
}
pub(super) fn source_name(relation: &AccessRelationView) -> String {
    if relation.relation_kind == "direct" {
        "Direktfreigabe".into()
    } else {
        format!("Raum „{}“", relation.relation_name)
    }
}
pub(super) fn has_files(relation: &AccessRelationView) -> bool {
    !relation.exports.roots.is_empty()
        || relation.exports.shares_connections()
        || relation.exports.include_connections
}
pub(super) fn remaining_room_access(view: &DeviceAccessView) -> bool {
    view.relations.iter().any(|relation| {
        relation.relation_kind == "room"
            && relation.active
            && (has_files(relation)
                || view
                    .reachable
                    .iter()
                    .any(|right| right.source == relation.target))
    })
}
pub(super) fn expiry(end: Option<i64>) -> String {
    end.map(|end| {
        chrono::DateTime::<chrono::Utc>::from_timestamp(end, 0)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%d.%m.%Y %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "Zeitpunkt nicht darstellbar".into())
    })
    .unwrap_or_else(|| "Unbegrenzt".into())
}

pub(super) fn condition_lines(relation: &AccessRelationView) -> Vec<String> {
    let mut lines = vec![
        format!("Konto: {}", relation.grant.account_label),
        format!("Gültigkeit: {}", expiry(relation.grant.valid_until)),
    ];
    if !relation.programs.is_empty() {
        lines.push(format!("Programme: {}", relation.programs.join(", ")));
    }
    let conditions = &relation.grant.constraints;
    if let Some(app) = &conditions.app_launch {
        lines.push(format!(
            "Apps: {}",
            app.app_ids
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "App-Benutzer: {} · Sitzungen: {} · Ende: {}",
            app.users
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            app.sessions
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            expiry(app.valid_until)
        ));
    }
    if let Some(login) = &conditions.login_access {
        lines.push(format!(
            "Anmeldebenutzer: {} · Sitzungen: {} · Ende: {}",
            login
                .users
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            login
                .sessions
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            expiry(login.valid_until)
        ));
        lines.push(format!(
            "Anmeldevorgänge: {}",
            login
                .operations
                .iter()
                .map(|operation| match operation {
                    crate::share::access_constraints::LoginOperation::Login => "Anmelden",
                    crate::share::access_constraints::LoginOperation::Unlock => "Entsperren",
                    crate::share::access_constraints::LoginOperation::SecureDesktop =>
                        "Geschützter Desktop",
                    crate::share::access_constraints::LoginOperation::UserSession =>
                        "Benutzersitzung",
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for rule in &conditions.tcp_forward {
        use crate::share::access_constraints::{TcpDirection, TcpHost};
        let host = match &rule.host {
            TcpHost::Loopback => "Loopback des Zielgeräts".into(),
            TcpHost::Ip { address } => address.to_string(),
            TcpHost::Dns { name, addresses } => format!(
                "{name} ({})",
                addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let direction = match rule.direction {
            TcpDirection::PeerToHost => "Gegenstelle → dieses Gerät",
            TcpDirection::HostToPeer => "Dieses Gerät → Gegenstelle",
        };
        lines.push(format!(
            "TCP: {direction} · Ziel {host} · Ports {} · Ende {}",
            rule.ports
                .iter()
                .map(|port| port.get().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            expiry(rule.valid_until)
        ));
    }
    if relation.grant.legacy_reader_disables_grant {
        lines.push(
            "Eine ältere App ohne diese Einschränkungen deaktiviert diese Freigabe vollständig."
                .into(),
        );
    }
    lines
}
