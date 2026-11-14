use crate::mcp_launcher::core::lifecycle::UpdateState;
use crate::mcp_launcher::core::manifest::{
    Manifest, RuntimeContract, IPC_CONTRACT, STATE_CONTRACT,
};
use crate::mcp_launcher::os;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Installation {
    pub root: PathBuf,
    pub manifest: Manifest,
}
#[derive(Clone, Debug)]
pub struct RuntimeIdentity {
    pub installation: Installation,
    pub contract: RuntimeContract,
    pub executable: PathBuf,
}
pub struct RetirementRoot {
    pub root: PathBuf,
    pub snapshot_sha256: String,
}
impl Installation {
    pub fn load(root: &Path) -> Result<Self, String> {
        let root = std::fs::canonicalize(root).map_err(|e| format!("installation_root: {e}"))?;
        let path = root.join("mcp/current.v1.json");
        os::validate_tree(&root, &path)?;
        let manifest: Manifest = read_json(&path)?;
        manifest.validate(os::CLI_NAME)?;
        let identity_path = root.join("mcp/installation.id");
        os::validate_tree(&root, &identity_path)?;
        let id = std::fs::read_to_string(identity_path).map_err(|e| e.to_string())?;
        if id != manifest.installation_id {
            return Err("runtime_installation_id_mismatch".into());
        }
        Ok(Self { root, manifest })
    }
    /// Derive scope from the actual image, never PATH, cwd or an environment variable.
    pub fn current() -> Result<Option<Self>, String> {
        let image = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::for_image(&image)
    }
    pub fn for_image(image: &Path) -> Result<Option<Self>, String> {
        let image = std::fs::canonicalize(image).map_err(|e| e.to_string())?;
        let parent = image.parent().ok_or("runtime_image_parent_missing")?;
        if let Some(category) = parent.parent() {
            if matches!(
                category.file_name().and_then(|s| s.to_str()),
                Some("runtimes" | "launchers")
            ) && category
                .parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str())
                == Some("mcp")
            {
                let root = category
                    .parent()
                    .and_then(Path::parent)
                    .ok_or("installation_root_missing")?;
                return Self::load(root).map(Some);
            }
        }
        if parent.join("mcp/uninstall.v1.json").exists() {
            return Err("installation_uninstall_in_progress".into());
        }
        if parent.join("mcp/current.v1.json").exists() {
            return Self::load(parent).map(Some);
        }
        Ok(None)
    }
    /// The installation a daemon answers a runtime for. A daemon started from
    /// an installation answers only for that one. A daemon whose image lies
    /// outside every installation (an app copy beside no `mcp/` directory)
    /// answers for the root the authenticated runtime names, after the same
    /// tree and identity checks `load` applies; without a root it asks for one.
    pub fn for_daemon(
        image: Result<Option<Self>, String>,
        installation_id: &str,
        named_root: Option<&str>,
    ) -> Result<Self, String> {
        let installation = match (image?, named_root) {
            (Some(own), _) => own,
            (None, Some(root)) => {
                let root = Path::new(root);
                // A runtime names the canonical root of its own image.
                if !root.is_absolute() {
                    return Err("runtime_installation_root_invalid".into());
                }
                // As `for_image` refuses an installation that is being removed.
                if root.join("mcp/uninstall.v1.json").exists() {
                    return Err("installation_uninstall_in_progress".into());
                }
                Self::load(root)?
            }
            (None, None) => {
                return Err(crate::mcp_launcher::DAEMON_OUTSIDE_INSTALLATION.into());
            }
        };
        if installation.manifest.installation_id != installation_id {
            return Err("runtime_installation_id_mismatch".into());
        }
        Ok(installation)
    }
    pub fn launcher_path(&self) -> PathBuf {
        self.root
            .join("mcp/launchers")
            .join(&self.manifest.launcher_sha256)
            .join(os::LAUNCHER_NAME)
    }
    pub fn checked_launcher_path(&self) -> Result<PathBuf, String> {
        let path = self.launcher_path();
        let _verified = os::open_verified(&self.root, &path, &self.manifest.launcher_sha256)?;
        Ok(path)
    }
    /// A profile may keep its own explicitly admitted immutable starter.
    pub fn registered_launcher_allowed(&self, path: &Path) -> Result<bool, String> {
        let Some(hash) = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
        else {
            return Ok(false);
        };
        if !crate::mcp_launcher::core::manifest::is_sha256(hash)
            || path
                != self
                    .root
                    .join("mcp/launchers")
                    .join(hash)
                    .join(os::LAUNCHER_NAME)
            || !self
                .manifest
                .allowed_launcher_hashes
                .iter()
                .any(|allowed| allowed == hash)
        {
            return Ok(false);
        }
        let _image = os::open_verified(&self.root, path, hash)?;
        Ok(true)
    }
    pub fn runtime_path(&self) -> PathBuf {
        self.root.join(&self.manifest.runtime_file)
    }
    pub fn installed_cli(&self) -> PathBuf {
        self.root.join(os::CLI_NAME)
    }
    pub fn lock(&self, name: &str, exclusive: bool) -> Result<File, String> {
        let path = self.root.join("mcp").join(name);
        os::validate_tree(&self.root, &path)?;
        os::lock(&path, exclusive)
    }
    pub fn state(&self) -> Result<Option<UpdateState>, String> {
        let path = self.root.join("mcp/update.v1.json");
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(_) => {
                os::validate_tree(&self.root, &path)?;
                let state: UpdateState = read_json(&path)?;
                state.validate(os::CLI_NAME)?;
                if state.installation_id != self.manifest.installation_id {
                    return Err("update_installation_mismatch".into());
                }
                Ok(Some(state))
            }
        }
    }
    pub fn status(&self) -> serde_json::Value {
        let state = self
            .lock("control.lock", false)
            .and_then(|_lock| self.state());
        let active = crate::mcp_launcher::current_runtime();
        let servers = self.active_servers();
        let reconnect = active
            .as_ref()
            .is_some_and(|r| !self.manifest.permits(&r.contract))
            || servers.iter().any(|r| r["reconnect_required"] == true);
        let new_capabilities = active
            .as_ref()
            .is_some_and(|r| r.contract.runtime_sha256 != self.manifest.runtime_sha256)
            || servers
                .iter()
                .any(|r| r["catalog_refresh_requires_new_process"] == true);
        match state {
            Ok(update) => serde_json::json!({
                "state": update.as_ref().filter(|s| s.blocks_work() || (s.retire_managed && s.phase==crate::mcp_launcher::core::lifecycle::Phase::Committed && self.manifest==s.previous)).map(|s| {
                    if self.owner_alive(s) { "update_in_progress" } else { "update_recovery_required" }
                }).unwrap_or("ready"),
                "installation_id":self.manifest.installation_id,
                "installed_generation":self.manifest.generation,
                "compatibility_catalog_sha256":self.manifest.compatibility_catalog_sha256,
                "next_runtime_sha256":if update.as_ref().is_some_and(|s|s.blocks_work()&&s.retire_managed) {None}else{Some(update.as_ref().filter(|s|s.blocks_work()).map(|s|s.target.runtime_sha256.as_str()).unwrap_or(self.manifest.runtime_sha256.as_str()))},
                "next_state":if update.as_ref().is_some_and(|s|s.retire_managed) {"legacy_reconnect_required"}else{"managed"},
                "active_runtime_sha256":active.as_ref().map(|r| &r.contract.runtime_sha256),
                "active_servers":servers,
                "launcher":self.launcher_path(),
                "launcher_version":self.manifest.launcher_product_version,
                "started_launcher":crate::mcp_launcher::current_starter(),
                "legacy_entrypoint_requires_reconnect":crate::mcp_launcher::migration_entry(),
                "launcher_protocol":self.manifest.launcher_protocol,
                "reconnect_required":reconnect,
                "new_process_for_new_capabilities":new_capabilities,
                "update":update,
            }),
            Err(error) => {
                serde_json::json!({"state":if error.starts_with("lock_busy:") {"busy"}else{"invalid"},"error":error,"reconnect_required":!error.starts_with("lock_busy:")})
            }
        }
    }
    fn active_servers(&self) -> Vec<serde_json::Value> {
        let sessions = crate::support_dirs::app_data_dir().join("mcp/sessions");
        let Ok(entries) = std::fs::read_dir(sessions) else {
            return Vec::new();
        };
        let mut active = Vec::new();
        for entry in entries.flatten() {
            if !entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            {
                continue;
            }
            let directory = entry.path();
            let Ok(lock) = File::options()
                .read(true)
                .write(true)
                .open(directory.join("session.lock"))
            else {
                continue;
            };
            if !matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
                continue;
            }
            let Ok(mut record) = read_json::<serde_json::Value>(&directory.join("runtime.v1.json"))
            else {
                continue;
            };
            if record["installation_id"].as_str() != Some(self.manifest.installation_id.as_str()) {
                continue;
            }
            let Some(hash) = record["runtime_sha256"].as_str() else {
                continue;
            };
            if !crate::mcp_launcher::core::manifest::is_sha256(hash) {
                continue;
            }
            let expected = self.root.join("mcp/runtimes").join(hash).join(os::CLI_NAME);
            if record["executable"].as_str() != Some(expected.to_string_lossy().as_ref()) {
                continue;
            }
            let changed = hash != self.manifest.runtime_sha256;
            let contract = RuntimeContract {
                runtime_sha256: hash.into(),
                ipc_contract_id: record["ipc_contract_id"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                shared_state_contract_id: record["shared_state_contract_id"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            };
            let reconnect = !self.manifest.permits(&contract)
                || record["entrypoint"] == "installed_cli_migration";
            record["reconnect_required"] = serde_json::json!(reconnect);
            record["catalog_refresh_requires_new_process"] = serde_json::json!(changed);
            active.push(record);
        }
        active
    }
    pub fn owner_alive(&self, state: &UpdateState) -> bool {
        // Both evidence sources must agree; an occupied lock alone is insufficient.
        let held = self
            .lock("update-owner.lock", true)
            .is_err_and(|e| e.starts_with("lock_busy:"));
        held && os::process_matches(&state.owner)
    }
}
impl RuntimeIdentity {
    pub fn discover() -> Result<Option<Self>, String> {
        let executable = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some("runtimes")
        {
            return Ok(None);
        }
        let Some(installation) = Installation::for_image(&executable)? else {
            return Ok(None);
        };
        let Some(hash) = executable
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
        else {
            return Ok(None);
        };
        if executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some("runtimes")
        {
            return Ok(None);
        }
        if !crate::mcp_launcher::core::manifest::is_sha256(hash)
            || executable
                != installation
                    .root
                    .join("mcp/runtimes")
                    .join(hash)
                    .join(os::CLI_NAME)
        {
            return Err("runtime_image_hash_path_invalid".into());
        }
        let contract = RuntimeContract {
            runtime_sha256: hash.into(),
            ipc_contract_id: IPC_CONTRACT.into(),
            shared_state_contract_id: STATE_CONTRACT.into(),
        };
        let _image = os::open_verified(&installation.root, &executable, hash)?;
        Ok(Some(Self {
            installation,
            contract,
            executable,
        }))
    }
    pub fn lifetime(&self) -> Result<File, String> {
        self.installation.lock(
            &format!("runtimes/{}/lifetime.lock", self.contract.runtime_sha256),
            false,
        )
    }
}
/// An elevated helper never writes another account's agent profiles. An
/// elevated helper updating an installation its own account owns (root
/// updating root's installation on Linux) is that account and refreshes them:
/// deferring would leave its hosts on a starter the new generation no longer
/// admits, and no MCP start through that starter reaches the deferred refresh.
/// The refresh writes below the home directory of the environment, so that
/// directory (and each override of it the agent hosts read) must belong to
/// the same account: root started with another account's home stays deferred.
pub fn profile_refresh_deferred(root: &Path, elevated: bool) -> bool {
    elevated && !refresh_stays_in_own_account(root, &profile_homes())
}
/// The directories the profile refresh of this process writes below: the home
/// directory (`None`: the environment names none) and the overrides the
/// profile writers and the agent hosts' own programs read.
fn profile_homes() -> Vec<Option<PathBuf>> {
    let mut homes = vec![crate::support_dirs::home_dir()];
    for key in ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "XDG_DATA_HOME"] {
        if let Some(named) = std::env::var_os(key).filter(|value| !value.is_empty()) {
            homes.push(Some(PathBuf::from(named)));
        }
    }
    homes
}
/// Whether the installation and every directory of `homes` exist and belong
/// to the account this process runs as.
pub(in crate::mcp_launcher) fn refresh_stays_in_own_account(
    root: &Path,
    homes: &[Option<PathBuf>],
) -> bool {
    os::owned_by_caller(root)
        && homes
            .iter()
            .all(|home| home.as_deref().is_some_and(os::owned_by_caller))
}
pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn verify_handle(file: &mut File, expected: &str) -> Result<(), String> {
    if handle_sha256(file)? != expected {
        return Err("runtime_sha256_mismatch".into());
    }
    Ok(())
}
pub fn handle_sha256(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    let hash = format!("{:x}", digest.finalize());
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(hash)
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 256 * 1024 {
        return Err("runtime_record_too_large".into());
    }
    serde_json::from_reader(file).map_err(|e| format!("{}: {e}", path.display()))
}
pub fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    atomic_bytes(path, &serde_json::to_vec(value).map_err(|e| e.to_string())?)
}
pub fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("runtime_record_parent_missing")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    staged
        .write_all(bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    os::protect(staged.path(), false)?;
    os::publish_staged(staged, path, true)
}
