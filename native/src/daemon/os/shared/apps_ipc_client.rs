//! Desktop, CLI, MCP and Android use the same protected embedded-worker IPC.
use super::ipc_protocol::{
    apps::{AppsIpcReply, AppsIpcRequest},
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::{
    apps::{CatalogSnapshot, LaunchReceipt, LaunchRequest},
    share::{
        apps_wire::{AppsCommand, AppsResult},
        loopback_services::LoopbackService,
        PeerOpenTarget,
    },
};
use std::{
    net::TcpStream,
    time::{Duration, Instant},
};

pub(crate) fn apps_command(
    target: PeerOpenTarget,
    command: AppsCommand,
    wait: Option<Duration>,
    mut pending: impl FnMut(),
) -> Result<(String, String, AppsResult), String> {
    crate::daemon::ensure_worker_ready()?;
    let address = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background worker IPC is not ready".to_string())?;
    let token = super::ipc_storage::read_token().map_err(|e| e.to_string())?;
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let deadline = wait.and_then(|duration| Instant::now().checked_add(duration));
    if wait.is_some() && deadline.is_none() {
        return Err("Device-command wait is too large".into());
    }
    set_stream_timeout(&stream, wait);
    write_request(
        &mut stream,
        &IpcRequest::Apps {
            token,
            req: AppsIpcRequest::Command { target, command },
        },
    )
    .map_err(|e| e.to_string())?;
    loop {
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Device-command wait expired; query the same launch request ID instead of starting again".into());
            }
            set_stream_timeout(&stream, Some(remaining));
        }
        match read_response(&mut stream).map_err(|e| {
            format!(
                "Device-command daemon request: {e}; an admitted launch may already have started"
            )
        })? {
            IpcResponse::Apps(AppsIpcReply::Pending) => pending(),
            IpcResponse::Apps(AppsIpcReply::Completed {
                node_id,
                device_id,
                result,
            }) => return Ok((node_id, device_id, result)),
            IpcResponse::Err { msg, code } => {
                return Err(code.map_or(msg.clone(), |code| format!("{code}: {msg}")))
            }
            _ => return Err("Unexpected daemon device-command response".into()),
        }
    }
}
pub(crate) fn apps_catalog(
    target: PeerOpenTarget,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<CatalogSnapshot, String> {
    match apps_command(target, AppsCommand::Catalog, wait, pending)?.2 {
        AppsResult::Catalog { catalog } => Ok(catalog),
        _ => Err("Unexpected app catalog reply".into()),
    }
}
pub(crate) fn apps_icon(
    target: PeerOpenTarget,
    app_id: crate::apps::AppId,
    icon: crate::screen::core::app_icon::IconRef,
    catalog_revision: String,
    wait: Option<Duration>,
) -> Result<Option<crate::screen::core::app_icon::IconReply>, String> {
    icon.validate().map_err(|e| e.to_string())?;
    match apps_command(
        target,
        AppsCommand::Icon {
            app_id,
            icon: icon.clone(),
            catalog_revision,
        },
        wait,
        || {},
    )?
    .2
    {
        AppsResult::Icon { reply } => {
            if let Some(reply) = &reply {
                reply.bytes_for(&icon).map_err(|e| e.to_string())?;
            }
            Ok(reply)
        }
        _ => Err("Unexpected app icon reply".into()),
    }
}
pub(crate) fn apps_launch(
    target: PeerOpenTarget,
    request: LaunchRequest,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<LaunchReceipt, String> {
    match apps_command(
        target,
        AppsCommand::Launch {
            request: request.clone(),
        },
        wait,
        pending,
    )?
    .2
    {
        AppsResult::Launch { receipt } if receipt.request == request => Ok(receipt),
        _ => Err("Unexpected app launch reply".into()),
    }
}
pub(crate) fn apps_receipt(
    target: PeerOpenTarget,
    request: LaunchRequest,
    wait: Option<Duration>,
) -> Result<Option<LaunchReceipt>, String> {
    match apps_command(
        target,
        AppsCommand::Receipt {
            request: request.clone(),
        },
        wait,
        || {},
    )?
    .2
    {
        AppsResult::Receipt { receipt }
            if receipt.as_ref().is_none_or(|r| r.request == request) =>
        {
            Ok(receipt)
        }
        _ => Err("Unexpected app receipt reply".into()),
    }
}
/// Only authenticated daemon IPC may reconstruct this non-deserializable type.
/// There is no JSON/file/report import entrypoint in the CLI or mobile domain.
pub(crate) fn apps_rdp_probe(
    target: PeerOpenTarget,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<crate::rdp::AuthenticatedProbe, String> {
    let (node, _, result) = apps_command(target, AppsCommand::RdpProbe, wait, pending)?;
    match result {
        AppsResult::RdpProbe { host } => {
            crate::rdp::AuthenticatedProbe::from_authenticated_peer(node, host)
        }
        _ => Err("Unexpected RDP probe reply".into()),
    }
}
/// A failed 0.5.189 command: when the same host still answers the old
/// probe it is an older version, which is reported as such. Nothing of the
/// old answer is used (no silent return to the 0.5.188 flow).
fn newer_command_failed(target: PeerOpenTarget, wait: Option<Duration>, error: String) -> String {
    match apps_rdp_probe(target, wait, || {}) {
        Ok(_) => crate::rdp::OLD_HOST.into(),
        Err(_) => error,
    }
}

fn probe_v2(
    node: String,
    host: crate::rdp::HostProbe,
    setup: crate::rdp::core::SetupState,
) -> Result<(crate::rdp::AuthenticatedProbe, crate::rdp::core::SetupState), String> {
    if host.version != crate::rdp::core::RDP_PROBE_VERSION_V2 {
        return Err(probe_v2_mismatch(&host, &setup));
    }
    Ok((
        crate::rdp::AuthenticatedProbe::from_authenticated_peer(node, host)?,
        setup,
    ))
}

/// A v2 reply in the v1 form (hosts of this version before review B2 built
/// an unavailable probe that way): the reason the reply carries, never a
/// bare version error.
fn probe_v2_mismatch(host: &crate::rdp::HostProbe, setup: &crate::rdp::core::SetupState) -> String {
    use crate::rdp::core::SetupState;
    match (setup, &host.unavailable_reason) {
        (SetupState::Impossible(_) | SetupState::Failed(_) | SetupState::Declined, _) => {
            setup.text()
        }
        (_, Some(reason)) => format!("Remote Desktop nicht verfügbar: {reason}"),
        _ => "RDP-Probe v2 antwortete mit einer anderen Version".into(),
    }
}

/// Design 9.2 `RdpProbeV2`: the authenticated host inventory with SHA-1
/// thumbprints and the host's self-setup state. Under „Nachfragen“ it waits
/// for the host's answer; `wait` = `None` has no time limit.
pub(crate) fn apps_rdp_probe_v2(
    target: PeerOpenTarget,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<(crate::rdp::AuthenticatedProbe, crate::rdp::core::SetupState), String> {
    let reply = apps_command(target.clone(), AppsCommand::RdpProbeV2, wait, pending);
    match reply.map_err(|error| newer_command_failed(target, wait, error))? {
        (node, _, AppsResult::RdpProbeV2 { host, setup }) => probe_v2(node, host, setup),
        _ => Err("Unexpected RDP probe v2 reply".into()),
    }
}

/// Design 9.2 `RdpPrepare` (trigger T4): the host runs its RDP self-setup,
/// which may show the host's UAC/polkit dialog, then probes again.
pub(crate) fn apps_rdp_prepare(
    target: PeerOpenTarget,
    wait: Option<Duration>,
    pending: impl FnMut(),
) -> Result<(crate::rdp::AuthenticatedProbe, crate::rdp::core::SetupState), String> {
    let reply = apps_command(target.clone(), AppsCommand::RdpPrepare, wait, pending);
    match reply.map_err(|error| newer_command_failed(target, wait, error))? {
        (node, _, AppsResult::RdpPrepare { setup, host }) => probe_v2(node, host, setup),
        _ => Err("Unexpected RDP prepare reply".into()),
    }
}

/// Design 9.2 `RdpCredential`: se-managed service credentials (GNOME
/// desktop sharing), newly set by the host on every call.
pub(crate) fn apps_rdp_credential(
    target: PeerOpenTarget,
    service_id: &str,
    wait: Option<Duration>,
) -> Result<(String, crate::share::apps_wire::ManagedSecret), String> {
    let command = AppsCommand::RdpCredential {
        service_id: service_id.into(),
    };
    match apps_command(target, command, wait, || {})?.2 {
        AppsResult::RdpCredential { username, password } => Ok((username, password)),
        _ => Err("Unexpected RDP credential reply".into()),
    }
}

/// Design 9.2 `LocalServices`: the host's services shared with this device
/// (CLI/MCP without a running feed); `unavailable` names why none are known.
pub(crate) fn apps_local_services(
    target: PeerOpenTarget,
    wait: Option<Duration>,
) -> Result<(Vec<LoopbackService>, Option<String>), String> {
    match apps_command(target, AppsCommand::LocalServices, wait, || {})?.2 {
        AppsResult::LocalServices {
            services,
            unavailable,
        } => Ok((services, unavailable)),
        _ => Err("Unexpected local services reply".into()),
    }
}
fn local(request: AppsIpcRequest) -> Result<AppsIpcReply, String> {
    crate::daemon::ensure_worker_ready()?;
    let address =
        super::ipc_storage::read_ipc_addr().ok_or("Background worker IPC is not ready")?;
    let token = super::ipc_storage::read_token().map_err(|e| e.to_string())?;
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    set_stream_timeout(&stream, Some(Duration::from_secs(30)));
    write_request(
        &mut stream,
        &IpcRequest::Apps {
            token,
            req: request,
        },
    )
    .map_err(|e| e.to_string())?;
    match read_response(&mut stream).map_err(|e| e.to_string())? {
        IpcResponse::Apps(reply) => Ok(reply),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unexpected local app grant reply".into()),
    }
}
pub(crate) fn apps_local_grants(
    device: &str,
    key: &str,
) -> Result<crate::apps::LocalAppGrantSnapshot, String> {
    match local(AppsIpcRequest::LocalGrantSnapshot {
        device_id: device.into(),
        public_key: key.into(),
    })? {
        AppsIpcReply::LocalGrantSnapshot { snapshot }
            if snapshot.peer_device_id == device && snapshot.peer_public_key == key =>
        {
            Ok(snapshot)
        }
        _ => Err("Local app grant inventory identity mismatch".into()),
    }
}
pub(crate) fn apps_set_local_grant(
    change: crate::apps::LocalAppGrantChange,
) -> Result<crate::daemon::ipc_host::exec_grant_journal::ExecGrantPersistResult, String> {
    let target = change.target.clone();
    match local(AppsIpcRequest::SetLocalGrant { change })? {
        AppsIpcReply::LocalGrantMutation { result } if result.target == target => Ok(result),
        _ => Err("Local app grant mutation identity mismatch".into()),
    }
}

#[cfg(test)]
mod einrichtung_review_tests {
    use crate::rdp::core::SetupState;

    /// Review B2: a v2 reply in the v1 form names the host's reason (Windows
    /// Home, Android, a failed inventory), never only a version error.
    #[test]
    fn einrichtung_task_review_v1_form_reply_names_the_reason() {
        let host = crate::rdp::HostProbe::unavailable("windows", "Windows Home edition");
        let home = SetupState::Impossible("windows_home".into());
        let text = super::probe_v2_mismatch(&host, &home);
        assert!(text.contains("Windows Home"), "{text}");
        let android = SetupState::Impossible("android".into());
        assert!(super::probe_v2_mismatch(&host, &android).contains("Android"));
        let reason = super::probe_v2_mismatch(&host, &SetupState::NotNeeded);
        assert!(reason.contains("Windows Home edition"), "{reason}");
        let mut v2 = host;
        v2.version = crate::rdp::core::RDP_PROBE_VERSION_V2;
        v2.validate()
            .expect("an unavailable probe is a valid v2 answer");
    }
}
