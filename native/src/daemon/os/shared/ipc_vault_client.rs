//! The client side of `IpcRequest::Vault` for processes without the vault
//! keys (desktop GUI, `se`, `se mcp`; VLT-M4): every request carries the
//! daemon token. Unlocking starts the worker when it does not run; status,
//! lock, open and seal do not: without a worker no vault is unlocked, open
//! and seal then answer `vault_locked`. Failures keep the structured code of
//! the worker (`VaultIpcError::code`, C3.2).
#![allow(dead_code)] // CLI (VLT-M9), GUI (VLT-M10) and se Desktop (DSK-M4) call the rest

use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, SealPurposeWire,
    SecretText, VaultIpcReply, VaultIpcRequest, VaultStatusView,
};
use super::ipc_storage::{read_ipc_addr, read_token};

/// scrypt of a key file may take long (up to 1 GiB of work memory, C1.3).
const UNLOCK_TIMEOUT: Duration = Duration::from_secs(180);
/// A lock waits for its participants (at most 60 s for drives, C3.3).
const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
const SHORT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// A refused or failed vault request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultIpcError {
    pub(crate) message: String,
    /// `vault_locked`, `vault_wrong_password`, … when the worker named one.
    pub(crate) code: Option<String>,
}

impl VaultIpcError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }
}

impl fmt::Display for VaultIpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VaultIpcError {}

/// The result of an unlock through the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkerUnlocked {
    pub(crate) site: String,
    pub(crate) epoch: u64,
    pub(crate) already: bool,
    pub(crate) remember_error: Option<String>,
}

fn connect() -> Result<TcpStream, VaultIpcError> {
    let addr = read_ipc_addr()
        .ok_or_else(|| VaultIpcError::plain("Background-Worker IPC nicht bereit"))?;
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|error| VaultIpcError::plain(format!("Background-Worker IPC: {error}")))
}

/// Sends `req` on `stream` and reads the reply; the stream stays open for
/// `Open`.
fn exchange_on(
    mut stream: TcpStream,
    req: VaultIpcRequest,
    timeout: Duration,
) -> Result<(VaultIpcReply, TcpStream), VaultIpcError> {
    let token = read_token()
        .map_err(|error| VaultIpcError::plain(format!("Background-Worker Token: {error}")))?;
    set_stream_timeout(&stream, Some(timeout));
    write_request(&mut stream, &IpcRequest::Vault { token, req })
        .map_err(|error| VaultIpcError::plain(format!("Tresor-IPC senden: {error}")))?;
    match read_response(&mut stream)
        .map_err(|error| VaultIpcError::plain(format!("Tresor-IPC lesen: {error}")))?
    {
        IpcResponse::Vault(reply) => Ok((reply, stream)),
        IpcResponse::Err { msg, code } => Err(VaultIpcError { message: msg, code }),
        _ => unexpected(),
    }
}

/// Starts the worker when it does not run, then exchanges.
fn exchange_ready(
    req: VaultIpcRequest,
    timeout: Duration,
) -> Result<(VaultIpcReply, TcpStream), VaultIpcError> {
    super::ipc_client::ensure_worker_ready().map_err(VaultIpcError::plain)?;
    exchange_on(connect()?, req, timeout)
}

/// Exchanges only with a worker that runs: without one no vault is
/// unlocked, so the answer is `vault_locked` without starting a worker.
fn exchange_running(
    req: VaultIpcRequest,
    timeout: Duration,
) -> Result<(VaultIpcReply, TcpStream), VaultIpcError> {
    let stream = connect().map_err(|_| VaultIpcError {
        message: "Kein Tresor entsperrt: der Background-Worker läuft nicht".into(),
        code: Some(crate::vault::VAULT_LOCKED_CODE.to_string()),
    })?;
    exchange_on(stream, req, timeout)
}

fn unexpected<T>() -> Result<T, VaultIpcError> {
    Err(VaultIpcError::plain(
        "Unerwartete Antwort des Background-Workers",
    ))
}

/// `Vault::Status`; `None` when no worker runs (then every site is locked).
pub(crate) fn vault_status() -> Result<Option<VaultStatusView>, VaultIpcError> {
    let Ok(stream) = connect() else {
        return Ok(None);
    };
    match exchange_on(stream, VaultIpcRequest::Status, SHORT_TIMEOUT)?.0 {
        VaultIpcReply::Status(status) => Ok(Some(status)),
        _ => unexpected(),
    }
}

fn unlocked(reply: VaultIpcReply) -> Result<WorkerUnlocked, VaultIpcError> {
    match reply {
        VaultIpcReply::Unlocked {
            site,
            epoch,
            already,
            remember_error,
        } => Ok(WorkerUnlocked {
            site,
            epoch,
            already,
            remember_error,
        }),
        _ => unexpected(),
    }
}

/// Unlocks the site `site` (ID, ID prefix or label) in the worker.
pub(crate) fn unlock_vault(
    site: &str,
    password: &str,
    remember: Option<bool>,
) -> Result<WorkerUnlocked, VaultIpcError> {
    let req = VaultIpcRequest::Unlock {
        site: site.to_string(),
        password: SecretText::new(password),
        remember,
    };
    unlocked(exchange_ready(req, UNLOCK_TIMEOUT)?.0)
}

/// Unlocks with the remembered password (`--saved-password`).
pub(crate) fn unlock_vault_saved(site: &str) -> Result<WorkerUnlocked, VaultIpcError> {
    let req = VaultIpcRequest::UnlockSaved {
        site: site.to_string(),
    };
    unlocked(exchange_ready(req, UNLOCK_TIMEOUT)?.0)
}

/// Locks `site` or, with `None`, every site; returns the sites locked.
/// Without a running worker nothing is unlocked: an empty list.
pub(crate) fn lock_vaults(
    site: Option<&str>,
    keep_unsaved: bool,
) -> Result<Vec<String>, VaultIpcError> {
    let Ok(stream) = connect() else {
        return Ok(Vec::new());
    };
    let req = VaultIpcRequest::Lock {
        site: site.map(str::to_string),
        keep_unsaved,
    };
    match exchange_on(stream, req, LOCK_TIMEOUT)?.0 {
        VaultIpcReply::Locked { sites } => Ok(sites),
        _ => unexpected(),
    }
}

/// Seals `items` for `site` in the worker (protocol fields, local data).
pub(crate) fn seal_vault_items(
    site: &str,
    purpose: SealPurposeWire,
    items: &[&str],
) -> Result<Vec<String>, VaultIpcError> {
    let req = VaultIpcRequest::Seal {
        site: site.to_string(),
        purpose,
        items: items.iter().map(|item| SecretText::new(*item)).collect(),
    };
    match exchange_running(req, SHORT_TIMEOUT)?.0 {
        VaultIpcReply::Sealed { items } => Ok(items),
        _ => unexpected(),
    }
}

/// Unseals items of `seal_vault_items`.
pub(crate) fn unseal_vault_items(
    site: &str,
    purpose: SealPurposeWire,
    items: &[String],
) -> Result<Vec<SecretText>, VaultIpcError> {
    let req = VaultIpcRequest::Unseal {
        site: site.to_string(),
        purpose,
        items: items.to_vec(),
    };
    match exchange_running(req, SHORT_TIMEOUT)?.0 {
        VaultIpcReply::Unsealed { items } => Ok(items),
        _ => unexpected(),
    }
}

/// `vault://<site-id><path>` through the worker: its `VaultBackend` over
/// the agent protocol. A lock later makes every call fail with
/// `VaultLocked` (`SE_VAULT_V1`); a pool drops the backend then.
pub(crate) fn open_vault_backend(
    site: &str,
    path: &str,
) -> Result<(crate::vfs::BackendHandle, String), VaultIpcError> {
    let req = VaultIpcRequest::Open {
        site: site.to_string(),
    };
    let (reply, stream) = exchange_running(req, SHORT_TIMEOUT)?;
    let VaultIpcReply::Opened {
        site,
        state_identity,
        namespace_identity,
        ..
    } = reply
    else {
        return unexpected();
    };
    set_stream_timeout(&stream, None);
    let read = stream
        .try_clone()
        .map_err(|error| VaultIpcError::plain(format!("Tresor-IPC: {error}")))?;
    let identity = Arc::new(WorkerVault {
        site,
        state_identity,
        namespace_identity,
    });
    let agent =
        crate::agent::AgentBackend::from_streams(Box::new(read), Box::new(stream), identity)
            .map_err(|error| VaultIpcError::plain(format!("Tresor-Verbindung: {error}")))?;
    Ok((Arc::new(agent), path.to_string()))
}

/// Scheme and identity of a vault the worker serves; the agent protocol
/// carries every operation.
struct WorkerVault {
    site: String,
    state_identity: String,
    namespace_identity: String,
}

fn closed<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "Tresor-Verbindung zum Background-Worker geschlossen",
    ))
}

impl crate::vfs::Backend for WorkerVault {
    fn scheme(&self) -> crate::vfs::Scheme {
        crate::vfs::Scheme::Vault
    }

    fn root_display(&self) -> String {
        "/".to_string()
    }

    fn state_identity(&self) -> String {
        self.state_identity.clone()
    }

    fn namespace_identity(&self) -> String {
        self.namespace_identity.clone()
    }

    fn list_dir(&self, _path: &str) -> crate::vfs::VfsResult<Vec<crate::vfs::VfsMeta>> {
        closed()
    }

    fn stat(&self, _path: &str) -> crate::vfs::VfsResult<crate::vfs::VfsMeta> {
        closed()
    }

    fn open_read(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Read + Send>> {
        closed()
    }

    fn open_write(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Write + Send>> {
        closed()
    }

    fn rename(&self, _src: &str, _dst: &str) -> crate::vfs::VfsResult<()> {
        closed()
    }

    fn remove_file(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        closed()
    }

    fn remove_dir(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        closed()
    }

    fn mkdir_all(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        closed()
    }
}

impl fmt::Debug for WorkerVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerVault")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}
