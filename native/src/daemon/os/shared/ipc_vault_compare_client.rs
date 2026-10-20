//! The client side of the comparison requests of `IpcRequest::Vault`
//! (VLT-M8, U26-8) for processes without the vault keys (`se`, the desktop
//! GUI, `se mcp`). Every request carries the daemon token. A comparison and
//! a check start the worker when it does not run (the answer is then
//! `vault_locked`: nothing is unlocked). The worker does not time out; only
//! the client gives up after `COMPARE_TIMEOUT` / `RESOLVE_TIMEOUT` (a large
//! vault over a network takes long and reports no progress), says so, and the
//! work goes on in the worker. Failures keep the structured code
//! (`vault_locked`, `compare_stale`).
#![allow(dead_code)] // CLI (VLT-M9), GUI (VLT-M10), Android (VLT-M11)

use std::io;
use std::net::TcpStream;
use std::time::Duration;

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, CompareHead, CompareItemWire, IpcRequest,
    IpcResponse, ResolveFailure, SecretText, VaultIpcReply, VaultIpcRequest,
};
use super::ipc_storage::{read_ipc_addr, read_token};
use super::ipc_vault_client::VaultIpcError;

/// Longest wait for a comparison or a check (large vaults over a network;
/// the engine reports no progress).
const COMPARE_TIMEOUT: Duration = Duration::from_secs(600);
/// Longest wait for an action on an entry.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// What the client says when it stops waiting.
const STILL_WORKING: &str = "Der Dienst antwortet noch nicht; der Vorgang läuft weiter.";

/// A comparison with all its entries: the head and what `Items` delivered
/// page by page. The surfaces never see pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompareViewFull {
    pub(crate) head: CompareHead,
    pub(crate) items: Vec<CompareItemWire>,
}

/// What `vault_resolve` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolveResult {
    pub(crate) item: Option<String>,
    pub(crate) resolved: u32,
    pub(crate) failed: Vec<ResolveFailure>,
}

fn plain(message: impl Into<String>) -> VaultIpcError {
    VaultIpcError {
        message: message.into(),
        code: None,
    }
}

fn stale_error() -> VaultIpcError {
    VaultIpcError {
        message: crate::vault::error_text_for_code(crate::vault::COMPARE_STALE_CODE)
            .unwrap_or("Der Vergleich ist veraltet.")
            .to_string(),
        code: Some(crate::vault::COMPARE_STALE_CODE.to_string()),
    }
}

fn unexpected<T>() -> Result<T, VaultIpcError> {
    Err(plain("Unerwartete Antwort des Background-Workers"))
}

fn exchange(
    req: VaultIpcRequest,
    timeout: Duration,
    start_worker: bool,
) -> Result<VaultIpcReply, VaultIpcError> {
    if start_worker {
        super::ipc_client::ensure_worker_ready().map_err(plain)?;
    }
    // An action on a comparison needs the worker that holds it: without a
    // worker (restarted, ended) the comparison is gone.
    let gone = |error: VaultIpcError| {
        if start_worker {
            error
        } else {
            stale_error()
        }
    };
    let addr = read_ipc_addr().ok_or_else(|| gone(plain("Background-Worker IPC nicht bereit")))?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|error| gone(plain(format!("Background-Worker IPC: {error}"))))?;
    let token = read_token().map_err(|error| plain(format!("Background-Worker Token: {error}")))?;
    set_stream_timeout(&stream, Some(timeout));
    write_request(&mut stream, &IpcRequest::Vault { token, req })
        .map_err(|error| plain(format!("Tresor-IPC senden: {error}")))?;
    match read_response(&mut stream) {
        Ok(IpcResponse::Vault(reply)) => Ok(reply),
        Ok(IpcResponse::Err { msg, code }) => Err(VaultIpcError { message: msg, code }),
        Ok(_) => unexpected(),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Err(plain(STILL_WORKING))
        }
        Err(error) => Err(plain(format!("Tresor-IPC lesen: {error}"))),
    }
}

/// The entries of the comparison `list` of the number `revision`, page by
/// page until `total` are there.
fn fetch_items(
    list: &str,
    revision: u64,
    total: u64,
) -> Result<Vec<CompareItemWire>, VaultIpcError> {
    let mut items = Vec::new();
    let mut offset = 0u64;
    while offset < total {
        let req = VaultIpcRequest::Items {
            list: list.to_string(),
            revision,
            offset,
        };
        let VaultIpcReply::ItemPage {
            items: page,
            next_offset,
        } = exchange(req, COMPARE_TIMEOUT, false)?
        else {
            return unexpected();
        };
        if page.is_empty() {
            break;
        }
        items.extend(page);
        match next_offset {
            Some(next) if next > offset => offset = next,
            _ => break,
        }
    }
    Ok(items)
}

/// Compares the vault job `job` (ID, name or ID prefix) in the worker:
/// `groups` are group keys (empty: every group but `structure`). The
/// worker answers with the head and the pages of the entries; this returns
/// all of them as one value.
pub(crate) fn vault_compare(job: &str, groups: &[&str]) -> Result<CompareViewFull, VaultIpcError> {
    let req = VaultIpcRequest::Compare {
        job: job.to_string(),
        groups: groups.iter().map(|group| (*group).to_string()).collect(),
    };
    let VaultIpcReply::Compared(head) = exchange(req, COMPARE_TIMEOUT, true)? else {
        return unexpected();
    };
    let items = fetch_items(&head.job, head.revision, head.total)?;
    Ok(CompareViewFull { head, items })
}

/// The two versions of a text conflict (cleartext, for the merge view).
pub(crate) fn vault_compare_text(job: &str, item: &str) -> Result<(String, String), VaultIpcError> {
    let req = VaultIpcRequest::CompareText {
        job: job.to_string(),
        item: item.to_string(),
    };
    match exchange(req, RESOLVE_TIMEOUT, false)? {
        VaultIpcReply::Texts { a, b } => Ok((a.expose().to_string(), b.expose().to_string())),
        _ => unexpected(),
    }
}

fn resolved(reply: VaultIpcReply) -> Result<ResolveResult, VaultIpcError> {
    match reply {
        VaultIpcReply::Resolved {
            item,
            resolved,
            failed,
            ..
        } => Ok(ResolveResult {
            item,
            resolved,
            failed,
        }),
        _ => unexpected(),
    }
}

/// One action on one entry, or with `all` the collective action on every
/// conflict with two files; `confirmed` answers the question the action
/// asks (the worker refuses a removal that was not confirmed).
pub(crate) fn vault_resolve(
    job: &str,
    item: Option<&str>,
    action: &str,
    all: bool,
    confirmed: bool,
) -> Result<ResolveResult, VaultIpcError> {
    let req = VaultIpcRequest::Resolve {
        job: job.to_string(),
        item: item.map(str::to_string),
        action: action.to_string(),
        all,
        confirmed,
    };
    resolved(exchange(req, RESOLVE_TIMEOUT, false)?)
}

/// Resolves a text conflict with the merged text `content`.
pub(crate) fn vault_merge_apply(job: &str, item: &str, content: &str) -> Result<(), VaultIpcError> {
    let req = VaultIpcRequest::MergeApply {
        job: job.to_string(),
        item: item.to_string(),
        content: SecretText::new(content),
    };
    resolved(exchange(req, RESOLVE_TIMEOUT, false)?).map(|_| ())
}

/// „Tresor prüfen“ for the site `site` (ID, ID prefix or label); the
/// entries come page by page like those of a comparison.
pub(crate) fn vault_check(site: &str) -> Result<Vec<CompareItemWire>, VaultIpcError> {
    let req = VaultIpcRequest::Check {
        site: site.to_string(),
    };
    let VaultIpcReply::Checked {
        site,
        revision,
        total,
    } = exchange(req, COMPARE_TIMEOUT, true)?
    else {
        return unexpected();
    };
    fetch_items(&format!("check:{site}"), revision, total)
}
