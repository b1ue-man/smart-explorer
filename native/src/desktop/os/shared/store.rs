//! Files of se Desktop under `<AD>/desktop/` (C5.4): `state.json`,
//! `config.json`, the shell's runtime file `shell.json`, its log `shell.log`,
//! the restart request `shell.stop` and `state.lock`, which orders the
//! writers (`se desktop`, the settings and the shell when it halts). `absent`
//! is the missing folder, so reading never creates it; only `enable` does.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::state::ModuleState;

/// Bound of every JSON file of the folder.
const MAX_JSON: u64 = 256 * 1024;
/// `shell.log` moves to `shell.log.1` above this size.
const MAX_LOG: u64 = 256 * 1024;
/// How long a writer waits for `state.lock`.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// What the running shell reports about itself (`shell.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShellRuntime {
    pub(crate) shell_pid: u32,
    /// The shell's own program; status and stop check images against it.
    pub(crate) exe: String,
    pub(crate) surfaces: Vec<SurfaceRuntime>,
    /// Monitors the shell wants a surface on.
    pub(crate) monitors: usize,
    /// `live` or `profiles` (C4.4), as the shell's model feed sees it.
    pub(crate) share_state: String,
    pub(crate) share_state_reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SurfaceRuntime {
    pub(crate) monitor: String,
    pub(crate) pid: u32,
}

/// The folder `<AD>/desktop/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopDir {
    root: PathBuf,
}

/// Held while one writer changes `state.json` or `config.json`.
pub(crate) struct StateLock {
    _file: File,
}

impl DesktopDir {
    pub(crate) fn production() -> DesktopDir {
        DesktopDir::at(crate::support_dirs::app_data_dir().join("desktop"))
    }

    pub(crate) fn at(root: PathBuf) -> DesktopDir {
        DesktopDir { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn exists(&self) -> bool {
        self.root.is_dir()
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub(crate) fn shell_path(&self) -> PathBuf {
        self.root.join("shell.json")
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        self.root.join("shell.log")
    }

    pub(crate) fn stop_path(&self) -> PathBuf {
        self.root.join("shell.stop")
    }

    /// Linux single-instance lock of the shell (C5.5).
    pub(crate) fn instance_lock_path(&self) -> PathBuf {
        self.root.join("shell.lock")
    }

    pub(crate) fn layout_dir(&self) -> PathBuf {
        self.root.join("layout")
    }

    /// Creates the folder (the step `absent → enabled`).
    pub(crate) fn create(&self) -> Result<(), String> {
        crate::support_dirs::ensure_private_dir(&self.root)
            .map_err(|error| format!("{}: {error}", self.root.display()))
    }

    fn read_text(&self, path: &Path) -> Result<Option<String>, String> {
        if !self.exists() || !path.is_file() {
            return Ok(None);
        }
        match crate::support_dirs::read_private_text(path, MAX_JSON) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    fn write_text(&self, path: &Path, text: &str) -> Result<(), String> {
        crate::support_dirs::write_private_atomic(path, text.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// `None`: absent (no folder or no `state.json`).
    pub(crate) fn read_state(&self) -> Result<Option<ModuleState>, String> {
        match self.read_text(&self.state_path())? {
            Some(text) => ModuleState::from_json(&text)
                .map(Some)
                .map_err(|error| error.to_string()),
            None => Ok(None),
        }
    }

    pub(crate) fn write_state(&self, state: &ModuleState) -> Result<(), String> {
        let text = state.to_json().map_err(|error| error.to_string())?;
        self.write_text(&self.state_path(), &text)
    }

    pub(crate) fn read_config(&self) -> Result<Option<DesktopConfig>, String> {
        match self.read_text(&self.config_path())? {
            Some(text) => DesktopConfig::from_json(&text)
                .map(Some)
                .map_err(|error| error.to_string()),
            None => Ok(None),
        }
    }

    pub(crate) fn write_config(&self, config: &DesktopConfig) -> Result<(), String> {
        let text = config.to_json().map_err(|error| error.to_string())?;
        self.write_text(&self.config_path(), &text)
    }

    /// The shell's runtime file; `None` when missing or unreadable.
    pub(crate) fn read_shell(&self) -> Option<ShellRuntime> {
        let text = self.read_text(&self.shell_path()).ok()??;
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn write_shell(&self, runtime: &ShellRuntime) -> Result<(), String> {
        let text = serde_json::to_string_pretty(runtime).map_err(|error| error.to_string())?;
        self.write_text(&self.shell_path(), &text)
    }

    pub(crate) fn clear_shell(&self) {
        let _ = std::fs::remove_file(self.shell_path());
    }

    /// Takes `state.lock`; the folder must exist. Waits up to ten seconds.
    pub(crate) fn lock(&self) -> Result<StateLock, String> {
        let path = self.root.join("state.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(StateLock { _file: file }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err("se Desktop wird gerade von einem anderen Vorgang geändert".into())
                }
                Err(TryLockError::Error(error)) => {
                    return Err(format!("{}: {error}", path.display()))
                }
            }
        }
    }

    /// Asks a running shell to stop for a restart (`shell.stop`, new mtime).
    pub(crate) fn request_stop(&self) -> Result<(), String> {
        self.write_text(&self.stop_path(), "restart")
    }

    /// Modification time of `shell.stop`; the shell compares it with the
    /// time it saw at its start (same rule as the update stop, V8).
    pub(crate) fn stop_marker(&self) -> Option<SystemTime> {
        std::fs::metadata(self.stop_path())
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// Whether an arrangement file exists (`layout_saved` of the status).
    pub(crate) fn layout_saved(&self) -> bool {
        std::fs::read_dir(self.layout_dir()).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
        })
    }

    /// `se desktop remove`: the whole folder. Links inside are removed as
    /// links; nothing outside the folder is touched.
    pub(crate) fn delete_all(&self) -> Result<(), String> {
        match std::fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", self.root.display())),
            Ok(meta) if !meta.is_dir() => std::fs::remove_file(&self.root)
                .map_err(|error| format!("{}: {error}", self.root.display())),
            Ok(_) => std::fs::remove_dir_all(&self.root)
                .map_err(|error| format!("{}: {error}", self.root.display())),
        }
    }

    /// Appends one line `<unix s.ms>\t<event>\t<detail>` to `shell.log` (the
    /// shell's protocol, read by the suite). Never creates the folder.
    pub(crate) fn log(&self, event: &str, detail: &str) {
        if !self.exists() {
            return;
        }
        let path = self.log_path();
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG) {
            let _ = std::fs::rename(&path, self.root.join("shell.log.1"));
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let line = format!(
            "{}.{:03}\t{event}\t{}\n",
            now.as_secs(),
            now.subsec_millis(),
            detail.replace(['\n', '\r', '\t'], " ")
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
