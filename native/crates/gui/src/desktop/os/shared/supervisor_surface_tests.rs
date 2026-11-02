//! DSK-M7 (U37-3): which surfaces the shell starts for a mechanism, and what
//! it sends a layer process. The surface program is a stand-in script that
//! records its arguments and everything the shell writes to its standard
//! input.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::mechanism::{GnomeVariant, LayerKeyboard, LayerNamespace, Reason};
use crate::desktop::core::protocol::decode_frames;
use crate::desktop::core::tiles::{project, ShareState, TileInput};
use crate::share::ShareProfiles;

fn layer() -> Mechanism {
    Mechanism::LayerShell {
        keyboard: LayerKeyboard::OnDemand,
        namespace: LayerNamespace::SmartExplorer,
    }
}

fn monitor(key: &str, primary: bool) -> Monitor {
    Monitor {
        key: key.into(),
        primary,
        rect: Some([0, 0, 1920, 1080]),
    }
}

fn outputs() -> Vec<Monitor> {
    vec![monitor("DP-1", true), monitor("HDMI-A-1", false)]
}

#[test]
fn layer_wanted_surfaces_by_mechanism() {
    let monitors = outputs();
    // One layer process for the whole session, whatever the outputs are.
    assert_eq!(
        wanted_surfaces(&layer(), true, &monitors, None),
        vec![LAYER_KEY.to_string()]
    );
    assert_eq!(
        wanted_surfaces(&layer(), true, &[], None),
        vec![LAYER_KEY.to_string()]
    );
    // One process per monitor on Windows and X11 (unchanged).
    for mechanism in [
        Mechanism::WindowsBottom,
        Mechanism::X11DesktopType,
        Mechanism::X11Below,
    ] {
        assert_eq!(
            wanted_surfaces(&mechanism, true, &monitors, None),
            vec!["DP-1".to_string(), "HDMI-A-1".to_string()]
        );
    }
    // The GNOME extension starts its own surfaces; nothing is wanted here.
    let gnome = Mechanism::GnomeExtension {
        variant: GnomeVariant::A,
    };
    assert!(wanted_surfaces(&gnome, true, &monitors, None).is_empty());
    let none = Mechanism::Unavailable(Reason::NoDesktopLayer);
    assert!(wanted_surfaces(&none, true, &monitors, None).is_empty());
    // Part off: nothing. The suite's monitor keys win over the session.
    assert!(wanted_surfaces(&layer(), false, &monitors, None).is_empty());
    let keys = vec!["task-a".to_string()];
    assert_eq!(
        wanted_surfaces(&layer(), true, &monitors, Some(&keys)),
        keys
    );
}

fn fake_gui(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("fake-gui.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$SE_LAYER_TEST_DIR/args\"\ncat > \"$SE_LAYER_TEST_DIR/stdin\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn sent_frames(dir: &Path, count: usize) -> Vec<ShellMessage> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let bytes = std::fs::read(dir.join("stdin")).unwrap_or_default();
        let (messages, rest) = decode_frames::<ShellMessage>(&bytes).unwrap_or_default();
        if rest == 0 && messages.len() >= count {
            return messages;
        }
        assert!(
            Instant::now() < deadline,
            "{count} Rahmen nicht angekommen ({} gelesen)",
            messages.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn feed() -> (crate::desktop::core::tiles::TileModel, DesktopConfig) {
    let profiles = ShareProfiles::default();
    let (paths, state) = (BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&profiles),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    (
        tiles,
        DesktopConfig::initial(SystemItems::initial(None, None)),
    )
}

#[test]
fn desktop_task_layer_supervisor_starts_one_layer_process() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("SE_LAYER_TEST_DIR", dir.path());
    let script = fake_gui(dir.path());
    let (events, _unused) = mpsc::channel();
    let mut spawn = SurfaceSpawn::new(script, events);
    let mut surface = spawn.start_layer().unwrap();
    assert_eq!(surface.monitor, LAYER_KEY);
    surface.greeted = true;

    let (tiles, config) = feed();
    let mut layout = DesktopLayout::default();
    let monitors = outputs();
    // Two outputs: one model each, with that output's key and arrangement.
    send_models(
        std::slice::from_mut(&mut surface),
        &monitors,
        5,
        (&tiles, &[], &config),
        &mut layout,
    );
    assert_eq!(surface.sent_rev, Some(5));
    // Nothing is sent twice for one revision, and nothing without outputs.
    send_models(
        std::slice::from_mut(&mut surface),
        &monitors,
        5,
        (&tiles, &[], &config),
        &mut layout,
    );
    send_models(
        std::slice::from_mut(&mut surface),
        &[],
        6,
        (&tiles, &[], &config),
        &mut layout,
    );
    assert_eq!(surface.sent_rev, Some(5));
    drop(surface);

    let frames = sent_frames(dir.path(), 3);
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        vec!["--desktop-surface", "--layer"],
        "one --layer process, never --monitor"
    );
    assert!(matches!(
        &frames[0],
        ShellMessage::Hello { monitor, .. } if monitor == LAYER_KEY
    ));
    let keyed: Vec<(u64, Option<String>, String)> = frames[1..]
        .iter()
        .filter_map(|frame| match frame {
            ShellMessage::Model {
                rev,
                monitor,
                model,
            } => Some((*rev, monitor.clone(), model.layout.monitor.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        keyed,
        vec![
            (5, Some("DP-1".into()), "DP-1".into()),
            (5, Some("HDMI-A-1".into()), "HDMI-A-1".into())
        ]
    );
    assert_eq!(frames.len(), 3, "no further message");
}

#[test]
fn layer_models_of_a_monitor_surface_carry_no_output_key() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("SE_LAYER_TEST_DIR", dir.path());
    let script = fake_gui(dir.path());
    let (events, _unused) = mpsc::channel();
    let mut spawn = SurfaceSpawn::new(script, events);
    let mut surface = spawn.start("DP-1").unwrap();
    surface.greeted = true;
    let (tiles, config) = feed();
    let mut layout = DesktopLayout::default();
    send_models(
        std::slice::from_mut(&mut surface),
        &outputs(),
        2,
        (&tiles, &[], &config),
        &mut layout,
    );
    assert_eq!(surface.sent_rev, Some(2));
    drop(surface);
    let frames = sent_frames(dir.path(), 2);
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        vec!["--desktop-surface", "--monitor", "DP-1"]
    );
    assert!(matches!(
        &frames[1],
        ShellMessage::Model {
            rev: 2,
            monitor: None,
            ..
        }
    ));
    assert_eq!(frames.len(), 2);
}
