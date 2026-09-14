//! Everything a removed connection leaves behind outside the Share profile:
//! ★ favourites and per-folder sort preferences keyed by its endpoint prefix,
//! drive mounts that target it, and sync jobs that reference it. Favourites,
//! preferences and mounts are removed; sync jobs are the user's own work and
//! are only reported as orphaned (the Sync UI marks them).
//!
//! Headless and GUI callers share this module; the GUI additionally reloads
//! its in-memory copies after a cleanup (see `App::cleanup_after_removal`).
use std::collections::HashMap;

use crate::creds::SavedConnection;

use super::platform_helpers::{favorites_path, load_dir_sort, save_dir_sort};

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

    fn matches_mount(&self, source: &crate::mount::MountSource) -> bool {
        use crate::mount::{MountSource, PeerMountTarget};
        match (&self.mounts, source) {
            (
                MountScope::DirectContact(contact_id),
                MountSource::Peer {
                    target: PeerMountTarget::Direct { contact_id: mounted },
                    ..
                },
            ) => contact_id == mounted,
            (
                MountScope::Room(room_id),
                MountSource::Peer {
                    target: PeerMountTarget::RoomDevice { room_id: mounted, .. },
                    ..
                },
            ) => room_id == mounted,
            (MountScope::SavedRemote(account), MountSource::SavedRemote { account: mounted, .. }) => {
                account == mounted
            }
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

/// Remove favourites, folder preferences and mounts of `scope` and report
/// sync jobs that still reference it. Never fails as a whole: each store
/// reports its own error and the others are still cleaned.
pub fn cleanup_removed_endpoint_state(scope: &RemovedEndpointScope) -> CleanupReport {
    let mut report = CleanupReport::default();
    if scope.prefixes.is_empty() {
        report.orphaned_sync_jobs = Vec::new();
    } else {
        match remove_favorites(scope) {
            Ok(removed) => report.favorites_removed = removed,
            Err(error) => report
                .file_errors
                .push(format!("Favoriten bereinigen: {error}")),
        }
        match remove_dir_sort(scope) {
            Ok(removed) => report.dir_sort_removed = removed,
            Err(error) => report
                .file_errors
                .push(format!("Ordner-Einstellungen bereinigen: {error}")),
        }
        report.orphaned_sync_jobs = orphaned_sync_jobs(scope);
    }
    match stop_mounts(scope) {
        Ok(stopped) => report.mounts_stopped = stopped,
        Err(error) => report.mount_error = Some(error),
    }
    report
}

fn remove_favorites(scope: &RemovedEndpointScope) -> std::io::Result<usize> {
    let path = favorites_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let (kept, removed) = filter_favorites(&text, scope);
    if removed > 0 {
        std::fs::write(&path, kept.join("\n"))?;
    }
    Ok(removed)
}

/// Pure half of the favourites cleanup: `(kept lines, removed count)`.
pub(crate) fn filter_favorites(text: &str, scope: &RemovedEndpointScope) -> (Vec<String>, usize) {
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

fn remove_dir_sort(scope: &RemovedEndpointScope) -> std::io::Result<usize> {
    let mut map = load_dir_sort();
    let removed = filter_dir_sort(&mut map, scope);
    if removed > 0 {
        save_dir_sort(&map)?;
    }
    Ok(removed)
}

pub(crate) fn filter_dir_sort(map: &mut HashMap<String, bool>, scope: &RemovedEndpointScope) -> usize {
    let before = map.len();
    map.retain(|key, _| !scope.matches_key(key));
    before - map.len()
}

fn stop_mounts(scope: &RemovedEndpointScope) -> Result<usize, String> {
    if scope.mounts == MountScope::None {
        return Ok(0);
    }
    let mounts = crate::daemon::list_mounts()?;
    let mut stopped = 0;
    let mut failures = Vec::new();
    for snapshot in mounts {
        if !scope.matches_mount(&snapshot.config.source) {
            continue;
        }
        match crate::daemon::stop_mount(snapshot.config.id.clone()) {
            Ok(_) => stopped += 1,
            Err(error) => failures.push(format!("{}: {error}", snapshot.config.label)),
        }
    }
    if failures.is_empty() {
        Ok(stopped)
    } else {
        Err(format!(
            "{stopped} getrennt, nicht getrennt: {}",
            failures.join(", ")
        ))
    }
}

fn orphaned_sync_jobs(scope: &RemovedEndpointScope) -> Vec<String> {
    let Ok(jobs) = crate::syncjobs::load() else {
        return Vec::new();
    };
    jobs.iter()
        .filter(|job| job_references_scope(job, scope))
        .map(|job| job.name.clone())
        .collect()
}

pub(crate) fn job_references_scope(job: &crate::syncjobs::SyncJob, scope: &RemovedEndpointScope) -> bool {
    scope.matches_key(&job.source) || scope.matches_key(&job.target)
}

impl super::App {
    /// GUI half of a removal: clean the persisted stores, reload the in-memory
    /// favourites/preferences, and close every tab that browses the endpoint.
    pub(in crate::app) fn cleanup_after_removal(
        &mut self,
        scope: &RemovedEndpointScope,
    ) -> CleanupReport {
        let mut report = cleanup_removed_endpoint_state(scope);
        self.favorites = std::fs::read_to_string(favorites_path())
            .ok()
            .map(|text| {
                text.lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string())
                    .collect()
            })
            .unwrap_or_default();
        self.dir_sort = load_dir_sort();
        let tab_matches = |remote: Option<&crate::connect::RemoteState>, root_path: &str| {
            remote
                .and_then(|remote| remote.endpoint_prefix.as_deref())
                .is_some_and(|prefix| scope.matches_key(prefix))
                || (root_path.starts_with("//") && scope.matches_key(root_path))
        };
        let inactive: Vec<usize> = (0..self.tabs.len())
            .filter(|&index| index != self.active_tab)
            .filter(|&index| {
                tab_matches(self.tabs[index].remote.as_ref(), &self.tabs[index].root_path)
            })
            .collect();
        for index in inactive.into_iter().rev() {
            self.close_tab(index);
            report.tabs_closed += 1;
        }
        if tab_matches(self.remote.as_ref(), &self.root_path) {
            self.clear_disconnected_source_view();
            report.tabs_closed += 1;
        }
        report
    }
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
