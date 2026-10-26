use crate::rdp::core::{
    CertificatePolicy, ClientCapability, ClientKind, HostService, StartRequest,
};
use crate::rdp::os::shared::clients::ClientProcess;
use crate::rdp::os::shared::process::{capture, find_tool};
use std::io::IsTerminal;
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
};
fn executable(id: &str) -> Option<PathBuf> {
    match id {
        "xfreerdp3" => find_tool(&["xfreerdp3", "xfreerdp"]),
        "wlfreerdp3" => find_tool(&["wlfreerdp3", "wlfreerdp"]),
        _ => None,
    }
}
fn version(path: &std::path::Path) -> Option<String> {
    let version = capture(path, &["/version"]).ok()?;
    // Restrict the adapter to the reviewed command-line major; the actual
    // binary's help must also advertise the options used below.
    if !version
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .any(|word| word.starts_with("3."))
    {
        return None;
    }
    let help = capture(path, &["/help"]).ok()?;
    if !help.contains("fingerprint:") || !help.contains("program:") || !help.contains("deny") {
        return None;
    }
    Some(version.trim().chars().take(256).collect())
}
pub(crate) fn inventory() -> Vec<ClientCapability> {
    ["xfreerdp3", "wlfreerdp3"].into_iter().filter_map(|id| {
        let path = executable(id)?;
        let trust = super::client_trust::verify(&path);
        let version = if trust.is_ok() { version(&path) } else { None };
        let remote_app = trust.is_ok() && capture(&path,&["/buildconfig"]).ok().is_some_and(|configuration|
            configuration.split_whitespace().any(|option| option=="CHANNEL_RAIL_CLIENT=ON"));
        let display = if id == "xfreerdp3" { std::env::var_os("DISPLAY") }
            else { std::env::var_os("WAYLAND_DISPLAY") };
        let display_present = display.is_some_and(|value| !value.is_empty());
        let terminal = std::io::stdin().is_terminal();
        let detail = if let Err(error) = &trust { error.clone() }
            else if version.is_none() { "Installed FreeRDP version/help does not prove the required version-3 fingerprint/deny/RemoteApp command-line contract".into() }
            else if !display_present { format!("{id} has no active {} display in this caller's session", if id=="xfreerdp3" { "X11" } else { "Wayland" }) }
            else if !terminal { "FreeRDP's native credential prompt needs an interactive terminal; this GUI/MCP/app caller has none. No password is accepted by the adapter".into() }
            else { "FreeRDP certificate verification accepts the peer pin OR native CA/known-host trust; no global certificate override is present. Unknown certificates are denied. Credentials stay in the native terminal prompt; exclusive peer pinning is unavailable".into() };
        Some(ClientCapability { id: id.into(), kind: ClientKind::FreeRdp3,
            available: version.is_some() && trust.is_ok() && display_present && terminal, version,
            certificate_policy: CertificatePolicy::PeerPinOrNativeTrust, remote_app,
            detail })
    }).collect()
}
pub(crate) fn launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
) -> Result<ClientProcess, String> {
    let certificate = service
        .certificate_sha256
        .as_ref()
        .ok_or_else(|| "Host certificate fingerprint is unavailable".to_string())?;
    let executable = executable(&request.client_id)
        .ok_or_else(|| "FreeRDP executable is unavailable".to_string())?;
    super::client_trust::verify(&executable)?;
    let mut command = Command::new(executable);
    command
        .arg(format!("/v:{listener}"))
        .arg(format!(
            "/cert:deny,fingerprint:sha256:{}",
            certificate.as_str()
        ))
        .arg("/sec:rdp:off"); // preserve TLS/NLA; forbid classic unauthenticated RDP security.
    if let Some(alias) = &request.remote_app {
        command.arg(format!("/app:program:||{alias}"));
    }
    let child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Cannot start installed FreeRDP client: {e}"))?;
    Ok(ClientProcess {
        child,
        resources: Box::new(()),
    })
}
pub(crate) fn hide_probe_console(_: &mut Command) {}
