//! Opening a session in its owner process: recognize the login shell and
//! start the session's shell on the target, run the engine, host it on this
//! process's attach service, list it when attachable, and connect the
//! owner's own client.
use super::client::SessionClient;
use super::engine::{Engine, Meta};
use super::registry::{self, Entry};
use super::server;
use super::transports::{self, TargetSpec, Transport};
use super::types::{validate_name, CommandRequest, Dialect, StdinMode};
use crate::creds::SavedConnection;
use crate::sftp::SftpBackend;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub(in crate::cli) struct OpenOptions {
    pub(in crate::cli) name: String,
    /// The owning process as others see it, e.g. `se mcp (pid 4711)`.
    pub(in crate::cli) owner: String,
    /// List the session for `se shell --attach` and the `sessions` tool.
    pub(in crate::cli) attachable: bool,
    /// Working directory and environment to start with.
    pub(in crate::cli) cwd: Option<String>,
    pub(in crate::cli) env: BTreeMap<String, String>,
}

/// Opens a session on `target`; `ssh` opens (or reuses) the SSH login of a
/// saved SFTP remote. Returns the owner's client.
pub(in crate::cli) fn open(
    target: &TargetSpec,
    ssh: &dyn Fn(&SavedConnection) -> Result<Arc<SftpBackend>, String>,
    mut options: OpenOptions,
) -> Result<SessionClient, String> {
    validate_name(&options.name)?;
    if options.cwd.is_none() {
        options.cwd = target.cwd().map(str::to_string);
    }
    let (dialect, transport) = transports::connect(target, ssh)?;
    host(
        transport,
        dialect,
        target.label(),
        options,
        &registry::directory(),
    )
}

/// Runs a session on an already started shell.
pub(super) fn host(
    transport: Box<dyn Transport>,
    dialect: Dialect,
    target: &str,
    options: OpenOptions,
    registry_dir: &Path,
) -> Result<SessionClient, String> {
    validate_name(&options.name)?;
    let id = session_id()?;
    let startup = CommandRequest {
        command: super::dialect::startup_command(dialect).to_string(),
        cwd: options.cwd.clone(),
        env: options.env.clone(),
        stdin: StdinMode::Null,
    };
    let engine = Engine::start(
        transport,
        Meta {
            id: id.clone(),
            name: options.name.clone(),
            target: target.to_string(),
            dialect,
            owner: options.owner.clone(),
            attachable: options.attachable,
        },
        &startup,
    )?;
    let owner = connect_owner(&engine, &options, target, registry_dir);
    if owner.is_err() {
        engine.cancel(&options.owner);
    }
    owner
}

fn connect_owner(
    engine: &Arc<Engine>,
    options: &OpenOptions,
    target: &str,
    registry_dir: &Path,
) -> Result<SessionClient, String> {
    let service = server::shared()?;
    let (owner_token, attach_token) = service.host(engine)?;
    if options.attachable {
        let registration = registry::register_in(
            registry_dir,
            &Entry {
                id: engine.meta.id.clone(),
                name: options.name.clone(),
                owner: options.owner.clone(),
                target: target.to_string(),
                dialect: engine.meta.dialect,
                port: service.port(),
                token: attach_token,
                created_ms: now_ms(),
            },
        )?;
        engine.on_end(Box::new(move || drop(registration)));
    }
    SessionClient::connect(
        service.port(),
        &engine.meta.id,
        &owner_token,
        &options.owner,
    )
}

fn session_id() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("cannot create a session id: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}
