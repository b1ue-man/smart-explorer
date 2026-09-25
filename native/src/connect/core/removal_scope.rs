//! What a removed connection leaves behind outside the Share profile, as pure
//! values: the location-key scope of the removed endpoint (favourites,
//! per-folder preferences, sync-job endpoints), the mounts that target it, and
//! the cleanup report. Location keys are the desktop `favorites.txt` /
//! `dir_sort.tsv` keys (`location_key`).
use std::collections::HashMap;

use crate::creds::SavedConnection;

/// Which persisted locations belong to the removed connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedEndpointScope {
    /// Location-key prefixes (`share://direct/<id>`, `sftp://u@h:22`, ...).
    pub prefixes: Vec<String>,
    pub mounts: MountScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountScope {
    DirectContact(String),
    Room(String),
    SavedRemote(String),
    None,
}

impl RemovedEndpointScope {
    pub fn for_direct_contact(contact_id: &str) -> Self {
        Self {
            prefixes: vec![format!("share://direct/{contact_id}")],
            mounts: MountScope::DirectContact(contact_id.to_string()),
        }
    }

    /// Room endpoints are keyed by the profile id in the GUI and by the wire
    /// room id on the CLI; both spellings are cleaned.
    pub fn for_room(room_profile_id: &str, room_id: &str) -> Self {
        let mut prefixes = vec![format!("share://room/{room_profile_id}")];
        if room_id != room_profile_id && !room_id.is_empty() {
            prefixes.push(format!("share://room/{room_id}"));
        }
        Self {
            prefixes,
            mounts: MountScope::Room(room_id.to_string()),
        }
    }

    pub fn for_saved_connection(connection: &SavedConnection) -> Self {
        let prefixes = if connection.protocol.is_url() {
            vec![format!(
                "{}://{}@{}:{}",
                connection.protocol.as_str(),
                connection.user,
                connection.host,
                connection.port
            )]
        } else {
            // UNC shares are browsed under their bare path; favourites store
            // it forward-slashed.
            let root = connection.root.replace('\\', "/");
            let root = root.trim_end_matches('/').to_string();
            if root.is_empty() {
                Vec::new()
            } else {
                vec![root]
            }
        };
        Self {
            prefixes,
            mounts: MountScope::SavedRemote(connection.account()),
        }
    }

    /// A location key belongs to the scope when it is the prefix itself or a
    /// path below it (`prefix/...`). Room prefixes match every device of the
    /// room the same way.
    pub fn matches_key(&self, key: &str) -> bool {
        let key = key.trim_end_matches('/');
        self.prefixes.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            key == prefix || key.starts_with(&format!("{prefix}/"))
        })
    }

    pub(super) fn matches_mount(&self, source: &crate::mount::MountSource) -> bool {
        use crate::mount::{MountSource, PeerMountTarget};
        match (&self.mounts, source) {
            (
                MountScope::DirectContact(contact_id),
                MountSource::Peer {
                    target:
                        PeerMountTarget::Direct {
                            contact_id: mounted,
                        },
                    ..
                },
            ) => contact_id == mounted,
            (
                MountScope::Room(room_id),
                MountSource::Peer {
                    target:
                        PeerMountTarget::RoomDevice {
                            room_id: mounted, ..
                        },
                    ..
                },
            ) => room_id == mounted,
            (
                MountScope::SavedRemote(account),
                MountSource::SavedRemote {
                    account: mounted, ..
                },
            ) => account == mounted,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub favorites_removed: usize,
    pub dir_sort_removed: usize,
    pub mounts_stopped: usize,
    /// Open GUI tabs on the removed endpoint that were closed.
    pub tabs_closed: usize,
    /// The mount daemon could not be asked; mounts may remain.
    pub mount_error: Option<String>,
    pub file_errors: Vec<String>,
    /// Names of sync jobs that still reference the removed endpoint.
    pub orphaned_sync_jobs: Vec<String>,
}

impl CleanupReport {
    /// One-line suffix for CLI output.
    pub fn summary_suffix(&self) -> String {
        let mut parts = Vec::new();
        if self.favorites_removed > 0 {
            parts.push(format!("favorites removed={}", self.favorites_removed));
        }
        if self.dir_sort_removed > 0 {
            parts.push(format!("folder prefs removed={}", self.dir_sort_removed));
        }
        if self.mounts_stopped > 0 {
            parts.push(format!("mounts stopped={}", self.mounts_stopped));
        }
        if let Some(error) = &self.mount_error {
            parts.push(format!("mounts not checked: {error}"));
        }
        for error in &self.file_errors {
            parts.push(error.clone());
        }
        if !self.orphaned_sync_jobs.is_empty() {
            parts.push(format!(
                "orphaned sync jobs kept: {}",
                self.orphaned_sync_jobs.join(", ")
            ));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("; {}", parts.join("; "))
        }
    }

    /// Human-readable lines for the GUI notice / error surface.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.favorites_removed > 0 {
            lines.push(format!("{} Favorit(en) entfernt", self.favorites_removed));
        }
        if self.dir_sort_removed > 0 {
            lines.push(format!(
                "{} Ordner-Einstellung(en) entfernt",
                self.dir_sort_removed
            ));
        }
        if self.mounts_stopped > 0 {
            lines.push(format!("{} Laufwerk(e) getrennt", self.mounts_stopped));
        }
        if self.tabs_closed > 0 {
            lines.push(format!("{} Tab(s) geschlossen", self.tabs_closed));
        }
        if let Some(error) = &self.mount_error {
            lines.push(format!("Laufwerke nicht geprueft: {error}"));
        }
        lines.extend(self.file_errors.iter().cloned());
        if !self.orphaned_sync_jobs.is_empty() {
            lines.push(format!(
                "Verwaiste Sync-Jobs behalten: {}",
                self.orphaned_sync_jobs.join(", ")
            ));
        }
        lines
    }
}

/// A re-openable, connection-namespaced key for a location: a bare path
/// locally, or `proto://user@host:port/path` on a remote (`endpoint_prefix` of
/// the open connection) — so favourites and per-folder prefs bind to the
/// connection (the "link id"), not just a path.
pub fn location_key(endpoint_prefix: Option<&str>, path: &str) -> String {
    let p = path.replace('\\', "/").trim_end_matches('/').to_string();
    match endpoint_prefix {
        Some(prefix) => format!("{}{}", prefix, p),
        None => p,
    }
}

/// Pure half of the favourites cleanup: `(kept lines, removed count)`.
pub(super) fn filter_favorites(text: &str, scope: &RemovedEndpointScope) -> (Vec<String>, usize) {
    let mut kept = Vec::new();
    let mut removed = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        if scope.matches_key(line) {
            removed += 1;
        } else {
            kept.push(line.to_string());
        }
    }
    (kept, removed)
}

pub(super) fn filter_dir_sort(
    map: &mut HashMap<String, bool>,
    scope: &RemovedEndpointScope,
) -> usize {
    let before = map.len();
    map.retain(|key, _| !scope.matches_key(key));
    before - map.len()
}

pub(super) fn job_references_scope(
    job: &crate::syncjobs::SyncJob,
    scope: &RemovedEndpointScope,
) -> bool {
    scope.matches_key(&job.source) || scope.matches_key(&job.target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creds::{AuthKind, Protocol};

    #[test]
    fn lan_cleanup_task_direct_scope_matches_only_its_own_paths() {
        let scope = RemovedEndpointScope::for_direct_contact("abc");
        assert!(scope.matches_key("share://direct/abc"));
        assert!(scope.matches_key("share://direct/abc/"));
        assert!(scope.matches_key("share://direct/abc/Docs/Sub"));
        assert!(!scope.matches_key("share://direct/abcd/Docs"));
        assert!(!scope.matches_key("share://room/abc/Docs"));
        assert!(!scope.matches_key("/home/user/abc"));
    }

    #[test]
    fn lan_cleanup_task_saved_url_connection_scope_uses_the_endpoint_prefix() {
        let connection = SavedConnection {
            protocol: Protocol::Sftp,
            host: "example.com".into(),
            port: 2222,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: "/srv".into(),
            label: "prod".into(),
            use_agent: false,
        };
        let scope = RemovedEndpointScope::for_saved_connection(&connection);
        assert_eq!(scope.prefixes, ["sftp://alice@example.com:2222"]);
        assert!(scope.matches_key("sftp://alice@example.com:2222/srv/data"));
        assert!(!scope.matches_key("sftp://alice@example.com:22/srv"));
    }

    #[test]
    fn lan_cleanup_task_unc_connection_scope_uses_the_forward_slashed_root() {
        let connection = SavedConnection {
            protocol: Protocol::Share,
            host: String::new(),
            port: 0,
            user: "bob".into(),
            auth: AuthKind::Password,
            root: r"\\server\share".into(),
            label: "NAS".into(),
            use_agent: false,
        };
        let scope = RemovedEndpointScope::for_saved_connection(&connection);
        assert_eq!(scope.prefixes, ["//server/share"]);
        assert!(scope.matches_key("//server/share/photos"));
        assert!(!scope.matches_key("//server/share2"));
    }

    #[test]
    fn lan_cleanup_task_favourites_and_prefs_filter_by_scope() {
        let scope = RemovedEndpointScope::for_room("profile-1", "room-9");
        let text = "share://room/profile-1/dev-a/Docs\n/local/path\nshare://room/room-9/dev-b\n\n";
        let (kept, removed) = filter_favorites(text, &scope);
        assert_eq!(removed, 2);
        assert_eq!(kept, ["/local/path"]);
        let mut map = HashMap::from([
            ("share://room/profile-1/dev-a/Docs".to_string(), true),
            ("/local/path".to_string(), false),
        ]);
        assert_eq!(filter_dir_sort(&mut map, &scope), 1);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn lan_cleanup_task_report_suffix_lists_only_what_happened() {
        let mut report = CleanupReport::default();
        assert_eq!(report.summary_suffix(), "");
        report.favorites_removed = 2;
        report.orphaned_sync_jobs = vec!["Backup".into()];
        let suffix = report.summary_suffix();
        assert!(suffix.contains("favorites removed=2"));
        assert!(suffix.contains("orphaned sync jobs kept: Backup"));
    }
}
