//! One dispatch for the daemon IPC variants that the batch „Rechte, se
//! Desktop, Tresore“ adds (plan 2026-10-17, B2, C2 Z2 and C3 W10): `Access`
//! and `Drive` (R2), `Vault` and `Screen` (R4), `MountHostExt` (R5). Each
//! strand adds its arm here, so neither `ipc.rs` nor `ipc_host.rs` grows
//! with the strands, and `exec_ipc.rs` stays unchanged.
//!
//! Wiring: `ipc.rs::handle_client` keeps answering every variant of
//! v0.5.182 and hands each later variant to [`ShareHost::serve_extension`]
//! after `ipc.rs::require_request_auth` accepted it. A later variant with a
//! daemon token is checked through `IpcRequest::daemon_token`; the fallback
//! arm of `require_request_auth` asks [`ShareHost::authorize_extension`] for
//! the others (`MountHostExt` checks the session token of its mount).
//! `ShareSnapshotPeek` (R2) is the one later variant that `ipc.rs` answers
//! itself: it shares the arm of `ShareSnapshot`, whose answer it repeats.
use std::io;
use std::net::TcpStream;

use super::super::ipc_protocol::{write_response, IpcRequest, IpcResponse};
use super::ShareHost;

/// Refusal of `ipc.rs::require_request_auth` for a request without any
/// capability; routing its fallback arm through here keeps the text.
const NO_CAPABILITY: &str = "daemon IPC request has no authentication capability";

impl ShareHost {
    /// Serves one authenticated request of a variant added after v0.5.182.
    /// The arm owns `stream`, so a streaming variant can keep it open.
    pub(in crate::daemon) fn serve_extension(
        &self,
        mut stream: TcpStream,
        req: IpcRequest,
    ) -> io::Result<()> {
        // One arm per later variant goes here: R2 `IpcRequest::Access` and
        // `IpcRequest::Drive`, R4 `IpcRequest::Vault` and `IpcRequest::Screen`,
        // R5 `IpcRequest::MountHostExt`; every other request stays unhandled.
        match req {
            IpcRequest::Access { request, .. } => self.serve_access(&mut stream, request),
            IpcRequest::Drive { req, .. } => {
                super::super::drive_modules::ipc_protocol_drives::serve(self, &mut stream, req)
            }
            IpcRequest::Vault { req, .. } => super::super::ipc_vault::serve(self, stream, req),
            other => unhandled(&mut stream, &other),
        }
    }

    /// Capability check of a later variant that carries no daemon token.
    // Staged: R5 routes the fallback arm of `ipc.rs::require_request_auth`
    // here (`MountHostExt`); until then nothing calls it.
    #[allow(dead_code)]
    pub(in crate::daemon) fn authorize_extension(&self, req: &IpcRequest) -> Result<(), String> {
        // R5: `IpcRequest::MountHostExt { id, session_token, .. }` checks
        // `self.mounts.check_session_token(id, session_token)`.
        let _ = req;
        Err(NO_CAPABILITY.into())
    }
}

/// Answer for a later variant that reaches the dispatch without an arm. The
/// error names only the variant tag, never a field (fields carry tokens).
fn unhandled(stream: &mut TcpStream, req: &IpcRequest) -> io::Result<()> {
    let value = serde_json::to_value(req).unwrap_or_default();
    // Indexing a missing key or a non-object yields `Null`, never a panic.
    let kind = value["t"].as_str().unwrap_or("unknown");
    let msg = format!("daemon IPC request `{kind}` has no handler in this daemon");
    write_response(stream, &IpcResponse::Err { msg, code: None })
}
