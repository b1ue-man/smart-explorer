//! Which desktop layer a session offers (C2.2 for Linux, C1.2 for Windows) and
//! how the surface background behaves there. Pure tables over the facts that
//! `os/linux_os/session_facts.rs` and the Windows adapter collect.
use super::config::ForeignDesktopIcons;

/// The display server of the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DisplayServer {
    Windows,
    /// `DISPLAY` without `WAYLAND_DISPLAY`.
    X11,
    /// `WAYLAND_DISPLAY` (XWayland is never used for the surface, C2.1 L10).
    Wayland,
    /// Neither a Windows desktop nor `DISPLAY`/`WAYLAND_DISPLAY`.
    #[default]
    Headless,
}

/// What the X11 window manager announces in `_NET_SUPPORTED`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NetSupported {
    pub(crate) window_type_desktop: bool,
    pub(crate) state_below: bool,
}

/// Facts of the running session, collected by the os layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionFacts {
    pub(crate) display: DisplayServer,
    /// `XDG_CURRENT_DESKTOP`, split at `:`.
    pub(crate) current_desktop: Vec<String>,
    /// X11: `None` when `_NET_SUPPORTED` is missing.
    pub(crate) net_supported: Option<NetSupported>,
    /// X11: a window of type DESKTOP in `_NET_CLIENT_LIST_STACKING`.
    pub(crate) foreign_desktop_window: bool,
    /// Wayland: version of the `zwlr_layer_shell_v1` global.
    pub(crate) layer_shell_version: Option<u32>,
    /// COSMIC: a `cosmic-files-applet` of the same user runs.
    pub(crate) cosmic_files_desktop: bool,
    /// GNOME: major version of `org.gnome.Shell` `ShellVersion`.
    pub(crate) gnome_shell_major: Option<u32>,
    /// GNOME: UUID of an active desktop-icons extension (DING, C2.5).
    pub(crate) foreign_desktop_icons: Option<String>,
    /// The surface gets an alpha channel (Annahme A22).
    pub(crate) alpha_supported: bool,
}

impl SessionFacts {
    fn desktop_is(&self, name: &str) -> bool {
        self.current_desktop
            .iter()
            .any(|entry| entry.trim().eq_ignore_ascii_case(name))
    }
}

/// Keyboard of a layer surface (C2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerKeyboard {
    /// layer-shell version 4 and later.
    OnDemand,
    /// Older versions know only `none` and `exclusive`: no keyboard.
    Off,
}

/// Namespace of a layer surface: `desktop` under KWin, so that "Desktop
/// anzeigen" keeps it (Ä9), `smart-explorer-desktop` elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerNamespace {
    Desktop,
    SmartExplorer,
}

impl LayerNamespace {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            LayerNamespace::Desktop => "desktop",
            LayerNamespace::SmartExplorer => "smart-explorer-desktop",
        }
    }
}

/// GNOME extension variant (C2.4): A for Shell 45–48, B for 49–51.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GnomeVariant {
    A,
    B,
}

/// Why the surface is not available in this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    NoGraphicalSession,
    NoEwmhDesktop,
    ForeignDesktopWithoutBelow,
    CosmicFilesActive,
    GnomeVersionUnknown,
    GnomeTooOld,
    GnomeVersionUnlisted(u32),
    ForeignDesktopIcons(String),
    NoDesktopLayer,
}

impl Reason {
    /// Code for `none:<Grund>` and `mechanism_reason`.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Reason::NoGraphicalSession => "no_graphical_session",
            Reason::NoEwmhDesktop => "no_ewmh_desktop",
            Reason::ForeignDesktopWithoutBelow => "foreign_desktop_without_below",
            Reason::CosmicFilesActive => "cosmic_files_active",
            Reason::GnomeVersionUnknown => "gnome_version_unknown",
            Reason::GnomeTooOld => "gnome_too_old",
            Reason::GnomeVersionUnlisted(_) => "unsupported_shell_version",
            Reason::ForeignDesktopIcons(_) => "foreign_desktop_icons",
            Reason::NoDesktopLayer => "no_desktop_layer",
        }
    }

    /// The sentence after "nicht verfügbar: " in status and settings.
    pub(crate) fn text(&self) -> String {
        match self {
            Reason::NoGraphicalSession => "Keine grafische Sitzung erkannt".into(),
            Reason::NoEwmhDesktop => "Der Fenstermanager unterstützt weder \
                _NET_WM_WINDOW_TYPE_DESKTOP noch _NET_WM_STATE_BELOW"
                .into(),
            Reason::ForeignDesktopWithoutBelow => "Ein anderes Desktopprogramm ist aktiv, und \
                der Fenstermanager unterstützt _NET_WM_STATE_BELOW nicht"
                .into(),
            Reason::CosmicFilesActive => "COSMIC-Desktopsymbole (COSMIC Files) sind aktiv; \
                beide Flächen lägen in derselben Ebene ohne festgelegte Reihenfolge. Schalte \
                die Desktopsymbole in den COSMIC-Einstellungen aus."
                .into(),
            Reason::GnomeVersionUnknown => "Die Version der GNOME Shell ist unbekannt".into(),
            Reason::GnomeTooOld => "GNOME Shell < 45 wird nicht unterstützt".into(),
            Reason::GnomeVersionUnlisted(major) => {
                format!(
                    "GNOME Shell {major} ist noch nicht in der Versionsliste der se-Erweiterung"
                )
            }
            Reason::ForeignDesktopIcons(_) => {
                "Ubuntu-Desktopsymbole sind aktiv; se Desktop kann sie übernehmen".into()
            }
            Reason::NoDesktopLayer => {
                "Diese Sitzung bietet keine Desktop-Ebene (kein wlr-layer-shell)".into()
            }
        }
    }
}

/// How the surface lies on the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mechanism {
    /// Windows: lowest borderless tool window per monitor (C1.2).
    WindowsBottom,
    /// X11 window type DESKTOP, no foreign desktop window (L1).
    X11DesktopType,
    /// X11 `_NET_WM_STATE_BELOW` (L2).
    X11Below,
    /// Own layer backend, layer `bottom` on every output (L3).
    LayerShell {
        keyboard: LayerKeyboard,
        namespace: LayerNamespace,
    },
    /// Own GNOME Shell extension starts the surfaces (L4).
    GnomeExtension {
        variant: GnomeVariant,
    },
    Unavailable(Reason),
}

impl Mechanism {
    /// `mechanism` of the status (`none` with [`Reason::code`]).
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Mechanism::WindowsBottom => "windows_bottom",
            Mechanism::X11DesktopType => "x11_desktop_type",
            Mechanism::X11Below => "x11_below",
            Mechanism::LayerShell { .. } => "layer_shell",
            Mechanism::GnomeExtension { .. } => "gnome_extension",
            Mechanism::Unavailable(_) => "none",
        }
    }
}

/// Background with `background = system` (C2.2 column "Hintergrund").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackgroundPolicy {
    /// Draws the system wallpaper (or colour) itself.
    Opaque,
    /// Lets the compositor's wallpaper show through.
    Transparent,
}

/// The mechanism of a session (C2.2; Windows always L3 of C1).
pub(crate) fn decide(facts: &SessionFacts, foreign_icons: ForeignDesktopIcons) -> Mechanism {
    match facts.display {
        DisplayServer::Windows => Mechanism::WindowsBottom,
        DisplayServer::X11 => decide_x11(facts),
        DisplayServer::Wayland => decide_wayland(facts, foreign_icons),
        DisplayServer::Headless => Mechanism::Unavailable(Reason::NoGraphicalSession),
    }
}

fn decide_x11(facts: &SessionFacts) -> Mechanism {
    let supported = facts.net_supported.unwrap_or_default();
    if facts.foreign_desktop_window {
        // Two DESKTOP windows have no defined order (EWMH).
        return if supported.state_below {
            Mechanism::X11Below
        } else if supported.window_type_desktop {
            Mechanism::Unavailable(Reason::ForeignDesktopWithoutBelow)
        } else {
            Mechanism::Unavailable(Reason::NoEwmhDesktop)
        };
    }
    if supported.window_type_desktop {
        Mechanism::X11DesktopType
    } else if supported.state_below {
        Mechanism::X11Below
    } else {
        Mechanism::Unavailable(Reason::NoEwmhDesktop)
    }
}

fn decide_wayland(facts: &SessionFacts, foreign_icons: ForeignDesktopIcons) -> Mechanism {
    if let Some(version) = facts.layer_shell_version {
        if facts.desktop_is("COSMIC") && facts.cosmic_files_desktop {
            return Mechanism::Unavailable(Reason::CosmicFilesActive);
        }
        return Mechanism::LayerShell {
            keyboard: if version >= 4 {
                LayerKeyboard::OnDemand
            } else {
                LayerKeyboard::Off
            },
            namespace: if facts.desktop_is("KDE") {
                LayerNamespace::Desktop
            } else {
                LayerNamespace::SmartExplorer
            },
        };
    }
    if !facts.desktop_is("GNOME") {
        return Mechanism::Unavailable(Reason::NoDesktopLayer);
    }
    let variant = match facts.gnome_shell_major {
        None => return Mechanism::Unavailable(Reason::GnomeVersionUnknown),
        Some(0..=44) => return Mechanism::Unavailable(Reason::GnomeTooOld),
        Some(45..=48) => GnomeVariant::A,
        Some(49..=51) => GnomeVariant::B,
        Some(major) => return Mechanism::Unavailable(Reason::GnomeVersionUnlisted(major)),
    };
    match (&facts.foreign_desktop_icons, foreign_icons) {
        (Some(uuid), ForeignDesktopIcons::Keep) => {
            Mechanism::Unavailable(Reason::ForeignDesktopIcons(uuid.clone()))
        }
        _ => Mechanism::GnomeExtension { variant },
    }
}

/// Whether foreign desktop icons lie below the surface (Explorer, a foreign
/// X11 desktop, the Plasma desktop under KWin).
pub(crate) fn icons_below(mechanism: &Mechanism, facts: &SessionFacts) -> bool {
    match mechanism {
        Mechanism::WindowsBottom => true,
        Mechanism::X11Below => facts.foreign_desktop_window,
        Mechanism::LayerShell { namespace, .. } => *namespace == LayerNamespace::Desktop,
        Mechanism::X11DesktopType
        | Mechanism::GnomeExtension { .. }
        | Mechanism::Unavailable(_) => false,
    }
}

/// Background policy for `background = system` (C2.2, A22).
pub(crate) fn background_policy(mechanism: &Mechanism, facts: &SessionFacts) -> BackgroundPolicy {
    let see_through = match mechanism {
        Mechanism::LayerShell { .. } | Mechanism::GnomeExtension { .. } => {
            !icons_below(mechanism, facts) && facts.alpha_supported
        }
        Mechanism::WindowsBottom
        | Mechanism::X11DesktopType
        | Mechanism::X11Below
        | Mechanism::Unavailable(_) => false,
    };
    if see_through {
        BackgroundPolicy::Transparent
    } else {
        BackgroundPolicy::Opaque
    }
}

/// `background = transparent` is allowed only where nothing lies below the
/// surface and the surface gets an alpha channel (C5.4); else the reason.
pub(crate) fn transparent_allowed(
    mechanism: &Mechanism,
    facts: &SessionFacts,
) -> Result<(), String> {
    if let Mechanism::Unavailable(reason) = mechanism {
        return Err(format!("se Desktop ist nicht verfügbar: {}", reason.text()));
    }
    if icons_below(mechanism, facts) {
        return Err("Unter der Fläche liegen Desktopsymbole; sie würden durchscheinen".into());
    }
    if !facts.alpha_supported {
        return Err("Die Sitzung liefert der Fläche keinen Alphakanal".into());
    }
    Ok(())
}
