//! FreeRDP 3 clients on Linux (design 2.4). Order: `sdl-freerdp3` (own
//! graphical credential and certificate dialogs, no terminal needed), then
//! `xfreerdp3` in an X11 session or `wlfreerdp3` in a Wayland session. The
//! certificate pin stays `/cert:deny,fingerprint:sha256:<pin>`; credentials
//! se supplies travel only through `/args-from:stdin`, never in argv.
use crate::rdp::core::{
    CertificatePolicy, ClientCapability, ClientKind, HostService, StartRequest,
};
use crate::rdp::os::shared::clients::{ClientProcess, Credentials, NEEDS_SDL};
use crate::rdp::os::shared::process::{capture, find_tool};
use std::io::{IsTerminal, Write};
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
};

const SDL: &str = "sdl-freerdp3";

fn executable(id: &str) -> Option<PathBuf> {
    match id {
        SDL => find_tool(&["sdl-freerdp3", "sdl-freerdp"]),
        "xfreerdp3" => find_tool(&["xfreerdp3", "xfreerdp"]),
        "wlfreerdp3" => find_tool(&["wlfreerdp3", "wlfreerdp"]),
        _ => None,
    }
}
fn version(path: &std::path::Path) -> Option<(String, bool)> {
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
    let args_from = help.contains("args-from");
    Some((version.trim().chars().take(256).collect(), args_from))
}
fn present(variable: &str) -> bool {
    std::env::var_os(variable).is_some_and(|value| !value.is_empty())
}
/// The client order of design 2.4: SDL first, then the client of this
/// session type (`XDG_SESSION_TYPE`).
fn order() -> [&'static str; 3] {
    if std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind == "wayland") {
        [SDL, "wlfreerdp3", "xfreerdp3"]
    } else {
        [SDL, "xfreerdp3", "wlfreerdp3"]
    }
}
/// `xfreerdp3`/`wlfreerdp3` without a terminal cannot ask for an account
/// password; se-supplied credentials (`/args-from:stdin`) replace the prompt.
pub(crate) fn needs_credential_input(client_id: &str) -> bool {
    client_id != SDL && !std::io::stdin().is_terminal()
}
pub(crate) fn inventory() -> Vec<ClientCapability> {
    order().into_iter().filter_map(|id| {
        let path = executable(id)?;
        let trust = super::client_trust::verify(&path);
        let version = if trust.is_ok() { version(&path) } else { None };
        let remote_app = trust.is_ok() && capture(&path,&["/buildconfig"]).ok().is_some_and(|configuration|
            configuration.split_whitespace().any(|option| option=="CHANNEL_RAIL_CLIENT=ON"));
        let display_present = match id {
            SDL => present("DISPLAY") || present("WAYLAND_DISPLAY"),
            "xfreerdp3" => present("DISPLAY"),
            _ => present("WAYLAND_DISPLAY"),
        };
        let detail = if let Err(error) = &trust { error.clone() }
            else if version.is_none() { "Installed FreeRDP version/help does not prove the required version-3 fingerprint/deny/RemoteApp command-line contract".into() }
            else if !display_present { format!("{id} has no active graphical display in this caller's session") }
            else if id == SDL { "FreeRDP SDL client: certificate pinned to the authenticated host fingerprint; account credentials in the client's own dialog".into() }
            else if needs_credential_input(id) { format!("{NEEDS_SDL}; this client asks for account credentials only in a terminal (se can hand over credentials it manages)") }
            else { "FreeRDP certificate verification accepts the peer pin OR native CA/known-host trust; no global certificate override is present. Unknown certificates are denied. Credentials stay in the native terminal prompt".into() };
        Some(ClientCapability { id: id.into(), kind: ClientKind::FreeRdp3,
            available: version.is_some() && trust.is_ok() && display_present,
            version: version.map(|(version, _)| version),
            certificate_policy: CertificatePolicy::PeerPinOrNativeTrust, remote_app,
            detail })
    }).collect()
}
pub(crate) fn launch(
    request: &StartRequest,
    service: &HostService,
    listener: SocketAddr,
    credentials: Option<&Credentials>,
) -> Result<ClientProcess, String> {
    let certificate = service
        .certificate_sha256
        .as_ref()
        .ok_or_else(|| "Host certificate fingerprint is unavailable".to_string())?;
    let executable = executable(&request.client_id)
        .ok_or_else(|| "FreeRDP executable is unavailable".to_string())?;
    super::client_trust::verify(&executable)?;
    let mut args = vec![
        format!("/v:{listener}"),
        format!("/cert:deny,fingerprint:sha256:{}", certificate.as_str()),
        // Preserve TLS/NLA; forbid classic unauthenticated RDP security.
        "/sec:rdp:off".to_string(),
    ];
    if let Some(alias) = &request.remote_app {
        args.push(format!("/app:program:||{alias}"));
    }
    let Some(credentials) = credentials else {
        if needs_credential_input(&request.client_id) {
            return Err(NEEDS_SDL.into());
        }
        let child = Command::new(executable)
            .args(&args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Cannot start installed FreeRDP client: {e}"))?;
        return Ok(ClientProcess {
            child,
            resources: Box::new(()),
        });
    };
    if !version(&executable).is_some_and(|(_, args_from)| args_from) {
        return Err(
            "Installed FreeRDP has no /args-from; se cannot hand over credentials without argv"
                .into(),
        );
    }
    launch_with_credentials(executable, args, credentials)
}
/// FreeRDP 3 `client/common/cmdline.h`: with `/args-from:stdin` every
/// argument comes from stdin, one per line, and none from argv.
fn launch_with_credentials(
    executable: PathBuf,
    mut args: Vec<String>,
    credentials: &Credentials,
) -> Result<ClientProcess, String> {
    let password = credentials.password.expose();
    let line_safe = |value: &str| !value.contains(['\n', '\r', '\0']);
    if !line_safe(&credentials.username)
        || !credentials.domain.as_deref().is_none_or(line_safe)
        || !line_safe(password)
    {
        return Err("RDP credentials contain a line break and cannot be handed over".into());
    }
    args.push(format!("/u:{}", credentials.username));
    if let Some(domain) = &credentials.domain {
        args.push(format!("/d:{domain}"));
    }
    let mut child = Command::new(executable)
        .arg("/args-from:stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Cannot start installed FreeRDP client: {e}"))?;
    let mut input = Vec::new();
    for arg in &args {
        input.extend_from_slice(arg.as_bytes());
        input.push(b'\n');
    }
    input.extend_from_slice(b"/p:");
    input.extend_from_slice(password.as_bytes());
    input.push(b'\n');
    let written = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(&input).and_then(|()| stdin.flush()),
        None => Err(std::io::Error::other("FreeRDP stdin is unavailable")),
    };
    // The only copy of the password outside `Credentials` is overwritten.
    input.fill(0);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("FreeRDP did not take its arguments: {error}"));
    }
    Ok(ClientProcess {
        child,
        resources: Box::new(()),
    })
}
/// FreeRDP keeps no se-written trust: the pin travels as an argument.
pub(crate) fn prepare_trust(
    _: SocketAddr,
    _: &HostService,
    _: Option<&Credentials>,
) -> Result<Box<dyn Send>, String> {
    Ok(Box::new(()))
}
/// Holds a pidfd of the bound client (design 2.6) and calls `on_exit` once
/// the process ended. The start time is checked after the pidfd is open, so
/// a reused PID is never watched.
pub(crate) fn watch_process_exit(
    pid: u32,
    started: u64,
    on_exit: Box<dyn FnOnce() + Send>,
) -> Result<(), String> {
    let raw = libc::pid_t::try_from(pid).map_err(|_| format!("PID {pid} is out of range"))?;
    // SAFETY: pidfd_open takes a PID and flags and returns a new descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, raw, 0) };
    if fd < 0 {
        return Err(format!(
            "Prozess {pid} nicht beobachtbar (pidfd_open, Linux ab 5.3): {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: a successful pidfd_open returned this descriptor; it is owned
    // here exactly once.
    let fd = unsafe {
        <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd as std::os::fd::RawFd)
    };
    let identity = crate::share::os_sockets::process_identity(pid)
        .map_err(|error| format!("Prozess {pid} nicht prüfbar: {error}"))?;
    if identity.started != started {
        return Err(format!(
            "Prozess {pid} wurde neu gestartet; Bindung abgelehnt"
        ));
    }
    std::thread::Builder::new()
        .name("rdp-client-exit".into())
        .spawn(move || {
            use std::os::fd::AsRawFd;
            let mut poll = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            loop {
                // SAFETY: one valid pollfd; -1 waits until the process ends.
                let ready = unsafe { libc::poll(&mut poll, 1, -1) };
                let interrupted =
                    std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted;
                // Any other poll error ends the binding too: the tunnel then
                // closes, the safe direction.
                if ready > 0 || !interrupted {
                    break;
                }
            }
            drop(fd);
            on_exit();
        })
        .map(|_| ())
        .map_err(|error| format!("Prozessbeobachtung nicht gestartet: {error}"))
}
pub(crate) fn hide_probe_console(_: &mut Command) {}
