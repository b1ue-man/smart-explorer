//! Opening a session in its owner process: recognize the login shell and
//! start the session's shell on the target, run the engine, host it on this
//! process's attach service, list it when attachable, and connect the
//! owner's own client. A command session (`open`) recognizes the login shell
//! and runs commands with markers; a terminal session (`open_terminal`) runs
//! the login shell in a pseudo-terminal.
use super::client::SessionClient;
use super::engine::{Engine, Meta};
use super::registry::{self, Entry};
use super::server;
use super::terminal_client::TerminalClient;
use super::terminal_engine::{TerminalEngine, TerminalMeta};
use super::terminal_transports::{connect_terminal, TerminalConnect};
use super::transports::{self, TargetSpec, Transport};
use super::types::{
    validate_name, CommandRequest, Dialect, SessionKind, StdinMode, TerminalOptions,
};
use crate::creds::SavedConnection;
use crate::sftp::SftpBackend;
use crate::share::ExecTerminalRequest;
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
                kind: SessionKind::Commands,
                dialect: Some(engine.meta.dialect),
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

/// What opening a terminal session produced.
pub(in crate::cli) enum TerminalOpen {
    Ready(TerminalClient),
    /// The target cannot give a terminal (an SSH server that refuses a PTY, a
    /// Share peer without terminal support); why. A command session can serve
    /// instead.
    Unsupported(String),
}

/// Opens a terminal session on `target`: its login shell in a PTY of
/// `options.size`, like `ssh HOST`. Returns the owner's client.
pub(in crate::cli) fn open_terminal(
    target: &TargetSpec,
    ssh: &dyn Fn(&SavedConnection) -> Result<Arc<SftpBackend>, String>,
    mut options: TerminalOptions,
) -> Result<TerminalOpen, String> {
    check_terminal_options(&options)?;
    if options.cwd.is_none() {
        options.cwd = target.cwd().map(str::to_string);
    }
    match connect_terminal(target, ssh, &options)? {
        TerminalConnect::Unsupported(why) => Ok(TerminalOpen::Unsupported(why)),
        TerminalConnect::Ready(transport, dialect) => host_terminal(
            transport,
            dialect,
            target.label(),
            options,
            &registry::directory(),
        )
        .map(TerminalOpen::Ready),
    }
}

/// The name, the size and the `TERM` value, before anything starts remotely.
fn check_terminal_options(options: &TerminalOptions) -> Result<(), String> {
    validate_name(&options.name)?;
    options.size.validate()?;
    ExecTerminalRequest {
        term: options.term.clone(),
        size: options.size.into(),
    }
    .validate()
    .map_err(|error| error.to_string())
}

/// Runs a terminal session on an already started shell.
pub(super) fn host_terminal(
    transport: Box<dyn Transport>,
    dialect: Option<Dialect>,
    target: &str,
    options: TerminalOptions,
    registry_dir: &Path,
) -> Result<TerminalClient, String> {
    check_terminal_options(&options)?;
    let id = session_id().inspect_err(|_| transport.input().cancel())?;
    let engine = TerminalEngine::start(
        transport,
        TerminalMeta {
            id,
            name: options.name.clone(),
            target: target.to_string(),
            owner: options.owner.clone(),
            attachable: options.attachable,
            dialect,
        },
        options.size,
    )?;
    let owner = connect_terminal_owner(&engine, &options, registry_dir);
    if owner.is_err() {
        engine.cancel(&options.owner);
    }
    owner
}

/// Hosts the session, joins the owner's client, and only then lets the shell's
/// first output flow and lists the session: the owner sees the banner and an
/// attached client never takes the owner's place.
fn connect_terminal_owner(
    engine: &Arc<TerminalEngine>,
    options: &TerminalOptions,
    registry_dir: &Path,
) -> Result<TerminalClient, String> {
    let service = server::shared()?;
    let (owner_token, attach_token) = service.host_terminal(engine)?;
    let client = TerminalClient::connect(
        service.port(),
        &engine.meta.id,
        &owner_token,
        &options.owner,
        options.size,
    )?;
    engine.begin()?;
    if options.attachable {
        let registration = registry::register_in(
            registry_dir,
            &Entry {
                id: engine.meta.id.clone(),
                name: engine.meta.name.clone(),
                owner: engine.meta.owner.clone(),
                target: engine.meta.target.clone(),
                kind: SessionKind::Terminal,
                dialect: engine.meta.dialect,
                port: service.port(),
                token: attach_token,
                created_ms: now_ms(),
            },
        )?;
        engine.on_end(Box::new(move || drop(registration)));
    }
    Ok(client)
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
