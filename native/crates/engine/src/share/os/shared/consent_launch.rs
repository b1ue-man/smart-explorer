//! How a desktop host asks its user to confirm an access (PRM-M3, C5): with
//! a graphical session the worker starts the window `smart_explorer
//! --access-consent <ID>` in its own session and shows a desktop
//! notification; the window does not reliably come to the front (Windows
//! flashes it in the taskbar, under Wayland the compositor decides), so the
//! notification names it. Without a graphical session (root worker, server)
//! nobody is asked actively: `se share exec approvals` and the page „Befehle“
//! answer (`cli_only`). Android asks through the app's notifier (PRM-M10);
//! until the app sets it, nobody can be asked there. A Linux worker without
//! session variables (a root service, E23) asks in the active graphical
//! session of `seat0` it follows: the window starts there with that
//! session's variables and as its account; without a uniquely verified
//! session it answers `cli_only` as before. Such a window reaches the
//! worker through a channel limited to its requests (E46, S13,
//! `spawn_window_with_channel`) and sends the notification itself, from
//! the session's account to its session bus; the worker does not. The
//! requests of one device share one window and one notification (E28): the
//! window lists all of them, and its exit is reported to the service
//! (`window_ended`).
use std::ffi::OsStr;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};

use crate::share::access_approval::{
    ApprovalId, ApprovalService, ApprovalView, ConsentNotifier, NoConsent,
};
use crate::share::access_grant::Capability;
use crate::share::exec_access_view::ApprovalSurface;

#[cfg(target_os = "linux")]
use crate::screen::os::session_detect::active_session::{
    current_generation, process::WindowHandle, spawn as session_target,
};

/// No other platform runs a worker outside the user's session.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
mod session_target {
    pub struct ActiveSession;
    pub fn has_own_session(_: &dyn Fn(&str) -> Option<String>) -> bool {
        true
    }
    pub fn for_worker() -> Option<ActiveSession> {
        None
    }
    pub fn apply(_: &ActiveSession, _: &mut std::process::Command) {}
    impl ActiveSession {
        pub fn generation(&self) -> &str {
            ""
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn current_generation() -> Option<String> {
    None
}
#[cfg(not(target_os = "linux"))]
struct WindowHandle;
#[cfg(not(target_os = "linux"))]
impl WindowHandle {
    fn of(_: &Child) -> Option<Self> {
        None
    }
    fn terminate(&self) {}
}

/// The V9 start switch of the window (`StartRequest::AccessConsent`).
const ACCESS_CONSENT_SWITCH: &str = "--access-consent";
/// Title of the notification of a request.
const NOTICE_TITLE: &str = "Zugriff bestätigen";

/// The variable naming the worker IPC channel of a window started in a
/// followed session (E46, S13); its token comes on the window's stdin.
pub const WORKER_IPC_CHANNEL_ENV: &str = "SE_WORKER_IPC_CHANNEL";

/// What a window of a followed session may ask its worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowScope {
    /// Ping; list, watch and end the screen sessions.
    Indicator,
    /// Ping; list and decide the open requests of `device` (E28: one window
    /// per device, so these are the requests the window shows).
    Consent { device: String },
}

/// The followed session a window starts in: a channel ends when another
/// session is the active one (`current` names it, `None` when unknown).
pub struct WindowSession {
    pub generation: String,
    pub current: fn() -> Option<String>,
}

/// A channel of the worker IPC for one window.
pub struct WindowChannel {
    pub addr: SocketAddr,
    pub token: String,
    /// Ends the token and the channel when dropped.
    pub guard: Box<dyn WindowChannelGuard>,
}

pub trait WindowChannelGuard: Send {
    /// The window runs; `end` ends it after a session switch.
    fn window_started(&self, end: Box<dyn Fn() + Send + Sync>);
    /// The channel ended because another session became the active one.
    fn session_switched(&self) -> bool;
}

/// Opens a window channel; set by the worker's IPC listener.
pub type WindowChannelOpener = fn(WindowScope, WindowSession) -> io::Result<WindowChannel>;
static CHANNELS: OnceLock<WindowChannelOpener> = OnceLock::new();

pub fn install_window_channels(open: WindowChannelOpener) {
    let _ = CHANNELS.set(open);
}

/// Starts `command`, prepared by `session_target::apply` for the session
/// `generation`, with a channel limited to `scope`: the address goes into
/// `SE_WORKER_IPC_CHANNEL`, the token only as the first line of stdin. Keep
/// the guard until the window exited.
pub fn spawn_window_with_channel(
    command: &mut Command,
    scope: WindowScope,
    generation: &str,
) -> io::Result<(Child, Box<dyn WindowChannelGuard>)> {
    let open = CHANNELS.get().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotConnected,
            "worker IPC has no window channel",
        )
    })?;
    let session = WindowSession {
        generation: generation.to_string(),
        current: current_generation,
    };
    let channel = open(scope, session)?;
    let mut child = command
        .env(WORKER_IPC_CHANNEL_ENV, channel.addr.to_string())
        .stdin(Stdio::piped())
        .spawn()?;
    let sent = match child.stdin.take() {
        Some(mut stdin) => writeln!(stdin, "{}", channel.token),
        None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
    };
    if let Err(error) = sent {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if let Some(window) = WindowHandle::of(&child) {
        channel
            .guard
            .window_started(Box::new(move || window.terminate()));
    }
    Ok((child, channel.guard))
}

/// Whether this process is a window with a worker IPC channel.
pub fn in_window_channel() -> bool {
    std::env::var_os(WORKER_IPC_CHANNEL_ENV).is_some()
}

/// The notification of `request` from its consent window (E46): only a
/// window with a channel sends it; otherwise the worker did.
pub fn notify_from_window(request: &ApprovalView) {
    if in_window_channel() {
        crate::notify_desktop::notify_access(NOTICE_TITLE, &notice_text(request));
    }
}

fn notice_text(request: &ApprovalView) -> String {
    format!(
        "{}. Bestätigen im Fenster „Zugriff bestätigen“ oder mit: se share exec approvals",
        request_line(request)
    )
}

/// The approval service of this process.
pub fn approvals() -> &'static ApprovalService {
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
pub struct SessionFacts {
    pub graphical: bool,
}

impl SessionFacts {
    pub fn from_env() -> Self {
        let display = std::env::var_os("DISPLAY");
        let wayland = std::env::var_os("WAYLAND_DISPLAY");
        Self::from_values(display.as_deref(), wayland.as_deref(), cfg!(windows))
    }

    /// Windows: the worker runs in its user's session. Elsewhere `DISPLAY` or
    /// `WAYLAND_DISPLAY` is set and not empty.
    pub fn from_values(display: Option<&OsStr>, wayland: Option<&OsStr>, windows: bool) -> Self {
        let set = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
        Self {
            graphical: windows || set(display) || set(wayland),
        }
    }
}

/// Asks through the consent window and a desktop notification.
pub struct DesktopConsent {
    facts: SessionFacts,
    /// The worker has no session variables and follows the active session.
    follows: bool,
    launch: Launch,
    notify: fn(&str, &str),
}

/// How the window starts: by its id (callers of `with`) or by its request.
enum Launch {
    Id(fn(&ApprovalId) -> io::Result<()>),
    Request(fn(&ApprovalView) -> io::Result<()>),
}

/// Whether a host asks in a window: in its own graphical session, or in the
/// session it follows (`follows`) while `followed` names one.
pub fn asks_in_window(facts: SessionFacts, follows: bool, followed: impl FnOnce() -> bool) -> bool {
    facts.graphical || (follows && followed())
}

impl DesktopConsent {
    pub fn new(facts: SessionFacts) -> Self {
        Self {
            facts,
            follows: !session_target::has_own_session(&|name| std::env::var(name).ok()),
            launch: Launch::Request(launch_window),
            notify: crate::notify_desktop::notify_access,
        }
    }

    /// `launch` starts the window, `notify` shows title and text.
    pub fn with(
        facts: SessionFacts,
        launch: fn(&ApprovalId) -> io::Result<()>,
        notify: fn(&str, &str),
    ) -> Self {
        Self {
            facts,
            follows: false,
            launch: Launch::Id(launch),
            notify,
        }
    }
}

impl ConsentNotifier for DesktopConsent {
    fn supports_contextual(&self) -> bool {
        true
    }

    fn show(&self, request: &ApprovalView) -> Option<ApprovalSurface> {
        if !asks_in_window(self.facts, self.follows, || {
            session_target::for_worker().is_some()
        }) {
            return Some(ApprovalSurface::CliOnly);
        }
        let text = notice_text(request);
        // E46: the window of a followed session notifies on its own bus.
        if !self.follows {
            (self.notify)(NOTICE_TITLE, &text);
        }
        let launched = match self.launch {
            Launch::Id(launch) => launch(&request.id),
            Launch::Request(launch) => launch(request),
        };
        match launched {
            Ok(()) => Some(ApprovalSurface::Window),
            Err(error) => {
                if self.follows {
                    (self.notify)(NOTICE_TITLE, &text);
                }
                crate::daemon::log_worker(&format!(
                    "Rückfragefenster nicht gestartet: {error}; die Benachrichtigung bleibt"
                ));
                Some(ApprovalSurface::Notification)
            }
        }
    }
}

/// „{Gerät} möchte …“ of a request (window and notification).
pub fn request_line(request: &ApprovalView) -> String {
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
pub fn capability_label(capability: Capability) -> &'static str {
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
pub fn approval_context_complete(view: &ApprovalView) -> bool {
    match &view.request {
        Some(request) => !view.relation_id.is_empty() && request.capability() == view.capability,
        None => !view.capability.requires_context(),
    }
}

/// Full, shared target text for desktop, Android and CLI. Identifiers are
/// intentionally not shortened: a different session, user or host is a
/// different approval. The typed requests reject control/bidi identities.
pub fn approval_context(view: &ApprovalView) -> Vec<String> {
    use crate::share::access_constraints::{AccessRequest, LoginOperation, TcpDirection, TcpHost};
    let mut lines = Vec::new();
    if !view.relation_id.is_empty() {
        lines.push(format!(
            "Quelle der Freigabe: {} / {}",
            view.relation,
            shown_relation(&view.relation_id)
        ));
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
            lines.push(format!(
                "Richtung: {}",
                match request.direction {
                    TcpDirection::PeerToHost => "Anfragendes Gerät → dieser Host",
                    TcpDirection::HostToPeer => "Dieser Host → anfragendes Gerät",
                }
            ));
            match &request.host {
                TcpHost::Loopback => lines.push("Zielhost: Loopback des Zielgeräts".into()),
                TcpHost::Ip { address } => lines.push(format!("Zielhost: {address}")),
                TcpHost::Dns { name, addresses } => {
                    lines.push(format!("Zielhost: {name}"));
                    if !addresses.is_empty() {
                        lines.push(format!(
                            "Mitgeteilte DNS-Adressen: {}",
                            addresses
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                }
            }
            lines.push(format!(
                "Frisch aufgelöste Ziel-IP: {}",
                request.resolved_ip
            ));
            lines.push(format!("Zielport: {}", request.port.get()));
        }
        None => {}
    }
    if !approval_context_complete(view) {
        lines.push(
            "Der vollständige Zielauftrag oder seine Beziehung fehlt; Erlauben ist gesperrt."
                .into(),
        );
    }
    lines
}
fn shown_relation(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
            {
                format!("\\u{{{:x}}}", u32::from(character))
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// Starts the window of `request` with the desktop program of this
/// installation; in a followed session with a channel for the requests of
/// its device.
fn launch_window(request: &ApprovalView) -> io::Result<()> {
    let id = &request.id;
    let current = std::env::current_exe()?;
    let program = gui_program(&current, Path::is_file).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the desktop program is not installed next to this program",
        )
    })?;
    let mut command = Command::new(program);
    command
        .arg(ACCESS_CONSENT_SWITCH)
        .arg(id.as_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (mut child, channel) = if session_target::has_own_session(&|name| std::env::var(name).ok())
    {
        (command.stdin(Stdio::null()).spawn()?, None)
    } else {
        // Never a window as root without a session (E23, S3).
        let session = session_target::for_worker().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "no active graphical session for the consent window",
            )
        })?;
        session_target::apply(&session, &mut command);
        let scope = WindowScope::Consent {
            device: request.device_id.clone(),
        };
        let (child, guard) = spawn_window_with_channel(&mut command, scope, session.generation())?;
        (child, Some(guard))
    };
    // Collects the window's exit so it leaves no zombie behind, ends its
    // channel and reports the exit. A window ended because its session is no
    // longer the active one (E23) starts again in the active one while its
    // request is open; it stays the window of its device (E28).
    let window = id.clone();
    let _ = std::thread::Builder::new()
        .name("access-consent-window".into())
        .spawn(move || {
            let _ = child.wait();
            let switched = channel.is_some_and(|channel| channel.session_switched());
            let open = switched.then(|| approvals().open(&window)).flatten();
            if !open.is_some_and(|open| launch_window(&open.view).is_ok()) {
                approvals().window_ended(&window);
            }
        });
    Ok(())
}

/// The desktop program: the worker itself when it is one, else the one next
/// to the command-line program `se`.
pub fn gui_program(current: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
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
