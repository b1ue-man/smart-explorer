//! The worker's side of `IpcRequest::Vault` (VLT-M4, C7.4): the vault
//! sessions of the desktop live in this process (`crate::vault::register`);
//! the GUI, `se` and `se mcp` unlock, lock, open and seal through here.
//! `ipc.rs` checked the daemon token before any request reaches this file
//! (arm in `ipc_host_ext.rs`). Replies and logs carry site IDs, labels and
//! states only: never a password, a path inside a vault or a cleartext name.
use std::io;
use std::net::TcpStream;

use crate::vault::{
    ConsumerKind, LockReason, LockRequest, OsSecretStore, RegistryStore, SealPurpose, SiteStatus,
    UnlockOptions, Unlocked,
};
use crate::vfs::Backend;

use super::ipc_host::ShareHost;
use super::ipc_protocol::{
    write_response, IpcResponse, SealPurposeWire, SecretText, VaultIpcReply, VaultIpcRequest,
    VaultSiteView, VaultStatusView,
};

/// Most items one `Seal`/`Unseal` takes, and the cleartext of all of them:
/// the reply must fit the IPC line (`MAX_IPC_LINE`, 2 MiB) with Base64url.
const MAX_SEAL_ITEMS: usize = 1024;
const MAX_SEAL_BYTES: usize = 512 * 1024;

/// Whether this process holds the vault keys of the device: the worker
/// while it serves (desktop), the app process (Android, worker in process).
pub(crate) fn hosts_vault_register() -> bool {
    crate::autostart::DAEMON_IN_PROCESS || super::live::serving_generation().is_some()
}

/// Serves one authenticated `IpcRequest::Vault`. `Open` keeps the stream.
pub(in crate::daemon) fn serve(
    _host: &ShareHost,
    mut stream: TcpStream,
    req: VaultIpcRequest,
) -> io::Result<()> {
    let response = match req {
        VaultIpcRequest::Open { site } => return serve_open(stream, &site),
        VaultIpcRequest::Status => status(),
        VaultIpcRequest::Unlock {
            site,
            password,
            remember,
        } => unlock(&site, &password, remember),
        VaultIpcRequest::UnlockSaved { site } => unlock_saved(&site),
        VaultIpcRequest::Lock { site, keep_unsaved } => lock(site.as_deref(), keep_unsaved),
        VaultIpcRequest::Seal {
            site,
            purpose,
            items,
        } => seal(&site, purpose, &items),
        VaultIpcRequest::Unseal {
            site,
            purpose,
            items,
        } => unseal(&site, purpose, &items),
    };
    write_response(&mut stream, &response)
}

fn failure(msg: impl Into<String>, code: Option<&str>) -> IpcResponse {
    IpcResponse::Err {
        msg: msg.into(),
        code: code.map(str::to_string),
    }
}

/// An `io::Error` of a vault operation; a locked vault keeps its code.
fn io_failure(error: io::Error) -> IpcResponse {
    let code = crate::vault::locked_site(&error).map(|_| crate::vault::VAULT_LOCKED_CODE);
    failure(error.to_string(), code)
}

fn resolve(location: &str) -> Result<(crate::vfs::BackendHandle, String), String> {
    crate::connect::resolve_endpoint(location)
}

fn site_view(status: SiteStatus) -> VaultSiteView {
    let record = status.record;
    VaultSiteView {
        label: record.display_label().to_string(),
        site: record.id,
        state: status.state.as_str().to_string(),
        epoch: status.epoch,
        consumers: u32::try_from(status.consumers).unwrap_or(u32::MAX),
        remember_password: record.remember_password,
        auto_lock_idle_min: record.lock.auto_lock_idle_min,
        lock_on_sleep: record.lock.lock_on_sleep,
        lock_on_screen_lock: record.lock.lock_on_screen_lock,
        lock_on_background_min: record.lock.lock_on_background_min,
        reason: None,
    }
}

/// Every entered site with its state; also the moment the register takes
/// changed lock settings („Register lädt neu“, T14).
fn status() -> IpcResponse {
    let load = match RegistryStore::open_default().load() {
        Ok(load) => load,
        Err(error) => return failure(format!("Tresorliste lesen: {error}"), None),
    };
    let register = crate::vault::register();
    register.reload_settings(&load.sites);
    let mut sites: Vec<VaultSiteView> = register
        .status(&load.sites)
        .into_iter()
        .map(site_view)
        .collect();
    for (file, reason) in load.broken {
        sites.push(VaultSiteView {
            site: file.trim_end_matches(".conf").to_string(),
            label: String::new(),
            state: crate::vault::SiteState::Unavailable.as_str().to_string(),
            epoch: None,
            consumers: 0,
            remember_password: false,
            auto_lock_idle_min: None,
            lock_on_sleep: false,
            lock_on_screen_lock: false,
            lock_on_background_min: None,
            reason: Some(reason),
        });
    }
    let (event_seq, events) = super::ipc_host::ui_events::vault_events();
    let signals = register.signal_support();
    IpcResponse::Vault(VaultIpcReply::Status(VaultStatusView {
        sites,
        event_seq,
        events,
        sleep_lock: signals.sleep,
        screen_lock: signals.screen_lock,
        screen_lock_note: crate::vault::SCREEN_LOCK_NOTE.map(str::to_string),
    }))
}

fn unlocked(unlocked: Unlocked) -> IpcResponse {
    IpcResponse::Vault(VaultIpcReply::Unlocked {
        site: unlocked.site,
        epoch: unlocked.epoch,
        already: unlocked.already,
        remember_error: unlocked.remember_error,
    })
}

fn unlock(site: &str, password: &SecretText, remember: Option<bool>) -> IpcResponse {
    let options = UnlockOptions {
        remember,
        available: crate::vault::available_memory(),
    };
    let store = RegistryStore::open_default();
    let register = crate::vault::register();
    match crate::vault::unlock_entered(
        register,
        &store,
        &OsSecretStore,
        site,
        password.expose(),
        options,
        &resolve,
    ) {
        Ok(result) => unlocked(result),
        Err(error) => failure(error.to_string(), error.code()),
    }
}

fn unlock_saved(site: &str) -> IpcResponse {
    let store = RegistryStore::open_default();
    let register = crate::vault::register();
    let available = crate::vault::available_memory();
    match crate::vault::unlock_saved(register, &store, &OsSecretStore, site, available, &resolve) {
        Ok(result) => unlocked(result),
        Err(error) => failure(error.to_string(), error.code()),
    }
}

fn lock(site: Option<&str>, keep_unsaved: bool) -> IpcResponse {
    let register = crate::vault::register();
    let locked = match site {
        None => register.lock_all_explicit(keep_unsaved),
        Some(query) => {
            let record = match RegistryStore::open_default().resolve(query) {
                Ok(record) => record,
                Err(message) => return failure(message, None),
            };
            let request = LockRequest::new(LockReason::Explicit, keep_unsaved);
            register.lock(&record.id, request).map(|locked| {
                if locked {
                    vec![record.id]
                } else {
                    Vec::new()
                }
            })
        }
    };
    match locked {
        Ok(sites) => IpcResponse::Vault(VaultIpcReply::Locked { sites }),
        Err(error) => failure(error.to_string(), Some(error.code())),
    }
}

/// The site ID of `query`, or the reply that says why there is none.
fn site_id(query: &str) -> Result<String, IpcResponse> {
    RegistryStore::open_default()
        .resolve(query)
        .map(|record| record.id)
        .map_err(|message| failure(message, None))
}

fn check_items(count: usize, bytes: usize) -> Result<(), IpcResponse> {
    if count > MAX_SEAL_ITEMS || bytes > MAX_SEAL_BYTES {
        return Err(failure(
            format!(
                "Höchstens {MAX_SEAL_ITEMS} Einträge mit zusammen {MAX_SEAL_BYTES} Byte je Anfrage"
            ),
            None,
        ));
    }
    Ok(())
}

fn seal(query: &str, purpose: SealPurposeWire, items: &[SecretText]) -> IpcResponse {
    let bytes = items.iter().map(|item| item.expose().len()).sum();
    if let Err(response) = check_items(items.len(), bytes) {
        return response;
    }
    let site = match site_id(query) {
        Ok(site) => site,
        Err(response) => return response,
    };
    let texts: Vec<&str> = items.iter().map(SecretText::expose).collect();
    match crate::vault::register().seal_texts(&site, SealPurpose::from(purpose), &texts) {
        Ok(items) => IpcResponse::Vault(VaultIpcReply::Sealed { items }),
        Err(error) => io_failure(error),
    }
}

fn unseal(query: &str, purpose: SealPurposeWire, items: &[String]) -> IpcResponse {
    let bytes = items.iter().map(String::len).sum();
    if let Err(response) = check_items(items.len(), bytes) {
        return response;
    }
    let site = match site_id(query) {
        Ok(site) => site,
        Err(response) => return response,
    };
    match crate::vault::register().unseal_texts(&site, SealPurpose::from(purpose), items) {
        Ok(texts) => IpcResponse::Vault(VaultIpcReply::Unsealed {
            items: texts
                .iter()
                .map(|text| SecretText::new(text.as_str()))
                .collect(),
        }),
        Err(error) => io_failure(error),
    }
}

/// `Open`: the worker's `VaultBackend` of the unlocked site over the agent
/// protocol, as `OpenShare` serves a Share peer. The connection is a
/// consumer of the session (C3.3); after a lock every request on it fails
/// with `VaultLocked`, which the client reads as `vault_locked`.
fn serve_open(mut stream: TcpStream, query: &str) -> io::Result<()> {
    let record = match RegistryStore::open_default().resolve(query) {
        Ok(record) => record,
        Err(message) => return write_response(&mut stream, &failure(message, None)),
    };
    let register = crate::vault::register();
    let locked = || {
        failure(
            crate::vault::VaultOpenError::Locked {
                site: record.id.clone(),
                label: record.display_label().to_string(),
            }
            .to_string(),
            Some(crate::vault::VAULT_LOCKED_CODE),
        )
    };
    let Some((session, epoch)) = register.session(&record.id) else {
        return write_response(&mut stream, &locked());
    };
    let Some(consumer) = register.consume(&record.id, ConsumerKind::Open) else {
        return write_response(&mut stream, &locked());
    };
    let opened = match crate::vault::open_with(&record, session, "/", &resolve) {
        Ok(opened) => opened,
        Err(error) => {
            let code = error.code();
            return write_response(&mut stream, &failure(error.to_string(), code));
        }
    };
    let reply = VaultIpcReply::Opened {
        site: record.id.clone(),
        label: record.display_label().to_string(),
        epoch,
        state_identity: opened.backend.state_identity(),
        namespace_identity: opened.backend.namespace_identity(),
    };
    write_response(&mut stream, &IpcResponse::Vault(reply))?;
    let read = stream.try_clone()?;
    let served = super::backend_server::serve_backend(read, stream, opened.backend);
    drop(consumer);
    served
}
