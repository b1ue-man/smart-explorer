//! Signals `desktop_task_linux_session_facts_table` (every row of C2.2: the
//! facts a stand-in session gives, and the mechanism they lead to) and
//! `desktop_task_linux_gnome_version_parse`. No display is needed.
use std::collections::HashMap;

use super::*;
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::mechanism::{
    self, GnomeVariant, LayerKeyboard, LayerNamespace, Mechanism, Reason,
};

#[derive(Default)]
struct Fake {
    env: HashMap<&'static str, &'static str>,
    x11: Option<X11Facts>,
    layer: Option<u32>,
    gnome: Option<&'static str>,
    active: Vec<&'static str>,
    cosmic_files: bool,
}

impl Probe for Fake {
    fn env(&self, name: &str) -> Option<String> {
        self.env.get(name).map(|value| value.to_string())
    }
    fn x11_root_atoms(&self) -> Option<X11Facts> {
        self.x11.clone()
    }
    fn wayland_layer_shell_version(&self) -> Option<u32> {
        self.layer
    }
    fn gnome_shell_version(&self) -> Option<String> {
        self.gnome.map(str::to_string)
    }
    fn gnome_active_extensions(&self) -> Vec<String> {
        self.active.iter().map(|uuid| uuid.to_string()).collect()
    }
    fn process_running(&self, comm: &str) -> bool {
        comm == "cosmic-files-applet" && self.cosmic_files
    }
}

fn x11(supported: &[&str], foreign: bool, compositor: bool) -> Option<X11Facts> {
    Some(X11Facts {
        supported: supported.iter().map(|name| name.to_string()).collect(),
        stacking_has_foreign_desktop: foreign,
        compositor,
    })
}

fn session(display: &[(&'static str, &'static str)], desktop: &'static str) -> Fake {
    let mut fake = Fake::default();
    for &(name, value) in display {
        fake.env.insert(name, value);
    }
    fake.env.insert("XDG_CURRENT_DESKTOP", desktop);
    fake
}

fn decide(fake: &Fake, icons: ForeignDesktopIcons) -> Mechanism {
    mechanism::decide(&probe(fake), icons)
}

const TYPE_DESKTOP: &str = "_NET_WM_WINDOW_TYPE_DESKTOP";
const BELOW: &str = "_NET_WM_STATE_BELOW";

#[test]
fn desktop_task_linux_session_facts_table() {
    let keep = ForeignDesktopIcons::Keep;
    let x = [("DISPLAY", ":0")];
    let wl = [("WAYLAND_DISPLAY", "wayland-0")];

    // X11 with the window type and no foreign desktop window: L1.
    let mut fake = session(&x, "XFCE");
    fake.x11 = x11(&[TYPE_DESKTOP, BELOW, "_NET_WORKAREA"], false, true);
    let facts = probe(&fake);
    assert_eq!(facts.display, DisplayServer::X11);
    assert!(facts.alpha_supported, "a compositor runs");
    assert_eq!(mechanism::decide(&facts, keep), Mechanism::X11DesktopType);

    // A foreign desktop window and `_NET_WM_STATE_BELOW`: L2.
    fake.x11 = x11(&[TYPE_DESKTOP, BELOW], true, false);
    assert_eq!(decide(&fake, keep), Mechanism::X11Below);

    // Only the window type next to a foreign desktop window: no order.
    fake.x11 = x11(&[TYPE_DESKTOP], true, false);
    assert_eq!(
        decide(&fake, keep),
        Mechanism::Unavailable(Reason::ForeignDesktopWithoutBelow)
    );

    // Only `_NET_WM_STATE_BELOW`, nothing foreign: L2 as well.
    fake.x11 = x11(&[BELOW], false, false);
    assert_eq!(decide(&fake, keep), Mechanism::X11Below);

    // No EWMH desktop atoms (i3), an empty list, no list, no connection.
    for supported in [
        x11(&["_NET_WM_NAME"], false, false),
        x11(&[], false, false),
        None,
    ] {
        fake.x11 = supported;
        assert_eq!(
            decide(&fake, keep),
            Mechanism::Unavailable(Reason::NoEwmhDesktop)
        );
    }
    assert_eq!(probe(&fake).net_supported, None);

    // Wayland with the layer protocol: version 4 and later with a keyboard.
    let mut fake = session(&wl, "sway");
    fake.layer = Some(4);
    let facts = probe(&fake);
    assert_eq!(facts.display, DisplayServer::Wayland);
    assert!(facts.alpha_supported);
    assert_eq!(
        mechanism::decide(&facts, keep),
        Mechanism::LayerShell {
            keyboard: LayerKeyboard::OnDemand,
            namespace: LayerNamespace::SmartExplorer,
        }
    );
    fake.layer = Some(3);
    assert_eq!(
        decide(&fake, keep),
        Mechanism::LayerShell {
            keyboard: LayerKeyboard::Off,
            namespace: LayerNamespace::SmartExplorer,
        }
    );

    // KDE: the namespace `desktop`.
    let mut kde = session(&wl, "KDE");
    kde.layer = Some(5);
    assert_eq!(
        decide(&kde, keep),
        Mechanism::LayerShell {
            keyboard: LayerKeyboard::OnDemand,
            namespace: LayerNamespace::Desktop,
        }
    );

    // COSMIC with its own desktop icons: unavailable; without them the layer.
    let mut cosmic = session(&wl, "COSMIC");
    cosmic.layer = Some(4);
    cosmic.cosmic_files = true;
    assert!(probe(&cosmic).cosmic_files_desktop);
    assert_eq!(
        decide(&cosmic, keep),
        Mechanism::Unavailable(Reason::CosmicFilesActive)
    );
    cosmic.cosmic_files = false;
    assert!(matches!(
        decide(&cosmic, keep),
        Mechanism::LayerShell { .. }
    ));
    // The process is only looked for on COSMIC.
    let mut other = session(&wl, "sway");
    other.layer = Some(4);
    other.cosmic_files = true;
    assert!(!probe(&other).cosmic_files_desktop);

    // GNOME without the layer protocol: the extension, by the Shell version.
    let mut gnome = session(&wl, "ubuntu:GNOME");
    gnome.gnome = Some("46.0");
    assert_eq!(probe(&gnome).gnome_shell_major, Some(46));
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::GnomeExtension {
            variant: GnomeVariant::A
        }
    );
    gnome.gnome = Some("49.alpha.1");
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::GnomeExtension {
            variant: GnomeVariant::B
        }
    );
    gnome.gnome = Some("44.9");
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::Unavailable(Reason::GnomeTooOld)
    );
    gnome.gnome = Some("52.0");
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::Unavailable(Reason::GnomeVersionUnlisted(52))
    );
    gnome.gnome = None;
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::Unavailable(Reason::GnomeVersionUnknown)
    );

    // An active desktop-icons extension: unavailable until the take-over.
    gnome.gnome = Some("46.0");
    gnome.active = vec!["other@example.org", "gtk4-ding@smedius.gitlab.com"];
    assert_eq!(
        probe(&gnome).foreign_desktop_icons.as_deref(),
        Some("gtk4-ding@smedius.gitlab.com")
    );
    assert_eq!(
        decide(&gnome, keep),
        Mechanism::Unavailable(Reason::ForeignDesktopIcons(
            "gtk4-ding@smedius.gitlab.com".into()
        ))
    );
    assert_eq!(
        decide(&gnome, ForeignDesktopIcons::TakeOver),
        Mechanism::GnomeExtension {
            variant: GnomeVariant::A
        }
    );

    // Neither the layer protocol nor GNOME (Weston, Cage).
    let weston = session(&wl, "weston");
    assert_eq!(
        decide(&weston, keep),
        Mechanism::Unavailable(Reason::NoDesktopLayer)
    );

    // Neither display variable; XWayland's `DISPLAY` beside a Wayland one.
    let headless = session(&[], "GNOME");
    assert_eq!(
        decide(&headless, keep),
        Mechanism::Unavailable(Reason::NoGraphicalSession)
    );
    assert_eq!(
        probe(&headless).gnome_shell_major,
        None,
        "no probe headless"
    );
    let xwayland = session(
        &[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")],
        "sway",
    );
    assert_eq!(probe(&xwayland).display, DisplayServer::Wayland);
}

#[test]
fn desktop_task_linux_gnome_version_parse() {
    assert_eq!(parse_shell_major("46.0"), Some(46));
    assert_eq!(parse_shell_major("49.alpha.1"), Some(49));
    assert_eq!(parse_shell_major("45"), Some(45));
    assert_eq!(parse_shell_major(" 47.beta "), Some(47));
    assert_eq!(parse_shell_major(""), None);
    assert_eq!(parse_shell_major("   "), None);
    assert_eq!(parse_shell_major("alpha.1"), None);
    assert_eq!(parse_shell_major("-3.0"), None);
    assert_eq!(parse_shell_major("x46"), None);
}
