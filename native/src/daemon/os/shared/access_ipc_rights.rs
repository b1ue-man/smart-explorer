//! `IpcRequest::Access` for the rights of the compact connect and share
//! flow (plan 2026-10-31 §F, F1/F4/F5/F6): presets, open confirmations,
//! „Bestätigen“, „Abbrechen“ and the Direct code's rights. A change is one
//! profile transaction (`connection_rights_actions.rs`, with the same host
//! checks as every other surface), persisted before the worker reloads and
//! applies it. The client calls never start a worker.
use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_protocol::{AccessIpcReply, AccessIpcRequest};
use crate::share::connection_rights::{ConnectionRights, PresetView, RightsTarget};
use crate::share::connection_rights_actions as actions;
use crate::share::connection_rights_apply::PendingRightsView;
use crate::share::ShareProfiles;

fn home() -> Option<String> {
    Some(crate::daemon::ipc_host::default_home())
}

fn rights_of(profiles: &ShareProfiles, changed: bool) -> AccessIpcReply {
    AccessIpcReply::Rights {
        pending: profiles.pending_rights_views(),
        code_rights: profiles.code_rights.clone(),
        changed,
    }
}

impl ShareHost {
    /// The rights requests of `AccessIpcRequest`; any other is refused.
    pub(super) fn rights_reply(&self, request: AccessIpcRequest) -> Result<AccessIpcReply, String> {
        let change = match request {
            AccessIpcRequest::RightsPresets => {
                return Ok(AccessIpcReply::RightsPresets {
                    presets: crate::share::connection_rights::preset_views(),
                })
            }
            AccessIpcRequest::PendingRights => {
                let profiles = ShareProfiles::load_checked(home())?;
                return Ok(rights_of(&profiles, false));
            }
            AccessIpcRequest::ConfirmRights { target, rights } => {
                actions::confirm_connection_rights(home(), &target, &rights)?
            }
            AccessIpcRequest::CancelRights { target } => {
                actions::cancel_connection_rights(home(), &target)?
            }
            AccessIpcRequest::SetCodeRights { rights } => actions::set_code_rights(home(), rights)?,
            _ => return Err("not a rights request".into()),
        };
        // Persisted first; the reload hands the stored rights to the worker.
        if let Err(error) = self.reload_now() {
            return Err(format!(
                "Rechte gespeichert, Worker nicht neu geladen: {error}"
            ));
        }
        Ok(rights_of(&change.profiles, change.changed))
    }
}

#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
fn expect_rights(
    reply: AccessIpcReply,
) -> Result<(Vec<PendingRightsView>, Option<ConnectionRights>, bool), String> {
    match reply {
        AccessIpcReply::Rights {
            pending,
            code_rights,
            changed,
        } => Ok((pending, code_rights, changed)),
        _ => Err(super::unexpected()),
    }
}

/// The presets of the running worker.
#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
pub(crate) fn rights_presets() -> Result<Vec<PresetView>, String> {
    match super::request(AccessIpcRequest::RightsPresets)? {
        AccessIpcReply::RightsPresets { presets } => Ok(presets),
        _ => Err(super::unexpected()),
    }
}

/// The open confirmations and the Direct code's rights.
#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
pub(crate) fn pending_connection_rights(
) -> Result<(Vec<PendingRightsView>, Option<ConnectionRights>), String> {
    let (pending, code_rights, _) =
        expect_rights(super::request(AccessIpcRequest::PendingRights)?)?;
    Ok((pending, code_rights))
}

/// „Bestätigen“ over IPC; returns whether anything changed.
#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
pub(crate) fn confirm_connection_rights(
    target: RightsTarget,
    rights: ConnectionRights,
) -> Result<bool, String> {
    let reply = super::request(AccessIpcRequest::ConfirmRights { target, rights })?;
    expect_rights(reply).map(|(_, _, changed)| changed)
}

/// „Abbrechen“ over IPC; returns whether the connection was waiting.
#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
pub(crate) fn cancel_connection_rights(target: RightsTarget) -> Result<bool, String> {
    let reply = super::request(AccessIpcRequest::CancelRights { target })?;
    expect_rights(reply).map(|(_, _, changed)| changed)
}

/// The Direct code's rights over IPC (`None` clears them).
#[allow(dead_code)] // Client side for the GUI packages (plan 2026-10-31 G2/G3).
pub(crate) fn set_code_rights(rights: Option<ConnectionRights>) -> Result<bool, String> {
    let reply = super::request(AccessIpcRequest::SetCodeRights { rights })?;
    expect_rights(reply).map(|(_, _, changed)| changed)
}
