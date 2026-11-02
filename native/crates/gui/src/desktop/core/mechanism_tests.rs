//! DSK-M1: mechanism per session (every row of C2.2 plus Windows), background
//! policy (C2.2, A22) and the rules of `config.json` (C5.4).
use super::config::{
    apply_change, Background, BackgroundKind, ConfigChange, ConfigContext, ConfigError,
    DesktopConfig, ForeignDesktopIcons, OpenDevicesWith, SystemItems,
};
use super::mechanism::{
    background_policy, decide, icons_below, transparent_allowed, BackgroundPolicy, DisplayServer,
    GnomeVariant, LayerKeyboard, LayerNamespace, Mechanism, NetSupported, Reason, SessionFacts,
};
use super::state::Platform;
use serde_json::{json, Value};

fn windows() -> SessionFacts {
    SessionFacts {
        display: DisplayServer::Windows,
        ..SessionFacts::default()
    }
}

fn x11(desktop_type: bool, below: bool, foreign_desktop: bool) -> SessionFacts {
    SessionFacts {
        display: DisplayServer::X11,
        net_supported: Some(NetSupported {
            window_type_desktop: desktop_type,
            state_below: below,
        }),
        foreign_desktop_window: foreign_desktop,
        ..SessionFacts::default()
    }
}

fn wayland(current_desktop: &str, layer_shell: Option<u32>) -> SessionFacts {
    SessionFacts {
        display: DisplayServer::Wayland,
        current_desktop: current_desktop.split(':').map(str::to_string).collect(),
        layer_shell_version: layer_shell,
        alpha_supported: true,
        ..SessionFacts::default()
    }
}

fn gnome(major: Option<u32>, ding: bool) -> SessionFacts {
    SessionFacts {
        gnome_shell_major: major,
        foreign_desktop_icons: ding.then(|| "ding@rastersoft.com".to_string()),
        ..wayland("ubuntu:GNOME", None)
    }
}

fn layer(keyboard: LayerKeyboard, namespace: LayerNamespace) -> Mechanism {
    Mechanism::LayerShell {
        keyboard,
        namespace,
    }
}

fn unavailable(reason: Reason) -> Mechanism {
    Mechanism::Unavailable(reason)
}

#[test]
fn desktop_task_mechanism_matrix() {
    use ForeignDesktopIcons::{Keep, TakeOver};
    use LayerKeyboard::{Off, OnDemand};
    use LayerNamespace::{Desktop, SmartExplorer};
    let without_ewmh = SessionFacts {
        net_supported: None,
        ..x11(false, false, false)
    };
    let cosmic_files = SessionFacts {
        cosmic_files_desktop: true,
        ..wayland("COSMIC", Some(4))
    };
    let rows = [
        ("Windows", windows(), Keep, Mechanism::WindowsBottom),
        (
            "X11 Openbox",
            x11(true, true, false),
            Keep,
            Mechanism::X11DesktopType,
        ),
        (
            "X11 xfdesktop",
            x11(true, true, true),
            Keep,
            Mechanism::X11Below,
        ),
        (
            "X11 nur BELOW",
            x11(false, true, false),
            Keep,
            Mechanism::X11Below,
        ),
        (
            "X11 fremder Desktop ohne BELOW",
            x11(true, false, true),
            Keep,
            unavailable(Reason::ForeignDesktopWithoutBelow),
        ),
        (
            "i3",
            x11(false, false, false),
            Keep,
            unavailable(Reason::NoEwmhDesktop),
        ),
        (
            "ohne _NET_SUPPORTED",
            without_ewmh,
            Keep,
            unavailable(Reason::NoEwmhDesktop),
        ),
        (
            "KDE Plasma",
            wayland("KDE", Some(5)),
            Keep,
            layer(OnDemand, Desktop),
        ),
        (
            "Sway",
            wayland("sway", Some(4)),
            Keep,
            layer(OnDemand, SmartExplorer),
        ),
        (
            "Hyprland",
            wayland("Hyprland", Some(5)),
            Keep,
            layer(OnDemand, SmartExplorer),
        ),
        (
            "phoc",
            wayland("Phosh:GNOME", Some(3)),
            Keep,
            layer(Off, SmartExplorer),
        ),
        (
            "COSMIC",
            wayland("COSMIC", Some(4)),
            Keep,
            layer(OnDemand, SmartExplorer),
        ),
        (
            "COSMIC mit cosmic-files",
            cosmic_files,
            Keep,
            unavailable(Reason::CosmicFilesActive),
        ),
        (
            "GNOME 44",
            gnome(Some(44), false),
            Keep,
            unavailable(Reason::GnomeTooOld),
        ),
        (
            "GNOME 45",
            gnome(Some(45), false),
            Keep,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::A,
            },
        ),
        (
            "GNOME 46",
            gnome(Some(46), false),
            Keep,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::A,
            },
        ),
        (
            "GNOME 48",
            gnome(Some(48), false),
            Keep,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::A,
            },
        ),
        (
            "GNOME 49",
            gnome(Some(49), false),
            Keep,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::B,
            },
        ),
        (
            "GNOME 51",
            gnome(Some(51), false),
            Keep,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::B,
            },
        ),
        (
            "GNOME 52",
            gnome(Some(52), false),
            Keep,
            unavailable(Reason::GnomeVersionUnlisted(52)),
        ),
        (
            "GNOME ohne Version",
            gnome(None, false),
            Keep,
            unavailable(Reason::GnomeVersionUnknown),
        ),
        (
            "GNOME mit DING, behalten",
            gnome(Some(46), true),
            Keep,
            unavailable(Reason::ForeignDesktopIcons("ding@rastersoft.com".into())),
        ),
        (
            "GNOME mit DING, übernommen",
            gnome(Some(46), true),
            TakeOver,
            Mechanism::GnomeExtension {
                variant: GnomeVariant::A,
            },
        ),
        (
            "Weston",
            wayland("Weston", None),
            Keep,
            unavailable(Reason::NoDesktopLayer),
        ),
        (
            "ohne Sitzung",
            SessionFacts::default(),
            Keep,
            unavailable(Reason::NoGraphicalSession),
        ),
    ];
    for (name, facts, icons, expected) in rows {
        let mechanism = decide(&facts, icons);
        assert_eq!(mechanism, expected, "{name}");
        if let Mechanism::Unavailable(reason) = &mechanism {
            assert_eq!(mechanism.key(), "none", "{name}");
            assert!(
                !reason.code().is_empty() && !reason.text().is_empty(),
                "{name}"
            );
        }
    }
    assert_eq!(
        Reason::GnomeVersionUnlisted(52).code(),
        "unsupported_shell_version"
    );
    assert_eq!(
        [
            Mechanism::WindowsBottom,
            Mechanism::X11DesktopType,
            Mechanism::X11Below,
            layer(OnDemand, Desktop),
            Mechanism::GnomeExtension {
                variant: GnomeVariant::B
            },
        ]
        .map(|mechanism| mechanism.key()),
        [
            "windows_bottom",
            "x11_desktop_type",
            "x11_below",
            "layer_shell",
            "gnome_extension"
        ]
    );
    assert_eq!(Desktop.as_str(), "desktop");
    assert_eq!(SmartExplorer.as_str(), "smart-explorer-desktop");
}

#[test]
fn desktop_task_background_policy() {
    use BackgroundPolicy::{Opaque, Transparent};
    let kde = wayland("KDE", Some(5));
    let sway = wayland("sway", Some(4));
    let sway_without_alpha = SessionFacts {
        alpha_supported: false,
        ..wayland("sway", Some(4))
    };
    let gnome_46 = gnome(Some(46), false);
    // (session, desktop icons below, policy for `background = system`)
    let rows = [
        (windows(), true, Opaque),
        (x11(true, true, false), false, Opaque),
        (x11(true, true, true), true, Opaque),
        (kde, true, Opaque),
        (sway, false, Transparent),
        (sway_without_alpha, false, Opaque),
        (gnome_46, false, Transparent),
    ];
    for (facts, below, policy) in rows {
        let mechanism = decide(&facts, ForeignDesktopIcons::Keep);
        assert_eq!(icons_below(&mechanism, &facts), below, "{mechanism:?}");
        assert_eq!(
            background_policy(&mechanism, &facts),
            policy,
            "{mechanism:?}"
        );
        // `transparent` is refused with icons below (C5.4) or without alpha.
        let allowed = transparent_allowed(&mechanism, &facts);
        assert_eq!(
            allowed.is_ok(),
            !below && facts.alpha_supported,
            "{mechanism:?}"
        );
    }
    let refused = transparent_allowed(&Mechanism::WindowsBottom, &windows()).unwrap_err();
    assert!(refused.contains("Desktopsymbole"), "{refused}");
    let off = unavailable(Reason::NoDesktopLayer);
    assert!(transparent_allowed(&off, &wayland("Weston", None)).is_err());
    assert_eq!(background_policy(&off, &wayland("Weston", None)), Opaque);
}

#[test]
fn desktop_task_config_validation() {
    let initial = DesktopConfig::initial(SystemItems::initial(None, None));
    let written = initial.to_json().unwrap();
    let value: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        value,
        json!({
            "schema": 1,
            "background": { "kind": "system", "value": null },
            "groups": {
                "desktop_files": true, "devices": true, "rooms": true,
                "vaults": true, "screens": true
            },
            "system_items": { "recycle_bin": true, "this_pc": false },
            "open_devices_with": "app",
            // Folder tiles open with the app by default (1aea1216).
            "open_folders_with": "app",
            "foreign_desktop_icons": "keep"
        })
    );
    assert_eq!(DesktopConfig::from_json(&written), Ok(initial.clone()));
    // Ä5: Explorer's values decide once; 0 shown, 1 hidden.
    assert_eq!(
        SystemItems::initial(Some(1), Some(0)),
        SystemItems {
            recycle_bin: false,
            this_pc: true
        }
    );

    // `--background` values.
    assert_eq!(
        Background::parse_cli("color:#12ab3F"),
        Ok(Background {
            kind: BackgroundKind::Color,
            value: Some("#12ab3F".into())
        })
    );
    assert_eq!(
        Background::parse_cli("image:/home/nutzer/Bilder/berg.png")
            .map(|background| background.kind),
        Ok(BackgroundKind::Image)
    );
    assert_eq!(
        Background::parse_cli("transparent"),
        Ok(Background {
            kind: BackgroundKind::Transparent,
            value: None
        })
    );
    for bad in [
        "color:#12345",
        "color:red",
        "color:#12345G",
        "image:",
        "rot",
        "system:",
        "transparent:x",
    ] {
        assert!(
            matches!(
                Background::parse_cli(bad),
                Err(ConfigError::InvalidBackground(_))
            ),
            "{bad}"
        );
    }

    // Stored files: schema and background are checked.
    assert_eq!(
        DesktopConfig::from_json(&written.replace("\"schema\": 1", "\"schema\": 2")),
        Err(ConfigError::Schema(2))
    );
    let colour_without_value = written.replace("\"kind\": \"system\"", "\"kind\": \"color\"");
    assert!(matches!(
        DesktopConfig::from_json(&colour_without_value),
        Err(ConfigError::InvalidBackground(_))
    ));

    let sway = ConfigContext {
        platform: Platform::Linux,
        transparent: Ok(()),
        device_tree_mounted: false,
    };
    let windows_context = ConfigContext {
        platform: Platform::Windows,
        transparent: transparent_allowed(&Mechanism::WindowsBottom, &windows()),
        device_tree_mounted: true,
    };
    let transparent = ConfigChange {
        background: Some(Background::parse_cli("transparent").unwrap()),
        ..ConfigChange::default()
    };
    assert!(matches!(
        apply_change(&initial, &transparent, &windows_context),
        Err(ConfigError::TransparentRefused(_))
    ));
    let on_sway = apply_change(&initial, &transparent, &sway).unwrap();
    assert_eq!(on_sway.background.kind, BackgroundKind::Transparent);
    // Stored `transparent` in a session that cannot show it draws the system background.
    assert_eq!(
        on_sway.effective_background(&windows_context.transparent),
        Background::system()
    );
    assert_eq!(on_sway.effective_background(&Ok(())), on_sway.background);

    let file_manager = ConfigChange {
        open_devices_with: Some(OpenDevicesWith::FileManager),
        ..ConfigChange::default()
    };
    assert_eq!(
        apply_change(&initial, &file_manager, &sway),
        Err(ConfigError::DeviceTreeNotMounted)
    );
    let with_tree = apply_change(&initial, &file_manager, &windows_context).unwrap();
    assert_eq!(with_tree.open_devices_with, OpenDevicesWith::FileManager);
    assert_eq!(with_tree.effective_open_with(false), OpenDevicesWith::App);
    assert_eq!(
        with_tree.effective_open_with(true),
        OpenDevicesWith::FileManager
    );

    // Platform-bound options.
    let recycle = ConfigChange {
        recycle_bin: Some(false),
        ..ConfigChange::default()
    };
    assert_eq!(
        apply_change(&initial, &recycle, &sway),
        Err(ConfigError::WindowsOnly("--recycle-bin"))
    );
    let this_pc = ConfigChange {
        this_pc: Some(true),
        ..ConfigChange::default()
    };
    assert_eq!(
        apply_change(&initial, &this_pc, &sway),
        Err(ConfigError::WindowsOnly("--this-pc"))
    );
    let take_over = ConfigChange {
        foreign_desktop_icons: Some(ForeignDesktopIcons::TakeOver),
        ..ConfigChange::default()
    };
    assert_eq!(
        apply_change(&initial, &take_over, &windows_context),
        Err(ConfigError::LinuxOnly("--foreign-desktop-icons"))
    );
    assert_eq!(
        apply_change(&initial, &take_over, &sway)
            .unwrap()
            .foreign_desktop_icons,
        ForeignDesktopIcons::TakeOver
    );

    // Groups and system items change only where given.
    let groups = ConfigChange {
        vaults: Some(false),
        screens: Some(false),
        this_pc: Some(true),
        ..ConfigChange::default()
    };
    let changed = apply_change(&initial, &groups, &windows_context).unwrap();
    assert!(!changed.groups.vaults && !changed.groups.screens);
    assert!(changed.groups.desktop_files && changed.groups.devices && changed.groups.rooms);
    assert!(changed.system_items.this_pc && changed.system_items.recycle_bin);
    assert_eq!(
        apply_change(&initial, &ConfigChange::default(), &sway),
        Ok(initial)
    );
}
