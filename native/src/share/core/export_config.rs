//! What a host exports to its peers (Direct default and each room): shared
//! folders, each read-only or read-write, and saved connections exported one
//! by one (RV1, FC1). New exports start read-only; profiles written before
//! RV1 keep their meaning (read-write, all saved connections).
use serde::{Deserialize, Serialize};

/// What peers may do inside one export. A write also needs the peer's own
/// write right (V5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAccess {
    /// Additionally create, change, move, recycle and remove files.
    ReadWrite,
    /// Browse, read, analyse and watch. Values of later versions read as
    /// this one, never as more access.
    #[default]
    #[serde(other)]
    ReadOnly,
}

impl ExportAccess {
    pub fn allows_write(self) -> bool {
        self == Self::ReadWrite
    }
}

/// Profiles before RV1 knew no access: their exports were read-write and
/// keep that meaning.
fn legacy_read_write() -> ExportAccess {
    ExportAccess::ReadWrite
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One folder of the host exported under `label`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedRoot {
    pub label: String,
    pub path: String,
    /// Absent in profiles written before RV1: such exports stay read-write.
    #[serde(default = "legacy_read_write")]
    pub access: ExportAccess,
    /// Autostart, login and key locations below this export may be written
    /// (explicit, unsafe opt-in); without it every write there is refused.
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_system_writes: bool,
}

/// Scope name of the Direct default exports (rooms use their profile id).
pub const DIRECT_EXPORT_SCOPE: &str = "direct";

impl SharedRoot {
    /// A new export: read-only, system locations protected.
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            access: ExportAccess::default(),
            allow_system_writes: false,
        }
    }

    /// The same export with `access`.
    pub fn with_access(mut self, access: ExportAccess) -> Self {
        self.access = access;
        self
    }

    /// A new export of `scope`: Direct exports are read-write, since writing
    /// between one's own accepted devices is the purpose of Direct Share
    /// (user decision 2026-10-09, revising the FC1 default); room exports
    /// stay read-only. System locations stay protected in both.
    pub fn new_in_scope(label: impl Into<String>, path: impl Into<String>, scope: &str) -> Self {
        let access = if scope == DIRECT_EXPORT_SCOPE {
            ExportAccess::ReadWrite
        } else {
            ExportAccess::ReadOnly
        };
        Self::new(label, path).with_access(access)
    }
}

/// One saved connection exported under `/Verbindungen`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedConnection {
    /// `SavedConnection::account()` of the exported connection: stable when
    /// the connection is renamed, different when it points elsewhere.
    pub account: String,
    /// New entries are read-only.
    #[serde(default)]
    pub access: ExportAccess,
}

/// The exports of the Direct default or of one room.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareExportConfig {
    #[serde(default)]
    pub roots: Vec<SharedRoot>,
    /// Profiles before RV1: every saved connection, read-write. Still written
    /// for older app versions, which require the field, and turned into
    /// `shared_connections` by `migrate_legacy_connections`; new code never
    /// sets it.
    #[serde(default)]
    pub include_connections: bool,
    /// Saved connections exported one by one (RV1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_connections: Vec<SharedConnection>,
}

impl ShareExportConfig {
    /// Whether any saved connection is exported (the `/Verbindungen` folder).
    pub fn shares_connections(&self) -> bool {
        self.include_connections || !self.shared_connections.is_empty()
    }

    /// The access peers have to the saved connection `account`; `None` when
    /// it is not exported. An explicit entry wins over the legacy flag.
    pub fn connection_access(&self, account: &str) -> Option<ExportAccess> {
        self.shared_connections
            .iter()
            .find(|connection| connection.account == account)
            .map(|connection| connection.access)
            .or_else(|| self.include_connections.then_some(ExportAccess::ReadWrite))
    }

    /// One-time migration of the legacy flag: every saved connection
    /// (`saved_accounts`) without an entry becomes a read-write entry, then
    /// the flag is cleared. Connections saved later are no longer exported
    /// on their own. Returns whether the configuration changed.
    pub fn migrate_legacy_connections<I>(&mut self, saved_accounts: I) -> bool
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        if !self.include_connections {
            return false;
        }
        for account in saved_accounts {
            let account: String = account.into();
            if self
                .shared_connections
                .iter()
                .all(|connection| connection.account != account)
            {
                self.shared_connections.push(SharedConnection {
                    account,
                    access: ExportAccess::ReadWrite,
                });
            }
        }
        self.include_connections = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_rights_task_new_direct_exports_write_and_room_exports_read() {
        let direct = SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", DIRECT_EXPORT_SCOPE);
        assert_eq!(direct.access, ExportAccess::ReadWrite);
        assert!(!direct.allow_system_writes);
        let room = SharedRoot::new_in_scope("Fotos", "/home/u/Fotos", "room-profile-1");
        assert_eq!(room.access, ExportAccess::ReadOnly);
        assert!(!room.allow_system_writes);
    }

    #[test]
    fn review_task_new_exports_are_read_only_and_old_profiles_keep_writing() {
        let legacy: SharedRoot = serde_json::from_str(r#"{"label":"Home","path":"/home/u"}"#)
            .expect("profile before RV1");
        assert_eq!(legacy.access, ExportAccess::ReadWrite);
        assert!(!legacy.allow_system_writes);

        let new = SharedRoot::new("Fotos", "/home/u/Fotos");
        assert_eq!(new.access, ExportAccess::ReadOnly);
        assert!(!new.access.allows_write());
        let encoded = serde_json::to_string(&new).unwrap();
        assert!(encoded.contains(r#""access":"read_only""#), "{encoded}");
        assert!(!encoded.contains("allow_system_writes"), "{encoded}");
        let decoded: SharedRoot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, new);

        let writable = new.with_access(ExportAccess::ReadWrite);
        assert!(writable.access.allows_write());
        let future: SharedRoot =
            serde_json::from_str(r#"{"label":"A","path":"/a","access":"append_only"}"#)
                .expect("a later access value");
        assert_eq!(future.access, ExportAccess::ReadOnly);
    }

    #[test]
    fn review_task_legacy_connection_flag_migrates_to_explicit_entries() {
        let mut config: ShareExportConfig =
            serde_json::from_str(r#"{"roots":[],"include_connections":true}"#)
                .expect("profile before RV1");
        assert!(config.shares_connections());
        assert_eq!(
            config.connection_access("sftp://u@nas:22/"),
            Some(ExportAccess::ReadWrite)
        );

        config.shared_connections.push(SharedConnection {
            account: "ftp://u@host:21/".into(),
            access: ExportAccess::ReadOnly,
        });
        assert_eq!(
            config.connection_access("ftp://u@host:21/"),
            Some(ExportAccess::ReadOnly)
        );
        assert!(config.migrate_legacy_connections(["sftp://u@nas:22/", "ftp://u@host:21/"]));
        assert!(!config.include_connections);
        assert_eq!(
            config.shared_connections,
            vec![
                SharedConnection {
                    account: "ftp://u@host:21/".into(),
                    access: ExportAccess::ReadOnly,
                },
                SharedConnection {
                    account: "sftp://u@nas:22/".into(),
                    access: ExportAccess::ReadWrite,
                },
            ]
        );
        assert_eq!(config.connection_access("webdav://u@later:443/"), None);
        assert!(!config.migrate_legacy_connections(["webdav://u@later:443/"]));

        let empty = ShareExportConfig::default();
        assert!(!empty.shares_connections());
        assert_eq!(empty.connection_access("sftp://u@nas:22/"), None);
    }

    #[test]
    fn review_task_export_config_stays_readable_for_older_versions() {
        #[derive(Deserialize)]
        struct LegacyRoot {
            label: String,
            path: String,
        }
        #[derive(Deserialize)]
        struct LegacyConfig {
            roots: Vec<LegacyRoot>,
            include_connections: bool,
        }
        let config = ShareExportConfig {
            roots: vec![SharedRoot::new("Docs", "/d")],
            include_connections: false,
            shared_connections: vec![SharedConnection {
                account: "sftp://u@nas:22/".into(),
                access: ExportAccess::ReadOnly,
            }],
        };
        let encoded = serde_json::to_string(&config).unwrap();
        let legacy: LegacyConfig = serde_json::from_str(&encoded).expect("older version");
        assert_eq!(legacy.roots.len(), 1);
        assert_eq!(
            (
                legacy.roots[0].label.as_str(),
                legacy.roots[0].path.as_str()
            ),
            ("Docs", "/d")
        );
        assert!(!legacy.include_connections);
        let decoded: ShareExportConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, config);
    }
}
