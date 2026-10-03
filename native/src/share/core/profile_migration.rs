//! FC1 migration of implicit exports. Explicit access and stored locator
//! strings keep their meaning; the migration is committed before use.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::export_config::{ExportAccess, ShareExportConfig};
use super::profiles::ShareProfiles;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoHomeMigration {
    /// `direct` or the existing room profile id.
    pub scope: String,
    pub path: String,
}

impl ShareProfiles {
    pub(super) fn migrate_export_policy(
        &mut self,
        raw: &Value,
        home: Option<&str>,
        saved_accounts: &[String],
    ) -> Result<bool, String> {
        if home.is_none() && requires_home_fact(raw) {
            return Err("Home-Ort fuer die alte Home-Freigabenmigration fehlt; Konfiguration wurde nicht ersetzt".into());
        }
        let mut changed = migrate_config(&mut self.default_direct_exports,
            raw.get("default_direct_exports"), "direct", home, saved_accounts,
            &mut self.auto_home_migrations);
        for room in &mut self.rooms {
            let old = raw.get("rooms").and_then(Value::as_array).and_then(|rooms|
                rooms.iter().find(|value| value.get("id").and_then(Value::as_str) == Some(room.id.as_str())))
                .and_then(|value| value.get("exports"));
            changed |= migrate_config(&mut room.exports, old, &room.id, home, saved_accounts,
                &mut self.auto_home_migrations);
        }
        Ok(changed)
    }

    pub fn auto_home_was_migrated(&self, scope: &str, path: &str) -> bool {
        self.auto_home_migrations.iter().any(|migration| migration.scope == scope && migration.path == path)
    }
}

fn requires_home_fact(raw: &Value) -> bool {
    let pending = |config: &Value| config.get("roots").and_then(Value::as_array)
        .is_some_and(|roots| roots.iter().any(|root|
            root.get("label").and_then(Value::as_str) == Some("Home")
                && root.get("access").is_none()
                && !root.get("allow_system_writes").and_then(Value::as_bool).unwrap_or(false)));
    raw.get("default_direct_exports").is_some_and(pending)
        || raw.get("rooms").and_then(Value::as_array).is_some_and(|rooms|
            rooms.iter().filter_map(|room| room.get("exports")).any(pending))
}

fn migrate_config(config: &mut ShareExportConfig, raw: Option<&Value>, scope: &str,
    home: Option<&str>, accounts: &[String], notices: &mut Vec<AutoHomeMigration>) -> bool {
    let mut changed = config.migrate_legacy_connections(accounts.iter().cloned());
    let Some(home) = home else { return changed; };
    let Some(roots) = raw.and_then(|value| value.get("roots")).and_then(Value::as_array) else {
        return changed;
    };
    // The old app seeded exactly this root. An explicit access value, a
    // custom label/path, or unsafe opt-in is always the user's configuration.
    for (root, old) in config.roots.iter_mut().zip(roots) {
        if root.label != "Home" || root.path != home
            || root.allow_system_writes || old.get("access").is_some()
            || root.access != ExportAccess::ReadWrite {
            continue;
        }
        root.access = ExportAccess::ReadOnly;
        if !notices.iter().any(|notice| notice.scope == scope && notice.path == root.path) {
            notices.push(AutoHomeMigration { scope: scope.into(), path: root.path.clone() });
        }
        changed = true;
    }
    changed
}
