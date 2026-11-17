//! Task tests of the contracts of the virtual display (plan 2026-11-17 P5):
//! the Xorg configuration of the driver setup, the compatibility of the
//! hello fields and the order of the display step in the one elevated call.
//! No test needs an X server.
use super::*;
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::virtual_display_config::{
    dummy_package, lightdm_dropin, os_release_ids, restart_allowed, sddm_dropin, written_by_se,
    xorg_conf, DisplayManager, DisplayManagerKind, GDM_XORG_PATH, LIGHTDM_DROPIN_PATH,
    SDDM_DROPIN_PATH,
};
use crate::screen::core::window_list::MonitorInfo;
use crate::screen::core::wire_hello::{ScreenClientHello, ScreenHelloOk, ScreenServerHello};

fn size(width: u32, height: u32) -> FrameSize {
    FrameSize::new(width, height)
}

/// E4: the Xorg configuration of the driver `dummy`, the drop-ins of the
/// display managers and what the elevated step decides from the system.
#[test]
fn virtual_display_task_xorg_conf_golden() {
    let expected = "# smart-explorer virtual display: written by se, removed with the last screen-control right.
Section \"Device\"
    Identifier \"se-virtual-device\"
    Driver \"dummy\"
    VideoRam 1048576
    Option \"Monitor-DUMMY0\" \"se-virtual-monitor\"
EndSection

Section \"Monitor\"
    Identifier \"se-virtual-monitor\"
    HorizSync 5.0 - 1000.0
    VertRefresh 5.0 - 200.0
    Modeline \"SE_1920x1080\" 138.50 1920 1968 2000 2080 1080 1083 1088 1111 +hsync -vsync
    Option \"PreferredMode\" \"SE_1920x1080\"
EndSection

Section \"Screen\"
    Identifier \"se-virtual-screen\"
    Device \"se-virtual-device\"
    Monitor \"se-virtual-monitor\"
    DefaultDepth 24
    SubSection \"Display\"
        Depth 24
    EndSubSection
EndSection

Section \"ServerLayout\"
    Identifier \"se-virtual-layout\"
    Screen \"se-virtual-screen\"
EndSection
";
    assert_eq!(xorg_conf(), expected);
    assert_eq!(
        lightdm_dropin(),
        "# smart-explorer virtual display\n[Seat:*]\nxserver-command=X -config smart-explorer-virtual.conf\n"
    );
    assert_eq!(
        sddm_dropin(),
        "# smart-explorer virtual display\n[X11]\nServerArguments=-nolisten tcp -config smart-explorer-virtual.conf\n"
    );
    assert!(written_by_se(&xorg_conf()) && written_by_se(&lightdm_dropin()));
    assert!(!written_by_se("Section \"Device\"\n"));
    let lightdm = DisplayManager::from_unit("lightdm.service");
    assert_eq!(lightdm.kind, DisplayManagerKind::LightDm);
    assert_eq!(
        lightdm.dropin().map(|(path, _)| path),
        Some(LIGHTDM_DROPIN_PATH)
    );
    let gdm = DisplayManager::from_unit("gdm3.service");
    assert_eq!(
        (gdm.kind, gdm.unit.as_str()),
        (DisplayManagerKind::Gdm, "gdm3.service")
    );
    assert_eq!(gdm.dropin(), Some((GDM_XORG_PATH, xorg_conf())));
    let sddm = DisplayManager::from_unit("sddm.service");
    assert_eq!(sddm.dropin().map(|(path, _)| path), Some(SDDM_DROPIN_PATH));
    assert_eq!(DisplayManager::from_unit("xdm.service").dropin(), None);
    let ids = os_release_ids("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\n");
    assert_eq!(ids, vec!["ubuntu".to_string(), "debian".to_string()]);
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(dummy_package(&ids), Some("xserver-xorg-video-dummy"));
    assert_eq!(dummy_package(&["fedora"]), Some("xorg-x11-drv-dummy"));
    assert_eq!(dummy_package(&["arch"]), Some("xf86-video-dummy"));
    assert_eq!(dummy_package(&["alpine"]), None);
    let classes = |list: &[&str]| {
        list.iter()
            .map(|class| class.to_string())
            .collect::<Vec<_>>()
    };
    assert!(restart_allowed(&classes(&[])));
    assert!(restart_allowed(&classes(&[
        "greeter",
        "manager",
        "background"
    ])));
    assert!(!restart_allowed(&classes(&["greeter", "user"])));
}

/// P5 protocol: every field is optional; a viewer and a host of v0.5.197
/// keep their behaviour, a new viewer names its wish only to a host that
/// announced it, and an unsupported answer is also a limit.
#[test]
fn virtual_display_task_hello_compat() {
    let server = ScreenServerHello::new(
        "c".repeat(32),
        "host".into(),
        "key".into(),
        "pin".into(),
        "node".into(),
    );
    assert_eq!(server.virtual_display, Some(VIRTUAL_DISPLAY_V1));
    let mut old_host = serde_json::to_value(&server).unwrap();
    old_host.as_object_mut().unwrap().remove("virtual_display");
    let old_host: ScreenServerHello = serde_json::from_value(old_host).unwrap();
    assert_eq!(old_host.virtual_display, None);
    // A hello of v0.5.197 has neither field; a new one round-trips.
    let old_viewer = serde_json::json!({"protocol_version":1,"relation_kind":"direct",
        "relation_id":"r","device_id":"d","device_name":"n","public_key":"k","fingerprint":"f",
        "node_id":"node","client_nonce":"nonce","proof":"p","mode":"view","max_fps":30});
    let mut hello: ScreenClientHello = serde_json::from_value(old_viewer).unwrap();
    assert_eq!(
        (hello.virtual_display, hello.virtual_display_size),
        (None, None)
    );
    let plain = serde_json::to_value(&hello).unwrap();
    assert!(plain.get("virtual_display").is_none() && plain.get("virtual_display_size").is_none());
    hello.virtual_display = Some(VirtualDisplayWish::Request);
    hello.virtual_display_size = Some(size(2560, 1440));
    hello.validate().unwrap();
    let json = serde_json::to_value(&hello).unwrap();
    assert_eq!(json["virtual_display"], "request");
    assert_eq!(json["virtual_display_size"]["width"], 2560);
    assert_eq!(
        serde_json::from_value::<ScreenClientHello>(json).unwrap(),
        hello
    );
    for wrong in [size(0, 1080), size(16_385, 1080)] {
        hello.virtual_display_size = Some(wrong);
        assert!(hello.validate().is_err(), "{wrong:?}");
    }
    // A wish of a newer viewer is `off`; a state of a newer host is `other`.
    assert_eq!(
        serde_json::from_value::<VirtualDisplayWish>(serde_json::json!("mirror")).unwrap(),
        VirtualDisplayWish::Off
    );
    assert_eq!(
        serde_json::from_value::<VirtualDisplayState>(serde_json::json!("cloned")).unwrap(),
        VirtualDisplayState::Other
    );
    for wish in VirtualDisplayWish::ALL {
        assert_eq!(VirtualDisplayWish::parse(wish.key()), Some(wish));
    }
    assert_eq!(VirtualDisplayWish::parse("on"), None);
    assert_eq!(parse_size("2560x1440"), Some(size(2560, 1440)));
    assert_eq!(parse_size(" 1920 × 1080 "), Some(size(1920, 1080)));
    assert_eq!(parse_size("1920X1200"), Some(size(1920, 1200)));
    for wrong in ["", "1920", "0x1080", "16385x100", "axb", "1920x1080x2"] {
        assert_eq!(parse_size(wrong), None, "{wrong}");
    }
    let choice = VirtualDisplayChoice {
        wish: VirtualDisplayWish::Request,
        size: Some(size(1920, 1200)),
    };
    assert_eq!(choice.text(), "request 1920×1200");
    assert_eq!(VirtualDisplayChoice::default().text(), "auto");
    // `HELLO_OK` of v0.5.197 has no answer; an unsupported one is a limit.
    let mut ok: ScreenHelloOk = serde_json::from_value(serde_json::json!({
        "authorization":{"view":"allow","control":"allow"},
        "platform":"windows","approval":"not_required"}))
    .unwrap();
    assert_eq!(ok.virtual_display, None);
    let pending = VirtualDisplayReport::new(VirtualDisplayState::Unsupported, None, "windows")
        .with_reason(WINDOWS_DRIVER_PENDING);
    ok.set_virtual_display(Some(pending.clone()));
    ok.set_virtual_display(Some(pending.clone()));
    assert_eq!(
        ok.limits,
        vec!["virtual_display_unsupported:windows_driver_pending".to_string()]
    );
    assert_eq!(ok.virtual_display, Some(pending));
    let active =
        VirtualDisplayReport::new(VirtualDisplayState::Active, Some(size(1920, 1080)), "randr");
    assert_eq!(active.chain_key(), "active:1920x1080:randr");
    assert_eq!(active.text(), "Virtueller Bildschirm: 1920×1080 (randr)");
    // A monitor and the statistics of an older host.
    let monitor: MonitorInfo = serde_json::from_value(serde_json::json!({
        "id":"0","x":0,"y":0,"width":1024,"height":768}))
    .unwrap();
    assert!(!monitor.is_virtual);
    assert!(serde_json::to_value(&monitor)
        .unwrap()
        .get("virtual")
        .is_none());
    let dummy = MonitorInfo {
        is_virtual: true,
        ..monitor
    };
    assert_eq!(serde_json::to_value(&dummy).unwrap()["virtual"], true);
    assert!(serde_json::to_value(ChainReport::default())
        .unwrap()
        .get("virtual_display")
        .is_none());
}

/// E4: the display step runs last in the one elevated call, after the login
/// and the RDP step; its exit codes 50 to 55 are its own.
#[test]
fn virtual_display_task_elevated_order() {
    use crate::rdp::host_setup::elevated::{plan, step_code, valid_step_name, Party, MAX_STEPS};
    let install = "virtual-display-install";
    assert!(valid_step_name(install) && valid_step_name("virtual-display-remove"));
    #[cfg(target_os = "linux")]
    {
        use crate::screen::os::virtual_display::setup::{INSTALL_STEP, REMOVE_STEP};
        assert_eq!(
            (INSTALL_STEP, REMOVE_STEP),
            (install, "virtual-display-remove")
        );
    }
    let call = plan(&[
        (Party::Display, install.into()),
        (Party::Rdp, "install-xrdp".into()),
        (Party::Login, "login-service-install".into()),
    ])
    .unwrap();
    assert_eq!(call.parties, vec![Party::Login, Party::Rdp, Party::Display]);
    assert_eq!(call.steps.last().map(String::as_str), Some(install));
    assert_eq!(call.launcher, Party::Rdp);
    assert!(call.steps.len() <= MAX_STEPS);
    // Alone, or with the login step only: the first party starts the call.
    let alone = plan(&[(Party::Display, install.into())]).unwrap();
    assert_eq!(
        (alone.parties.clone(), alone.launcher),
        (vec![Party::Display], Party::Display)
    );
    assert_eq!(step_code(&alone, Party::Display, Some(50)), Some(50));
    let pair = plan(&[
        (Party::Display, install.into()),
        (Party::Login, "login-service-install".into()),
    ])
    .unwrap();
    assert_eq!(pair.launcher, Party::Login);
    // 50 (takes effect after a restart) and the failures 51 to 55 are the
    // display step's: the steps before it succeeded.
    for code in 50..=55 {
        assert_eq!(step_code(&call, Party::Display, Some(code)), Some(code));
        assert_eq!(step_code(&call, Party::Rdp, Some(code)), Some(0));
        assert_eq!(step_code(&call, Party::Login, Some(code)), Some(0));
    }
    // A failed RDP or login step leaves the display step's code unknown.
    assert_eq!(step_code(&call, Party::Display, Some(1)), None);
    assert_eq!(step_code(&call, Party::Display, Some(44)), None);
    assert_eq!(step_code(&call, Party::Display, Some(0)), Some(0));
    // A dismissed prompt ends every step.
    assert_eq!(step_code(&call, Party::Display, Some(126)), Some(126));
}
