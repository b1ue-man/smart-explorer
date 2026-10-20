//! Stand-ins for the checks of the GNOME extension (U34-16, U34-17): a
//! `gsettings` program that keeps its keys in a folder (the schema
//! `org.gnome.shell` exists only where gnome-shell is installed, so the
//! suite's key-file backend would need it too), and a Shell on the bus that
//! keeps the extension list like `org.gnome.Shell.Extensions` does.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use super::gnome_dbus::ShellBus;
use super::gnome_settings::{
    format_string_array, parse_string_array, GnomeSettings, DISABLED, ENABLED,
};
use crate::desktop::core::ledger::GNOME_EXTENSION_UUID;

const SCHEMA: &str = "org.gnome.shell";
/// `ExtensionState.DISABLED` of the Shell.
const STATE_DISABLED: f64 = 2.0;

/// The keys of `org.gnome.shell`, one file each.
pub(super) struct Store {
    dir: PathBuf,
}

impl Store {
    pub(super) fn new(root: &Path) -> Store {
        let dir = root.join("gsettings-store");
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\nstore='{}'\n\
             case \"$1\" in\n\
             get) f=\"$store/$2.$3\"\n\
             if [ -f \"$f\" ]; then cat \"$f\"; else\n\
             case \"$3\" in disable-user-extensions) echo false;; *) echo '@as []';; esac\n\
             fi ;;\n\
             set) printf '%s\\n' \"$4\" > \"$store/$2.$3\" ;;\n\
             *) exit 2 ;;\n\
             esac\n",
            dir.display()
        );
        let program = dir.join("gsettings");
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        Store { dir }
    }

    pub(super) fn settings(&self) -> GnomeSettings {
        GnomeSettings::with_program(self.dir.join("gsettings"))
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{SCHEMA}.{key}"))
    }

    /// The list under `key` (`enabled-extensions`, `disabled-extensions`).
    pub(super) fn list(&self, key: &str) -> Vec<String> {
        std::fs::read_to_string(self.file(key))
            .ok()
            .and_then(|text| parse_string_array(&text))
            .unwrap_or_default()
    }

    pub(super) fn set_list(&self, key: &str, items: &[&str]) {
        let items: Vec<String> = items.iter().map(|item| item.to_string()).collect();
        std::fs::write(self.file(key), format_string_array(&items)).unwrap();
    }

    pub(super) fn set_bool(&self, key: &str, value: bool) {
        std::fs::write(self.file(key), format!("{value}\n")).unwrap();
    }
}

/// The Shell on the bus.
pub(super) struct FakeShell {
    store: Arc<Store>,
    /// The running Shell knows the extension of se (it was there at its
    /// start); a new one is unknown until the next login.
    knows_own: bool,
    extensions: Mutex<Vec<(String, f64)>>,
    calls: Mutex<Vec<String>>,
    version: String,
}

impl FakeShell {
    pub(super) fn new(store: &Arc<Store>, version: &str, knows_own: bool) -> Arc<FakeShell> {
        Arc::new(FakeShell {
            store: store.clone(),
            knows_own,
            extensions: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            version: version.into(),
        })
    }

    /// An extension the Shell has loaded, enabled.
    pub(super) fn load(&self, uuid: &str) {
        self.extensions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((uuid.to_string(), super::gnome_dbus::STATE_ENABLED));
    }

    /// `enable <uuid>` and `disable <uuid>`, in the order of the calls.
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The handle the code under test owns.
    pub(super) fn bus(self: &Arc<Self>) -> Box<dyn ShellBus> {
        Box::new(Shared(self.clone()))
    }

    /// The same, for code that shares the Shell with another owner.
    pub(super) fn shared(self: &Arc<Self>) -> Arc<dyn ShellBus> {
        Arc::new(Shared(self.clone()))
    }
}

struct Shared(Arc<FakeShell>);

impl ShellBus for Shared {
    fn shell_version(&self) -> Option<String> {
        Some(self.0.version.clone())
    }

    fn list_extensions(&self) -> Option<Vec<(String, f64)>> {
        Some(
            self.0
                .extensions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }

    fn set_extension(&self, uuid: &str, enable: bool) -> Result<bool, String> {
        let shell = &self.0;
        shell
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!(
                "{} {uuid}",
                if enable { "enable" } else { "disable" }
            ));
        let mut list = shell
            .extensions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let known = list.iter().any(|(known, _)| known == uuid)
            || (uuid == GNOME_EXTENSION_UUID && shell.knows_own);
        if !known {
            return Ok(false);
        }
        list.retain(|(known, _)| known != uuid);
        let mut enabled = shell.store.list(ENABLED);
        enabled.retain(|known| known != uuid);
        if enable {
            list.push((uuid.to_string(), super::gnome_dbus::STATE_ENABLED));
            enabled.push(uuid.to_string());
        } else {
            list.push((uuid.to_string(), STATE_DISABLED));
        }
        let items: Vec<&str> = enabled.iter().map(String::as_str).collect();
        shell.store.set_list(ENABLED, &items);
        let mut disabled = shell.store.list(DISABLED);
        disabled.retain(|known| known != uuid);
        let items: Vec<&str> = disabled.iter().map(String::as_str).collect();
        shell.store.set_list(DISABLED, &items);
        Ok(true)
    }
}
