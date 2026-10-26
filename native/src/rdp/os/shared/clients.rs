use crate::rdp::core::{ClientCapability, HostService, StartRequest};
use crate::rdp::os::platform::clients as platform;
use std::{
    net::SocketAddr,
    process::{Child, Command},
};

pub(crate) struct ClientProcess {
    pub(crate) child: Child,
    // Platform-owned temporary profiles remain alive until the child exits.
    pub(crate) resources: Box<dyn Send>,
}

pub(crate) fn local_clients() -> Vec<ClientCapability> {
    platform::inventory()
}

pub(crate) fn check_client(request: &StartRequest) -> Result<ClientCapability, String> {
    let client = local_clients()
        .into_iter()
        .find(|client| client.id == request.client_id)
        .ok_or_else(|| {
            "The selected RDP client is not installed or has no supported verification adapter"
                .to_string()
        })?;
    if !client.available {
        return Err(client.detail.clone());
    }
    if client.certificate_policy != request.certificate_policy {
        return Err(
            "Selected RDP client does not implement the requested certificate policy".into(),
        );
    }
    if request.remote_app.is_some() && !client.remote_app {
        return Err("Selected native client has no implemented RemoteApp start adapter".into());
    }
    Ok(client)
}

pub(crate) fn launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
) -> Result<ClientProcess, String> {
    check_client(request)?;
    if !listener.ip().is_loopback() || listener.port() == 0 {
        return Err("RDP client requires a loopback tunnel".into());
    }
    platform::launch(request, service, listener)
}

pub(crate) fn hide_probe_console(command: &mut Command) {
    platform::hide_probe_console(command);
}
