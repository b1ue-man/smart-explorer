//! Clearing the device's file digests in every process of the app that this
//! process reaches: itself, and the background worker of the app data folder
//! (the sync daemon, with the Share worker in it) through its loopback IPC
//! (`IpcRequest::HashCacheClear`). Android's worker runs in the app process,
//! so the clear of that process reaches it. The cache file and the memory of a
//! process are cleared by `agent_proto::hash_cache::clear`.
use std::io;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::ipc_storage::{read_ipc_addr, read_token};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(8);

/// What one process of the app did with its file digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClearStep {
    /// The process's memory and its cache file are cleared.
    Cleared,
    /// The worker runs in this process (Android): this process's clear covers it.
    SameProcess,
    /// No worker of this account runs (none answers on the IPC address of
    /// this account's app data folder). A worker under another account has
    /// its own app data folder and is neither reached nor named by this.
    NotRunning,
    /// The process could not be asked; the text is the reason.
    NotReached(String),
    /// The process answered with an error; the text is the reason.
    Failed(String),
}

/// The result of `clear_hash_cache`: this process (`app`) and the background
/// worker of this account's app data folder (`worker`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashCacheClearReport {
    pub app: ClearStep,
    pub worker: ClearStep,
}

impl ClearStep {
    /// The status text for users (German), with the reason where there is one.
    fn text(&self) -> String {
        match self {
            Self::Cleared => "geleert".to_string(),
            Self::SameProcess => "im selben Prozess geleert".to_string(),
            Self::NotRunning => "läuft nicht".to_string(),
            Self::NotReached(reason) => format!("nicht erreichbar: {reason}"),
            Self::Failed(reason) => format!("nicht geleert: {reason}"),
        }
    }

    /// True for the steps that leave no digest of this device in memory.
    fn cleared(&self) -> bool {
        matches!(self, Self::Cleared | Self::SameProcess | Self::NotRunning)
    }

    /// The state word and the reason of the JSON answer.
    fn state(&self) -> (&'static str, &str) {
        match self {
            Self::Cleared => ("cleared", ""),
            Self::SameProcess => ("same_process", ""),
            Self::NotRunning => ("not_running", ""),
            Self::NotReached(reason) => ("not_reached", reason.as_str()),
            Self::Failed(reason) => ("failed", reason.as_str()),
        }
    }
}

impl HashCacheClearReport {
    /// True when this process and the worker of this account were cleared (or
    /// no such worker runs). It says nothing about a worker of another
    /// account, which keeps its own digests in its own app data folder.
    pub fn complete(&self) -> bool {
        self.app.cleared() && self.worker.cleared()
    }

    /// One sentence for users (German): what was cleared and what was not,
    /// always for this account: the sentence names "dieses Kontos" for the
    /// cache and for the background service, so it claims nothing about a
    /// service under another account.
    pub fn summary(&self) -> String {
        let outcome = if self.complete() {
            "geleert"
        } else {
            "nicht vollständig geleert"
        };
        format!(
            "Prüfsummen-Cache dieses Kontos {outcome}: diese App {}; Hintergrunddienst dieses Kontos {}",
            self.app.text(),
            self.worker.text()
        )
    }

    /// The answer of the bridge and of `se doctor --json`: `cleared`, the
    /// `summary`, and the `state` and `detail` of each step.
    pub fn json(&self) -> Value {
        let (app_state, app_detail) = self.app.state();
        let (worker_state, worker_detail) = self.worker.state();
        json!({
            "cleared": self.complete(),
            "summary": self.summary(),
            "steps": {
                "app": { "state": app_state, "detail": app_detail },
                "worker": { "state": worker_state, "detail": worker_detail },
            },
        })
    }
}

/// Clears the file digests of this account: this process first, then the
/// background worker of the app data folder. A worker that runs under another
/// account has its own app data folder, whose IPC address this app cannot
/// read: this clear neither reaches nor claims it (`summary` says "dieses
/// Kontos").
pub fn clear_hash_cache() -> HashCacheClearReport {
    let app = match clear_folder(&crate::support_dirs::app_data_dir()) {
        Ok(()) => ClearStep::Cleared,
        Err(reason) => ClearStep::Failed(reason),
    };
    HashCacheClearReport {
        app,
        worker: clear_worker(),
    }
}

/// The worker's answer to `IpcRequest::HashCacheClear`: its memory and the
/// cache file of its app data folder are cleared, or the reason they are not.
pub(super) fn answer() -> IpcResponse {
    match clear_folder(&crate::support_dirs::app_data_dir()) {
        Ok(()) => IpcResponse::Ok,
        Err(msg) => IpcResponse::Err { msg, code: None },
    }
}

fn clear_folder(folder: &Path) -> Result<(), String> {
    crate::agent_proto::hash_cache::clear(folder)
        .map_err(|error| format!("Cache-Datei in {} nicht geleert: {error}", folder.display()))
}

fn clear_worker() -> ClearStep {
    if crate::autostart::DAEMON_IN_PROCESS {
        return ClearStep::SameProcess;
    }
    let Some(addr) = read_ipc_addr() else {
        return if crate::daemon::is_running() {
            ClearStep::NotReached("der Dienst meldet sich, seine IPC-Adresse fehlt".to_string())
        } else {
            ClearStep::NotRunning
        };
    };
    let token = match read_token() {
        Ok(token) => token,
        Err(error) => return ClearStep::NotReached(format!("Token: {error}")),
    };
    let mut stream = match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
        Ok(stream) => stream,
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            return ClearStep::NotRunning;
        }
        Err(error) => return ClearStep::NotReached(error.to_string()),
    };
    set_stream_timeout(&stream, Some(ANSWER_TIMEOUT));
    if let Err(error) = write_request(&mut stream, &IpcRequest::HashCacheClear { token }) {
        return ClearStep::NotReached(error.to_string());
    }
    match read_response(&mut stream) {
        Ok(IpcResponse::Ok) => ClearStep::Cleared,
        Ok(IpcResponse::Err { msg, .. }) => ClearStep::Failed(msg),
        Ok(_) => ClearStep::NotReached("unerwartete Antwort des Hintergrunddienstes".to_string()),
        Err(error) => ClearStep::NotReached(error.to_string()),
    }
}

#[cfg(test)]
#[path = "hash_cache_clear_tests.rs"]
mod tests;
