//! `se update`: feed check, installation layout, and the in-place
//! replacement of a terminal-only `se`.
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::cli_swap::{
    arm_interrupt_undo, install_cli_in_place, remove_orphans, ReplacedCli, UpdateLock,
};
use super::config::update_source_str;
use super::core::parse_ver;
use super::feed::classify_feed;
use super::feed_files::read_feed_version;
use super::os;
use super::staging::{remove_staged_path, stage_from_feed};
use super::types::StagedUpdate;

const NO_SOURCE: &str = "no update source is configured";

#[derive(Clone, Debug, Serialize)]
pub struct FeedCheck {
    pub source: String,
    pub current_version: String,
    pub feed_version: String,
    pub update_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Installation {
    /// Only `se`: it replaces its own installed file.
    TerminalOnly { cli: PathBuf },
    /// `se` beside the desktop app: the hash-bound helper replaces app,
    /// helper and `se` together, as the app's own update does.
    Desktop { cli: PathBuf, app: PathBuf },
}

impl Installation {
    pub fn cli(&self) -> &Path {
        match self {
            Self::TerminalOnly { cli } | Self::Desktop { cli, .. } => cli,
        }
    }
}

/// The configured feed's version compared with this build; `None` without
/// a configured update source.
pub fn check_feed() -> Result<Option<FeedCheck>, String> {
    let Some(source) = update_source_str() else {
        return Ok(None);
    };
    let feed_version = read_feed_version(&source)?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let update_available = parse_ver(&feed_version) > parse_ver(&current_version);
    Ok(Some(FeedCheck {
        source,
        current_version,
        feed_version,
        update_available,
    }))
}

pub fn detect_installation() -> Result<Installation, String> {
    let cli = os::running_cli_path()?;
    let dir = cli
        .parent()
        .ok_or_else(|| format!("{} has no installation folder", cli.display()))?
        .to_path_buf();
    Ok(installation_in(&dir, cli))
}

fn installation_in(dir: &Path, cli: PathBuf) -> Installation {
    let app = os::installed_app_names()
        .iter()
        .map(|name| dir.join(name))
        .find(|app| app.is_file());
    match app {
        Some(app) => Installation::Desktop { cli, app },
        None => Installation::TerminalOnly { cli },
    }
}

/// Keep a read-only image lease while generating legacy user profiles. The
/// installation namespace is never written by this guard.
pub(crate) struct LegacyIntegrationGuard { image:std::fs::File, cli:PathBuf }
pub(crate) fn lock_cli_for_integration(cli:&Path)->Result<LegacyIntegrationGuard,String> {
    super::cli_swap::check_cli_update_idle(cli)?;
    let image=crate::mcp_launcher::legacy_profile_image(cli)?;
    let guard=LegacyIntegrationGuard {image,cli:cli.into()};
    guard.verify_current()?;
    Ok(guard)
}
impl LegacyIntegrationGuard {
    pub(crate) fn verify_current(&self)->Result<(),String> {
        super::cli_swap::check_cli_update_idle(&self.cli)?;
        if !crate::mcp_launcher::same_image_file(&self.image,&self.cli)? {return Err("integration_generator_replaced: retry with the installed CLI".into());}
        Ok(())
    }
}

/// The desktop helper restarts Smart Explorer after replacing it; without a
/// graphical session that restart fails and the helper rolls everything back.
pub fn desktop_update_possible() -> Result<(), String> {
    os::desktop_session()
}

/// Downloads and verifies app, helper and `se` for the desktop helper, the
/// same staging the app's update check produces.
pub fn stage_update(version: &str) -> Result<StagedUpdate, String> {
    let source = update_source_str().ok_or(NO_SOURCE)?;
    stage_from_feed(&classify_feed(&source), version)
}

/// Replaces the installed terminal-only `se` with the feed's `se` payload.
pub fn replace_cli(cli: &Path, version: &str) -> Result<ReplacedCli, String> {
    os::cli_self_replacement()?;
    let lock = UpdateLock::acquire(cli)?;
    remove_orphans(cli);
    arm_interrupt_undo()?;
    let source = update_source_str().ok_or(NO_SOURCE)?;
    let feed = classify_feed(&source);
    let payload = feed.fetch_cli_exe(version)?;
    let launcher = feed.fetch_launcher_exe(version)?;
    let mcp = McpTerminalUpdate::prepare(cli, version, &payload, launcher.as_ref())?;
    let replaced = install_cli_in_place(payload.path(), payload.sha256(), cli, lock).map(|mut replaced| { replaced.mcp = Some(mcp); replaced });
    remove_staged_path(payload.path());
    replaced
}

pub(super) struct McpTerminalUpdate {
    root: PathBuf,
    initial: Option<crate::mcp_launcher::InitialGenerationLease>,
    transaction: Option<crate::mcp_launcher::UpdateTransaction>,
    previous_cli: String,
    cli: PathBuf,
    finished: bool,
}
impl McpTerminalUpdate {
    fn prepare(cli:&Path,version:&str,payload:&super::types::VerifiedPayload,launcher:Option<&super::types::VerifiedPayload>)->Result<Self,String> {
        let root=cli.parent().ok_or("MCP installation root missing")?.to_path_buf();
        let mut value=Self {root:root.clone(),initial:None,transaction:None,previous_cli:super::core::sha256_file(cli)?,cli:cli.into(),finished:false};
        if root.join("mcp/current.v1.json").exists() && crate::mcp_launcher::Installation::load(&root)?.status()["state"]=="update_recovery_required" {
            if crate::mcp_launcher::recover_update(&root,false).is_err() {
                crate::mcp_launcher::verify_recovery_targets(&root,true)?;
                if launcher.is_some() {crate::mcp_launcher::verify_installed_product(&root,cli,payload.sha256(),version)?;}
                else {crate::mcp_launcher::verify_legacy_product(&root,cli,payload.sha256(),version)?;}
                crate::mcp_launcher::recover_update(&root,true)?;
            }
        }
        let Some(launcher)=launcher else {
            if crate::mcp_launcher::staged_runtime_managed(payload.path(),payload.sha256(),version)? {
                return Err("Der Managed-faehige CLI-Payload benoetigt den MCP-Starter derselben Feedversion".into());
            }
            if root.join("mcp/current.v1.json").exists() {
                let proof=crate::mcp_launcher::core::lifecycle::InstallationTarget {file:cli.file_name().and_then(|s|s.to_str()).ok_or("installed CLI name invalid")?.into(),previous_sha256:Some(value.previous_cli.clone()),target_sha256:payload.sha256().into()};
                let mut transaction=crate::mcp_launcher::UpdateTransaction::prepare_retirement(&root,version,vec![proof])?;
                transaction.drain(std::time::Duration::from_secs(30))?;
                transaction.publish_stop(&crate::support_dirs::sync_data_dir().join("daemon.stop"))?;
                value.transaction=Some(transaction);
            }
            return Ok(value);
        };
        let old=if root.join("mcp/current.v1.json").exists() {Some(crate::mcp_launcher::Installation::load(&root)?)}else{None};
        let installation_id=match &old {Some(old)=>old.manifest.installation_id.clone(),None=>match std::fs::read_to_string(root.join("mcp/installation.id")) {Ok(id)=>id,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>crate::mcp_launcher::new_installation_id()?,Err(e)=>return Err(e.to_string())}};
        let input=crate::mcp_launcher::GenerationInput {installation_id,product_version:version.into(),runtime_path:payload.path().into(),runtime_sha256:payload.sha256().into(),launcher_path:launcher.path().into(),launcher_sha256:launcher.sha256().into()};
        let manifest=crate::mcp_launcher::stage_generation(&root,&input)?;
        if old.is_some() {
            let proof=crate::mcp_launcher::core::lifecycle::InstallationTarget {file:cli.file_name().and_then(|s|s.to_str()).ok_or("installed CLI name invalid")?.into(),previous_sha256:Some(value.previous_cli.clone()),target_sha256:payload.sha256().into()};
            let mut transaction=crate::mcp_launcher::UpdateTransaction::prepare_with_targets(&root,manifest.clone(),vec![proof])?;
            transaction.drain(std::time::Duration::from_secs(30))?;
            transaction.publish_stop(&crate::support_dirs::sync_data_dir().join("daemon.stop"))?;
            value.transaction=Some(transaction);
        }
        if old.is_none() {value.initial=Some(crate::mcp_launcher::InitialGenerationLease::prepare(&root,&manifest)?);}
        Ok(value)
    }
    pub(super) fn commit(&mut self)->Result<(),String> {
        if let Some(transaction)=self.transaction.as_mut() {transaction.commit()?;}
        else if let Some(initial)=&self.initial {initial.commit()?;}
        else {crate::mcp_launcher::disable_legacy_generation(&self.root)?;}
        self.finished=true;Ok(())
    }
    pub(super) fn rollback(&mut self)->Result<(),String> {
        if super::core::sha256_file(&self.cli)?!=self.previous_cli {return Err("update_recovery_required: previous CLI not restored".into());}
        if let Some(transaction)=self.transaction.as_mut() {transaction.rollback()?;}
        self.finished=true;Ok(())
    }
}
impl Drop for McpTerminalUpdate {
    fn drop(&mut self) { if !self.finished && super::core::sha256_file(&self.cli).is_ok_and(|h|h==self.previous_cli) {let _=self.rollback();} }
}

#[cfg(test)]
mod tests {
    use super::{installation_in, Installation};

    #[cfg(any(target_os="linux",windows))]
    struct RestorePermissions(Vec<(std::path::PathBuf,std::fs::Permissions)>);
    #[cfg(any(target_os="linux",windows))]
    impl Drop for RestorePermissions {
        fn drop(&mut self) {for (path,permissions) in &self.0 {let _=std::fs::set_permissions(path,permissions.clone());}}
    }
    #[cfg(any(target_os="linux",windows))]
    fn readonly(paths:&[std::path::PathBuf])->RestorePermissions {
        let restore=RestorePermissions(paths.iter().map(|path|(path.clone(),std::fs::metadata(path).unwrap().permissions())).collect());
        for path in paths.iter().rev() {
            let mut permissions=std::fs::metadata(path).unwrap().permissions();permissions.set_readonly(true);
            std::fs::set_permissions(path,permissions).unwrap();
        }
        restore
    }

    #[test]
    fn cli_task_installation_detects_terminal_only_and_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("se");
        std::fs::write(&cli, b"se").unwrap();
        assert_eq!(
            installation_in(dir.path(), cli.clone()),
            Installation::TerminalOnly { cli: cli.clone() }
        );
        let app = dir.path().join(super::os::installed_app_names()[0]);
        std::fs::write(&app, b"app").unwrap();
        let desktop = installation_in(dir.path(), cli.clone());
        assert_eq!(desktop.cli(), cli.as_path());
        assert_eq!(desktop, Installation::Desktop { cli, app });
    }

    #[cfg(target_os="linux")]
    #[test]
    fn batches_p13_legacy_profile_generation_serializes_with_terminal_updates() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");
        std::fs::write(&cli,b"local CLI").unwrap();
        let profile=super::lock_cli_for_integration(&cli).unwrap();
        let second=super::lock_cli_for_integration(&cli).unwrap();
        assert!(super::UpdateLock::acquire(&cli).is_err());
        assert!(!root.path().join("se.update-lock").exists());
        drop(profile);
        assert!(super::UpdateLock::acquire(&cli).is_err());
        drop(second);
        let update=super::UpdateLock::acquire(&cli).unwrap();
        assert!(super::lock_cli_for_integration(&cli).is_err());
        drop(update);
        drop(super::lock_cli_for_integration(&cli).unwrap());
        assert!(!root.path().join("se.update-lock").exists());
    }

    #[cfg(any(target_os="linux",windows))]
    #[test]
    fn batches_p13_legacy_profiles_need_no_installation_writes() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");
        std::fs::write(&cli,b"read-only installed CLI").unwrap();
        let paths=[root.path().to_path_buf(),cli.clone()];
        let _permissions=readonly(&paths);
        let first=super::lock_cli_for_integration(&cli).unwrap();
        let second=super::lock_cli_for_integration(&cli).unwrap();
        first.verify_current().unwrap();second.verify_current().unwrap();
        let names=std::fs::read_dir(root.path()).unwrap().map(|entry|entry.unwrap().file_name()).collect::<Vec<_>>();
        assert_eq!(names,[std::ffi::OsString::from("se")]);
        assert_eq!(std::fs::read(&cli).unwrap(),b"read-only installed CLI");
        assert!(std::fs::metadata(&cli).unwrap().permissions().readonly());
        assert!(std::fs::metadata(root.path()).unwrap().permissions().readonly());
        drop(first);drop(second);
        assert!(!root.path().join("se.update-lock").exists());
    }

    #[cfg(any(target_os="linux",windows))]
    #[test]
    fn batches_p13_legacy_profiles_read_historical_update_state_without_mutation() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");let state=root.path().join("se.update-lock");
        std::fs::write(&cli,b"installed CLI").unwrap();
        for (raw,prefix) in [(std::process::id().to_string(),"integration_update_in_progress"),("invalid".into(),"integration_update_state_unknown"),("0".into(),"integration_update_state_unknown")] {
            std::fs::write(&state,&raw).unwrap();
            assert!(super::lock_cli_for_integration(&cli).err().unwrap().starts_with(prefix));
            assert_eq!(std::fs::read_to_string(&state).unwrap(),raw);
        }
        std::fs::remove_file(&state).unwrap();
        let profile=super::lock_cli_for_integration(&cli).unwrap();
        let owner=std::process::id().to_string();std::fs::write(&state,&owner).unwrap();
        assert!(profile.verify_current().unwrap_err().starts_with("integration_update_in_progress"));
        drop(profile);
        assert_eq!(std::fs::read_to_string(&state).unwrap(),owner);
    }

    #[cfg(target_os="linux")]
    #[test]
    fn batches_p13_legacy_profiles_preserve_stale_update_lock_in_readonly_installation() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");let state=root.path().join("se.update-lock");
        std::fs::write(&cli,b"installed CLI").unwrap();
        assert!(!super::os::process_alive(u32::MAX));
        let owner=u32::MAX.to_string();std::fs::write(&state,&owner).unwrap();
        let permissions=readonly(&[root.path().to_path_buf(),cli.clone(),state.clone()]);
        drop(super::lock_cli_for_integration(&cli).unwrap());
        assert_eq!(std::fs::read_to_string(&state).unwrap(),owner);
        drop(permissions);
        let update=super::UpdateLock::acquire(&cli).unwrap();
        assert_eq!(std::fs::read_to_string(&state).unwrap(),std::process::id().to_string());
        drop(update);assert!(!state.exists());
    }

    #[cfg(target_os="linux")]
    #[test]
    fn batches_p13_legacy_profiles_reject_same_byte_image_replacement() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");let replacement=root.path().join("replacement");
        std::fs::write(&cli,b"same CLI bytes").unwrap();
        let profile=super::lock_cli_for_integration(&cli).unwrap();
        std::fs::write(&replacement,b"same CLI bytes").unwrap();
        // An uncooperative rename does not inherit the old file object's lease.
        std::fs::rename(&replacement,&cli).unwrap();
        assert!(profile.verify_current().unwrap_err().starts_with("integration_generator_replaced"));
        drop(profile);
        drop(super::lock_cli_for_integration(&cli).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn batches_p13_legacy_windows_profile_pins_cli_without_sidecar() {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se.exe");let moved=root.path().join("moved.exe");
        std::fs::write(&cli,b"installed CLI").unwrap();
        let profile=super::lock_cli_for_integration(&cli).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&cli).is_err());
        assert!(std::fs::rename(&cli,&moved).is_err());
        assert!(!root.path().join("se.exe.update-lock").exists());
        profile.verify_current().unwrap();drop(profile);
        std::fs::rename(&cli,&moved).unwrap();
        assert_eq!(std::fs::read(&moved).unwrap(),b"installed CLI");
    }
}
