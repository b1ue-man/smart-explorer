//! Which kind of graphical session this process belongs to (SCR-M5, U19-3):
//! Wayland (portal and PipeWire), X11 (MIT-SHM) or none. A process without
//! the session variables, such as a root worker without a display, follows
//! the active graphical session of `seat0` (E23, `active_session`): it is of
//! that session's kind and starts its screen host and indicator window
//! there; without a uniquely verified active session it shows no screen.

#[path = "active_session.rs"]
pub mod active_session;

/// The graphical session of a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
    None,
}

/// The kind of session `env` describes: `WAYLAND_DISPLAY` set and not empty
/// or `XDG_SESSION_TYPE=wayland` is Wayland (also when `DISPLAY` names an
/// XWayland server: the portal decides then, not the X server); else a
/// non-empty `DISPLAY` is X11; else none.
pub fn detect(env: &dyn Fn(&str) -> Option<String>) -> SessionKind {
    let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    if set("WAYLAND_DISPLAY") || env("XDG_SESSION_TYPE").is_some_and(|kind| kind == "wayland") {
        SessionKind::Wayland
    } else if set("DISPLAY") {
        SessionKind::X11
    } else {
        SessionKind::None
    }
}

/// The kind of session of this process: its own variables decide when it
/// has them (unchanged path); otherwise the active session it follows.
pub fn detect_process() -> SessionKind {
    match detect(&|name| std::env::var(name).ok()) {
        SessionKind::None => {
            active_session::current().map_or(SessionKind::None, |session| session.kind)
        }
        own => own,
    }
}

/// Whether the desktop in `XDG_CURRENT_DESKTOP` is built on wlroots; their
/// portal backend keeps a screen share only with `XDPW_PERSIST_MODE`.
pub fn is_wlroots_desktop(current_desktop: &str) -> bool {
    let lower = current_desktop.to_ascii_lowercase();
    ["sway", "hyprland", "river", "wayfire", "labwc", "niri"]
        .iter()
        .any(|name| lower.contains(name))
}
