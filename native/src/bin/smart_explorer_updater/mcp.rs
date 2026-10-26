//! Couples immutable generation publication to the existing apply transaction.
use super::args::ApplyArgs;
use smart_explorer::mcp_launcher::{GenerationInput, Installation, UpdateTransaction};
use std::path::PathBuf;

pub(super) struct McpUpdate {
    root: PathBuf,
    initial: Option<smart_explorer::mcp_launcher::InitialGenerationLease>,
    transaction: Option<UpdateTransaction>,
    previous_cli: Option<String>,
    previous_app: Option<String>,
    app: PathBuf,
    cli: PathBuf,
    finished: bool,
    already_completed: bool,
}
impl McpUpdate {
    pub(super) fn prepare(args: &ApplyArgs) -> Result<Self, String> {
        let root = args
            .cli_target
            .parent()
            .ok_or("MCP installation root missing")?
            .to_path_buf();
        let previous_cli = if args.cli_target.exists() {
            Some(super::hash::sha256_file(&args.cli_target)?)
        } else {
            None
        };
        let previous_app = if args.target.exists() {
            Some(super::hash::sha256_file(&args.target)?)
        } else {
            None
        };
        let mut value = Self {
            root: root.clone(),
            initial: None,
            transaction: None,
            previous_cli,
            previous_app,
            app: args.target.clone(),
            cli: args.cli_target.clone(),
            finished: false,
            already_completed: false,
        };
        if let Some(launcher) = args.launcher_staged.as_ref() {
            super::hash::verify_sha256(
                launcher,
                args.launcher_sha256
                    .as_ref()
                    .ok_or("MCP launcher hash missing")?,
            )?;
        }
        if root.join("mcp/current.v1.json").exists() {
            let installation = Installation::load(&root)?;
            if installation.status()["state"] == "update_recovery_required" {
                if smart_explorer::mcp_launcher::recover_update(&root, false).is_err() {
                    smart_explorer::mcp_launcher::verify_recovery_targets(&root, true)?;
                    if args.launcher_sha256.as_ref().is_some_and(|expected| {
                        installation.status()["update"]["target"]["launcher_sha256"].as_str()
                            != Some(expected.as_str())
                    }) {
                        return Err("recovery_candidate_launcher_mismatch".into());
                    }
                    super::hash::verify_sha256(&args.cli_target, &args.cli_sha256)?;
                    super::hash::verify_sha256(&args.helper_target, &args.helper_sha256)?;
                    if args.install_mode == super::args::InstallMode::GuiAck {
                        super::launch::spawn_verified_acknowledged(
                            &args.target,
                            &args.staged_sha256,
                            &["--updated"],
                        )
                        .map_err(|e| e.to_string())?;
                    } else {
                        super::installer::verify_completion(args)?;
                    }
                    smart_explorer::mcp_launcher::recover_update(&root, true)?;
                    value.finished = true;
                    value.already_completed = true;
                    return Ok(value);
                }
            }
        }
        let Some(launcher) = args.launcher_staged.as_ref() else {
            if root.join("mcp/current.v1.json").exists() {
                let mut targets = vec![
                    target_proof(&args.cli_target, &args.cli_sha256)?,
                    target_proof(&args.helper_target, &args.helper_sha256)?,
                ];
                if args.install_mode != super::args::InstallMode::CliOnly || args.target.exists() {
                    targets.push(target_proof(&args.target, &args.staged_sha256)?);
                }
                let mut transaction =
                    UpdateTransaction::prepare_retirement(&root, &args.version, targets)?;
                transaction.drain(std::time::Duration::from_secs(30))?;
                transaction
                    .publish_stop(&super::logging::appdata_dir().join("sync/daemon.stop"))?;
                value.transaction = Some(transaction);
            }
            return Ok(value);
        };
        let hash = args
            .launcher_sha256
            .as_ref()
            .ok_or("MCP launcher hash missing")?;
        super::hash::verify_sha256(launcher, hash)?;
        let old = if root.join("mcp/current.v1.json").exists() {
            Some(Installation::load(&root)?)
        } else {
            None
        };
        let id = match &old {
            Some(old) => old.manifest.installation_id.clone(),
            None => match std::fs::read_to_string(root.join("mcp/installation.id")) {
                Ok(id) => id,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    smart_explorer::mcp_launcher::new_installation_id()?
                }
                Err(error) => return Err(error.to_string()),
            },
        };
        let input = GenerationInput {
            installation_id: id,
            product_version: args.version.clone(),
            runtime_path: args.cli_staged.clone(),
            runtime_sha256: args.cli_sha256.clone(),
            launcher_path: launcher.clone(),
            launcher_sha256: hash.clone(),
        };
        let manifest = smart_explorer::mcp_launcher::stage_generation(&root, &input)?;
        if old.is_some() {
            let mut targets = vec![
                target_proof(&args.cli_target, &args.cli_sha256)?,
                target_proof(&args.helper_target, &args.helper_sha256)?,
            ];
            if args.install_mode != super::args::InstallMode::CliOnly || args.target.exists() {
                targets.push(target_proof(&args.target, &args.staged_sha256)?);
            }
            let mut transaction =
                UpdateTransaction::prepare_with_targets(&root, manifest.clone(), targets)?;
            transaction.drain(std::time::Duration::from_secs(30))?;
            let marker = super::logging::appdata_dir().join("sync/daemon.stop");
            transaction.publish_stop(&marker)?;
            value.transaction = Some(transaction);
        }
        if old.is_none() {
            value.initial = Some(
                smart_explorer::mcp_launcher::InitialGenerationLease::prepare(&root, &manifest)?,
            );
        }
        Ok(value)
    }
    pub(super) fn already_completed(&self) -> bool {
        self.already_completed
    }
    pub(super) fn commit(&mut self) -> Result<(), String> {
        if let Some(transaction) = self.transaction.as_mut() {
            transaction.commit()?;
        } else if let Some(initial) = &self.initial {
            initial.commit()?;
        } else {
            smart_explorer::mcp_launcher::disable_legacy_generation(&self.root)?;
        }
        self.finished = true;
        Ok(())
    }
}
impl Drop for McpUpdate {
    fn drop(&mut self) {
        if !self.finished
            && previous_matches(&self.cli, self.previous_cli.as_deref())
            && previous_matches(&self.app, self.previous_app.as_deref())
        {
            if let Some(transaction) = self.transaction.as_mut() {
                if let Err(error) = transaction.rollback() {
                    super::logging::append_log(&format!("MCP rollback: {error}"));
                }
            }
        }
    }
}

fn previous_matches(path: &std::path::Path, hash: Option<&str>) -> bool {
    match hash {
        Some(hash) => super::hash::verify_sha256(path, hash).is_ok(),
        None => !path.exists(),
    }
}

fn target_proof(
    path: &std::path::Path,
    hash: &str,
) -> Result<smart_explorer::mcp_launcher::core::lifecycle::InstallationTarget, String> {
    Ok(
        smart_explorer::mcp_launcher::core::lifecycle::InstallationTarget {
            file: path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("installation target name invalid")?
                .into(),
            previous_sha256: if path.exists() {
                Some(super::hash::sha256_file(path)?)
            } else {
                None
            },
            target_sha256: hash.into(),
        },
    )
}
