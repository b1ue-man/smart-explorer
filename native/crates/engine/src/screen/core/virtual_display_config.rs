//! Files of the virtual display's driver setup (plan 2026-11-17 P5 stage 2,
//! decision E4): the Xorg configuration with the driver `dummy` (RandR 1.2
//! outputs `DUMMY0`…, every mode valid within `VideoRam`; 1048576 KiB as
//! Chrome Remote Desktop uses it) and the drop-ins that point the display
//! manager's X server at it (LightDM `xserver-command`, SDDM
//! `ServerArguments`; GDM on Xorg reads `xorg.conf.d`). Xorg resolves the
//! relative `-config` name in `/etc/X11` also for a server not run as root.
//! Pure text and decisions: the elevated step writes them
//! (`screen/os/linux_os/virtual_display_setup.rs`).
use super::pixel_convert::FrameSize;
use super::virtual_display::{cvt_rb, mode_name, REFRESH_HZ};

/// The first line of every file se writes; only such files are replaced or
/// removed.
pub const MARKER: &str = "# smart-explorer virtual display";
/// Name of the Xorg configuration (relative `-config` argument).
pub const XORG_CONF_NAME: &str = "smart-explorer-virtual.conf";
pub const XORG_CONF_PATH: &str = "/etc/X11/smart-explorer-virtual.conf";
pub const LIGHTDM_DROPIN_PATH: &str = "/etc/lightdm/lightdm.conf.d/90-smart-explorer-virtual.conf";
pub const SDDM_DROPIN_PATH: &str = "/etc/sddm.conf.d/90-smart-explorer-virtual.conf";
/// GDM on Xorg passes no `-config`; the snippet directory carries the same
/// configuration.
pub const GDM_XORG_PATH: &str = "/etc/X11/xorg.conf.d/90-smart-explorer-virtual.conf";
/// `VideoRam` of the driver in KiB: 1 GiB, one 16384 × 16384 framebuffer.
pub const VIDEO_RAM_KIB: u64 = 1_048_576;
/// The size the X server starts with before a viewer asks for another.
pub const START_SIZE: FrameSize = FrameSize::new(1920, 1080);

/// The kind of display manager of the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayManagerKind {
    LightDm,
    Sddm,
    Gdm,
    /// Another one; se cannot point it at the configuration.
    Other,
}

/// The display manager of the seat (`display-manager.service`) and the unit
/// it runs as (`gdm3.service` on Debian, `gdm.service` elsewhere).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayManager {
    pub kind: DisplayManagerKind,
    pub unit: String,
}

impl DisplayManager {
    /// From the unit `display-manager.service` links to (`lightdm.service`).
    pub fn from_unit(unit: &str) -> Self {
        let unit = unit.trim();
        let kind = match unit.trim_end_matches(".service") {
            "lightdm" => DisplayManagerKind::LightDm,
            "sddm" => DisplayManagerKind::Sddm,
            "gdm" | "gdm3" => DisplayManagerKind::Gdm,
            _ => DisplayManagerKind::Other,
        };
        Self {
            kind,
            unit: unit.to_string(),
        }
    }

    /// The drop-in of this display manager: path and text.
    pub fn dropin(&self) -> Option<(&'static str, String)> {
        match self.kind {
            DisplayManagerKind::LightDm => Some((LIGHTDM_DROPIN_PATH, lightdm_dropin())),
            DisplayManagerKind::Sddm => Some((SDDM_DROPIN_PATH, sddm_dropin())),
            DisplayManagerKind::Gdm => Some((GDM_XORG_PATH, xorg_conf())),
            DisplayManagerKind::Other => None,
        }
    }
}

/// The Xorg configuration of the virtual display.
pub fn xorg_conf() -> String {
    let name = mode_name(START_SIZE);
    let modeline = cvt_rb(START_SIZE.width, START_SIZE.height, REFRESH_HZ)
        .map(|timing| timing.modeline(&name))
        .unwrap_or_default();
    format!(
        "{MARKER}: written by se, removed with the last screen-control right.\n\
         Section \"Device\"\n\
         \x20   Identifier \"se-virtual-device\"\n\
         \x20   Driver \"dummy\"\n\
         \x20   VideoRam {VIDEO_RAM_KIB}\n\
         \x20   Option \"Monitor-DUMMY0\" \"se-virtual-monitor\"\n\
         EndSection\n\
         \n\
         Section \"Monitor\"\n\
         \x20   Identifier \"se-virtual-monitor\"\n\
         \x20   HorizSync 5.0 - 1000.0\n\
         \x20   VertRefresh 5.0 - 200.0\n\
         \x20   {modeline}\n\
         \x20   Option \"PreferredMode\" \"{name}\"\n\
         EndSection\n\
         \n\
         Section \"Screen\"\n\
         \x20   Identifier \"se-virtual-screen\"\n\
         \x20   Device \"se-virtual-device\"\n\
         \x20   Monitor \"se-virtual-monitor\"\n\
         \x20   DefaultDepth 24\n\
         \x20   SubSection \"Display\"\n\
         \x20       Depth 24\n\
         \x20   EndSubSection\n\
         EndSection\n\
         \n\
         Section \"ServerLayout\"\n\
         \x20   Identifier \"se-virtual-layout\"\n\
         \x20   Screen \"se-virtual-screen\"\n\
         EndSection\n"
    )
}

/// LightDM: every seat's X server reads se's configuration.
pub fn lightdm_dropin() -> String {
    format!("{MARKER}\n[Seat:*]\nxserver-command=X -config {XORG_CONF_NAME}\n")
}

/// SDDM: its default arguments (`-nolisten tcp`) and se's configuration.
pub fn sddm_dropin() -> String {
    format!("{MARKER}\n[X11]\nServerArguments=-nolisten tcp -config {XORG_CONF_NAME}\n")
}

/// The package of the Xorg driver `dummy` for the distribution named by
/// `/etc/os-release` (`ID` and `ID_LIKE`); `None`: unknown, reported as the
/// limit of this host.
pub fn dummy_package(ids: &[&str]) -> Option<&'static str> {
    let has = |wanted: &[&str]| ids.iter().any(|id| wanted.contains(id));
    if has(&["debian", "ubuntu"]) {
        Some("xserver-xorg-video-dummy")
    } else if has(&["fedora", "rhel", "centos"]) {
        Some("xorg-x11-drv-dummy")
    } else if has(&[
        "arch",
        "opensuse",
        "suse",
        "opensuse-tumbleweed",
        "opensuse-leap",
    ]) {
        Some("xf86-video-dummy")
    } else {
        None
    }
}

/// `ID` and `ID_LIKE` of an `/etc/os-release` text, lowercase.
pub fn os_release_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "ID" || key.trim() == "ID_LIKE" {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            ids.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    ids
}

/// Whether the display manager may restart now (E4): only when logind shows
/// the greeter or no session; a session of the class `user` (a desktop, also
/// a text login) keeps it running and the setup takes effect after sign-out
/// or restart. The classes `greeter`, `lock-screen`, `background` and
/// `manager` (systemd ≥ 256 per account) are no sign-in of a person.
pub fn restart_allowed(classes: &[String]) -> bool {
    !classes.iter().any(|class| class.starts_with("user"))
}

/// Whether a file's text is one se wrote (its first line is the marker).
pub fn written_by_se(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with(MARKER))
}
