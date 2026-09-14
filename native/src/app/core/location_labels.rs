//! Display names for location keys (favourites, landing tiles).
//!
//! A key is either a bare local path or a connection-namespaced endpoint URL
//! (`share://direct/<id>/Docs`, `sftp://u@host:22/srv`, `gdrive:///x`). Remote
//! keys are labelled `"<Remote> › <Ordner>"` so the same folder name on two
//! devices stays distinguishable; the remote name is resolved live from the
//! current Share profile and saved connections.
use super::*;

/// Last path segment of a key, or the key itself for a bare root.
pub(in crate::app) fn location_basename(key: &str) -> String {
    let trimmed = key.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if base.is_empty() {
        key.to_string()
    } else {
        base.to_string()
    }
}

impl App {
    /// The remote a key belongs to, when it is an endpoint URL.
    pub(in crate::app) fn location_remote_name(&self, key: &str) -> Option<String> {
        if let Some((target, _)) = crate::share::PeerOpenTarget::from_endpoint(key) {
            return Some(match target {
                crate::share::PeerOpenTarget::Direct { contact_id } => self
                    .share_profiles
                    .direct_contacts
                    .iter()
                    .find(|contact| contact.id == contact_id)
                    .map(|contact| contact.display_name.clone())
                    .unwrap_or_else(|| "Direkt (entfernt)".to_string()),
                crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                    let room = self
                        .share_profiles
                        .rooms
                        .iter()
                        .find(|room| room.id == room_id || room.room_id == room_id);
                    let device = room.and_then(|room| {
                        room.members
                            .iter()
                            .find(|member| member.device_id == device_id)
                            .map(|member| member.device_name.clone())
                    });
                    match (room, device) {
                        (Some(room), Some(device)) => format!("{} / {}", room.name, device),
                        (Some(room), None) => format!("{} / {}", room.name, device_id),
                        (None, _) => "Raum (entfernt)".to_string(),
                    }
                }
            });
        }
        if key.starts_with("gdrive://") {
            return Some("Google Drive".to_string());
        }
        if crate::connect::is_remote_url(key) {
            let (proto, user, host, port, _) = crate::connect::parse_remote_url(key)?;
            let saved = self.saved_connections.iter().find(|connection| {
                connection.protocol == proto
                    && connection.user == user
                    && connection.host == host
                    && connection.port == port
            });
            return Some(match saved {
                Some(connection) if !connection.label.trim().is_empty() => {
                    connection.label.clone()
                }
                _ if user.is_empty() => host,
                _ => format!("{user}@{host}"),
            });
        }
        None
    }

    /// `"<Remote> › <Ordner>"` for endpoint keys, the folder name for local
    /// paths. The key itself stays the hover text.
    pub(in crate::app) fn location_label(&self, key: &str) -> String {
        let Some(remote) = self.location_remote_name(key) else {
            return location_basename(key);
        };
        let path = if let Some((_, path)) = crate::share::PeerOpenTarget::from_endpoint(key) {
            path
        } else if let Some((_, _, _, _, path)) = crate::connect::parse_remote_url(key) {
            path
        } else if let Some(rest) = key.strip_prefix("gdrive://") {
            rest.to_string()
        } else {
            key.to_string()
        };
        let folder = if path.trim_matches('/').is_empty() {
            "/".to_string()
        } else {
            location_basename(&path)
        };
        format!("{remote} › {folder}")
    }
}

impl App {
    /// A location key whose connection no longer exists: a Share peer/room
    /// that was removed or a remote URL without a saved connection. Local
    /// paths and cloud roots never count as orphaned here.
    pub(in crate::app) fn location_is_orphaned(&self, key: &str) -> bool {
        if let Some((target, _)) = crate::share::PeerOpenTarget::from_endpoint(key) {
            return match target {
                crate::share::PeerOpenTarget::Direct { contact_id } => !self
                    .share_profiles
                    .direct_contacts
                    .iter()
                    .any(|contact| contact.id == contact_id),
                crate::share::PeerOpenTarget::RoomDevice { room_id, .. } => !self
                    .share_profiles
                    .rooms
                    .iter()
                    .any(|room| room.id == room_id || room.room_id == room_id),
            };
        }
        if key.starts_with("gdrive://") || !crate::connect::is_remote_url(key) {
            return false;
        }
        match crate::connect::parse_remote_url(key) {
            Some((proto, user, host, port, _)) => !self.saved_connections.iter().any(|c| {
                c.protocol == proto && c.user == user && c.host == host && c.port == port
            }),
            None => false,
        }
    }

    pub(in crate::app) fn sync_job_is_orphaned(&self, job: &crate::syncjobs::SyncJob) -> bool {
        self.location_is_orphaned(&job.source) || self.location_is_orphaned(&job.target)
    }
}

#[cfg(test)]
mod tests {
    use super::location_basename;

    #[test]
    fn lan_cleanup_task_basename_of_keys() {
        assert_eq!(location_basename("/home/user/Docs"), "Docs");
        assert_eq!(location_basename("/home/user/Docs/"), "Docs");
        assert_eq!(location_basename("C:"), "C:");
        assert_eq!(location_basename("/"), "/");
        assert_eq!(location_basename("share://direct/abc/Gate/Sub"), "Sub");
    }
}
