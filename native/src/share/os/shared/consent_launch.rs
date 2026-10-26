//! How a desktop host asks its user to confirm an access (PRM-M3, C5): with
//! a graphical session the worker starts the window `smart_explorer
//! --access-consent <ID>` in its own session and shows a desktop
//! notification; the window does not reliably come to the front (Windows
//! flashes it in the taskbar, under Wayland the compositor decides), so the
//! notification names it. Without a graphical session (root worker, server)
//! nobody is asked actively: `se share exec approvals` and the page „Befehle“
//! answer (`cli_only`). Android asks through the app's notifier (PRM-M10);
//! until the app sets it, nobody can be asked there.
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use crate::share::access_approval::{
    ApprovalId, ApprovalService, ApprovalView, ConsentNotifier, NoConsent,
};
use crate::share::access_grant::Capability;
use crate::share::exec_access_view::ApprovalSurface;

/// The V9 start switch of the window (`StartRequest::AccessConsent`).
const ACCESS_CONSENT_SWITCH: &str = "--access-consent";

/// The approval service of this process.
pub(crate) fn approvals() -> &'static ApprovalService {
    static SERVICE: OnceLock<ApprovalService> = OnceLock::new();
    SERVICE.get_or_init(|| {
        let notifier: Arc<dyn ConsentNotifier> = if cfg!(target_os = "android") {
            Arc::new(NoConsent)
        } else {
            Arc::new(DesktopConsent::new(SessionFacts::from_env()))
        };
        ApprovalService::new(notifier)
    })
}

/// What decides how a desktop host asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionFacts {
    pub(crate) graphical: bool,
}

impl SessionFacts {
    pub(crate) fn from_env() -> Self {
        let display = std::env::var_os("DISPLAY");
        let wayland = std::env::var_os("WAYLAND_DISPLAY");
        Self::from_values(display.as_deref(), wayland.as_deref(), cfg!(windows))
    }

    /// Windows: the worker runs in its user's session. Elsewhere `DISPLAY` or
    /// `WAYLAND_DISPLAY` is set and not empty.
    pub(crate) fn from_values(
        display: Option<&OsStr>,
        wayland: Option<&OsStr>,
        windows: bool,
    ) -> Self {
        let set = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
        Self {
            graphical: windows || set(display) || set(wayland),
        }
    }
}

/// Asks through the consent window and a desktop notification.
pub(crate) struct DesktopConsent {
    facts: SessionFacts,
    launch: fn(&ApprovalId) -> io::Result<()>,
    notify: fn(&str, &str),
}

impl DesktopConsent {
    pub(crate) fn new(facts: SessionFacts) -> Self {
        Self::with(facts, launch_window, crate::notify_desktop::notify_access)
    }

    /// `launch` starts the window, `notify` shows title and text.
    pub(crate) fn with(
        facts: SessionFacts,
        launch: fn(&ApprovalId) -> io::Result<()>,
        notify: fn(&str, &str),
    ) -> Self {
        Self {
            facts,
            launch,
            notify,
        }
    }
}

impl ConsentNotifier for DesktopConsent {
    fn supports_contextual(&self) -> bool { true }

    fn show(&self, request: &ApprovalView) -> Option<ApprovalSurface> {
        if !self.facts.graphical {
            return Some(ApprovalSurface::CliOnly);
        }
        let text = format!(
            "{}. Bestätigen im Fenster „Zugriff bestätigen“ oder mit: se share exec approvals",
            request_line(request)
        );
        (self.notify)("Zugriff bestätigen", &text);
        match (self.launch)(&request.id) {
            Ok(()) => Some(ApprovalSurface::Window),
            Err(error) => {
                crate::daemon::log_worker(&format!(
                    "Rückfragefenster nicht gestartet: {error}; die Benachrichtigung bleibt"
                ));
                Some(ApprovalSurface::Notification)
            }
        }
    }
}

/// „{Gerät} möchte …“ of a request (window and notification).
pub(crate) fn request_line(request: &ApprovalView) -> String {
    let wish = match request.capability {
        Capability::ExecCommands => "Befehle ausführen",
        Capability::ExecTerminal => "ein Terminal öffnen",
        Capability::ScreenView => "den Bildschirm ansehen",
        Capability::ScreenControl => "den Bildschirm steuern",
        Capability::AppLaunch => "eine App starten",
        Capability::LoginAccess => "auf eine Anmeldung oder Sitzung zugreifen",
        Capability::TcpForward => "einen TCP-Dienst weiterleiten",
    };
    format!("{} möchte {wish}", request.device_name)
}

/// The capability as the dialogs name it.
pub(crate) fn capability_label(capability: Capability) -> &'static str {
    match capability {
        Capability::ExecCommands => "Befehle",
        Capability::ExecTerminal => "Terminal",
        Capability::ScreenView => "Bildschirm ansehen",
        Capability::ScreenControl => "Bildschirm steuern",
        Capability::AppLaunch => "App starten",
        Capability::LoginAccess => "Anmeldung und Sitzung",
        Capability::TcpForward => "TCP-Weiterleitung",
    }
}

/// A contextual Allow is unavailable when an older reader lost the target
/// or exact relation. Legacy Exec/terminal/screen questions retain their UI.
pub(crate) fn approval_context_complete(view: &ApprovalView) -> bool {
    match &view.request {
        Some(request) => !view.relation_id.is_empty() && request.capability() == view.capability,
        None => !view.capability.requires_context(),
    }
}

/// Full, shared target text for desktop, Android and CLI. Identifiers are
/// intentionally not shortened: a different session, user or host is a
/// different approval. The typed requests reject control/bidi identities.
pub(crate) fn approval_context(view: &ApprovalView) -> Vec<String> {
    use crate::share::access_constraints::{AccessRequest, LoginOperation, TcpDirection, TcpHost};
    let mut lines = Vec::new();
    if !view.relation_id.is_empty() {
        lines.push(format!("Quelle der Freigabe: {} / {}", view.relation, shown_relation(&view.relation_id)));
    }
    match &view.request {
        Some(AccessRequest::AppLaunch(request)) => {
            lines.push("Zielgerät: dieses Gerät".into());
            lines.push(format!("App-ID: {}", request.app_id.as_str()));
            lines.push(format!("Sitzung: {}", request.session.as_str()));
            lines.push(format!("Benutzer: {}", request.user.as_str()));
        }
        Some(AccessRequest::LoginAccess(request)) => {
            let operation = match request.operation {
                LoginOperation::Login => "Anmelden",
                LoginOperation::Unlock => "Entsperren",
                LoginOperation::SecureDesktop => "Geschützter Desktop",
                LoginOperation::UserSession => "Benutzersitzung",
            };
            lines.push("Zielgerät: dieses Gerät".into());
            lines.push(format!("Vorgang: {operation}"));
            lines.push(format!("Sitzung: {}", request.session.as_str()));
            lines.push(format!("Benutzer: {}", request.user.as_str()));
        }
        Some(AccessRequest::TcpForward(request)) => {
            lines.push(format!("Initiatorgerät: {}", request.initiator.as_str()));
            lines.push(format!("Zielgerät: {}", request.target.as_str()));
            lines.push(format!("Richtung: {}", match request.direction {
                TcpDirection::PeerToHost => "Anfragendes Gerät → dieser Host",
                TcpDirection::HostToPeer => "Dieser Host → anfragendes Gerät",
            }));
            match &request.host {
                TcpHost::Loopback => lines.push("Zielhost: Loopback des Zielgeräts".into()),
                TcpHost::Ip { address } => lines.push(format!("Zielhost: {address}")),
                TcpHost::Dns { name, addresses } => {
                    lines.push(format!("Zielhost: {name}"));
                    if !addresses.is_empty() {
                        lines.push(format!("Mitgeteilte DNS-Adressen: {}",
                            addresses.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")));
                    }
                }
            }
            lines.push(format!("Frisch aufgelöste Ziel-IP: {}", request.resolved_ip));
            lines.push(format!("Zielport: {}", request.port.get()));
        }
        None => {},
    }
    if !approval_context_complete(view) {
        lines.push("Der vollständige Zielauftrag oder seine Beziehung fehlt; Erlauben ist gesperrt.".into());
    }
    lines
}
fn shown_relation(value: &str) -> String {
    value.chars().map(|character| {
        if character.is_control() || matches!(character, '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}') {
            format!("\\u{{{:x}}}", u32::from(character))
        } else { character.to_string() }
    }).collect()
}

/// Starts the window of `id` with the desktop program of this installation.
fn launch_window(id: &ApprovalId) -> io::Result<()> {
    let current = std::env::current_exe()?;
    let program = gui_program(&current, Path::is_file).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the desktop program is not installed next to this program",
        )
    })?;
    let mut child = Command::new(program)
        .arg(ACCESS_CONSENT_SWITCH)
        .arg(id.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Collects the window's exit so it leaves no zombie behind.
    let _ = std::thread::Builder::new()
        .name("access-consent-window".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// The desktop program: the worker itself when it is one, else the one next
/// to the command-line program `se`.
pub(crate) fn gui_program(current: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let name = current.file_name()?.to_str()?.to_ascii_lowercase();
    if name != "se" && name != "se.exe" {
        return Some(current.to_path_buf());
    }
    let names: &[&str] = if cfg!(windows) {
        &["Smart Explorer.exe", "smart_explorer.exe"]
    } else {
        &["smart_explorer"]
    };
    names
        .iter()
        .map(|name| current.with_file_name(name))
        .find(|candidate| exists(candidate))
}
