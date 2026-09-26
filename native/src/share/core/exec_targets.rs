//! Exec grant targets of this device: one per exact direct grant and room
//! member, as the desktop Share page and the Android facade show them, and
//! the lookup of a target key against the current profiles. A key names the
//! device and its fingerprint only; the resolved target repeats every
//! cryptographic pin of the current profile, so a replaced identity fails
//! closed instead of inheriting a grant.
use super::types::{DirectGrantState, ExecGrantTarget};
use super::ShareProfiles;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecTargetRelation {
    Direct,
    Room,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecTargetView {
    pub target: ExecGrantTarget,
    /// `direct/<deviceId>/<fingerprint>` or
    /// `room/<roomId>/<deviceId>/<fingerprint>` (`roomId` = wire room id).
    pub target_key: String,
    pub relation: ExecTargetRelation,
    /// Wire room id and room name of a room member.
    pub room_id: Option<String>,
    pub room_name: Option<String>,
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub enabled: bool,
    pub policy_revision: u64,
    /// The file relation the grant builds on is active (accepted direct
    /// grant, joined room with an unblocked member).
    pub base_authorized: bool,
}

/// Every direct grant and room member, sorted by name, then key.
pub fn exec_target_views(profiles: &ShareProfiles) -> Vec<ExecTargetView> {
    let mut views = Vec::new();
    for grant in &profiles.direct_grants {
        views.push(ExecTargetView {
            target: ExecGrantTarget::Direct {
                device_id: grant.device_id.clone(),
                public_key: grant.public_key.clone(),
                fingerprint: grant.fingerprint.clone(),
                node_id: grant.node_id.clone(),
            },
            target_key: format!("direct/{}/{}", grant.device_id, grant.fingerprint),
            relation: ExecTargetRelation::Direct,
            room_id: None,
            room_name: None,
            device_id: grant.device_id.clone(),
            device_name: display_name(&grant.device_name, &grant.device_id),
            fingerprint: grant.fingerprint.clone(),
            enabled: grant.exec.enabled,
            policy_revision: grant.exec.policy_revision,
            base_authorized: grant.state == DirectGrantState::Accepted,
        });
    }
    for room in &profiles.rooms {
        for member in &room.members {
            views.push(ExecTargetView {
                target: ExecGrantTarget::RoomMember {
                    room_id: room.room_id.clone(),
                    device_id: member.device_id.clone(),
                    public_key: member.public_key.clone(),
                    fingerprint: member.fingerprint.clone(),
                    node_id: member.node_id.clone(),
                },
                target_key: format!(
                    "room/{}/{}/{}",
                    room.room_id, member.device_id, member.fingerprint
                ),
                relation: ExecTargetRelation::Room,
                room_id: Some(room.room_id.clone()),
                room_name: Some(room.name.clone()),
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

/// The target behind `key` in the current profiles (its `target` carries
/// every identity pin); `None` when no or more than one identity matches,
/// since an ambiguous key must not pick one of them.
pub fn resolve_exec_target(profiles: &ShareProfiles, key: &str) -> Option<ExecTargetView> {
    let mut matching = exec_target_views(profiles)
        .into_iter()
        .filter(|view| view.target_key == key);
    let view = matching.next()?;
    matching.next().is_none().then_some(view)
}

fn display_name(name: &str, device_id: &str) -> String {
    if name.trim().is_empty() {
        let short: String = device_id.chars().take(8).collect();
        format!("Geraet {short}")
    } else {
        name.to_string()
    }
}
