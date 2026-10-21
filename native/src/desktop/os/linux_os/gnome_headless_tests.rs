//! DSK-M8 signals `desktop_task_gnome_headless_variant_a` (Ubuntu 24.04,
//! GNOME Shell 46; assumptions A17, A19, A21) and
//! `desktop_task_gnome_headless_variant_b` (container `fedora:43`, GNOME Shell
//! 49; A18, A19) with a real GNOME Shell without a screen:
//! `gnome-shell --headless --wayland --virtual-monitor 1920x1080` on a session
//! bus of the check's own (`dbus-daemon`, not the user's) and a key-file
//! `gsettings` backend, all in private folders. The extension is installed
//! before the Shell starts (what the first activation does), so the Shell
//! loads it at its start (A21); the check then plays the se Desktop shell
//! (control file, token, socket), the extension starts a surface as a
//! `Meta.WaylandClient`, and the check asks for the surface to connect, for
//! its window to be placed (`handled` in the extension's report) and for the
//! state `running`. Where no headless Shell starts (stage `desktop-gnome`,
//! `SE_DESKTOP_TASK_GNOME=1`), the check ends as "nicht ausgeführt" with the
//! reason.
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::gnome_channel::{GnomeListener, ListenerEvent};
use super::gnome_control::{Control, ControlMonitor, RuntimeFiles, PROTOCOL};
use super::gnome_dbus::{display_monitors, NoShellBus, SessionBus, ShellBus, STATE_ENABLED};
use super::gnome_extension::{current_state, extension_dir, files_current, install};
use super::gnome_settings::GnomeSettings;
use super::integration::LinuxPaths;
use super::session_facts::{probe, SystemProbe};
use super::x11_task_env::{not_run, tool, wait_for};
use super::x11_task_surface::{config_with, model_for};
use crate::desktop::core::ledger::GNOME_EXTENSION_UUID;
use crate::desktop::core::mechanism::GnomeVariant;
use crate::desktop::core::protocol::{write_frame, ShellMessage, PROTOCOL_VERSION};
use crate::desktop::os::store::unix_now;

/// Sets variables of this process and puts them back.
struct Env {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl Env {
    fn set(&mut self, name: &'static str, value: Option<OsString>) {
        self.saved.push((name, std::env::var_os(name)));
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (name, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn shell_major() -> u32 {
    let output = Command::new(tool("gnome-shell"))
        .arg("--version")
        .output()
        .unwrap_or_else(|error| not_run(&format!("gnome-shell --version: {error}")));
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find_map(|word| word.split('.').next()?.parse().ok())
        .unwrap_or_else(|| not_run("gnome-shell nennt seine Version nicht"))
}

/// The lines of the Shell's log that name extensions or errors (the tail of
/// the log holds the X server's key map warnings after the start).
fn extension_lines(log: &std::path::Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            line.contains("extension") || line.contains("js error") || line.contains("critical")
        })
        .take(20)
        .collect();
    lines.join(" | ")
}

fn headless(variant: GnomeVariant, majors: std::ops::RangeInclusive<u32>) {
    if std::env::var("SE_DESKTOP_TASK_GNOME").as_deref() != Ok("1") {
        not_run("SE_DESKTOP_TASK_GNOME=1 fehlt (Stufe desktop-gnome)");
    }
    let major = shell_major();
    if !majors.contains(&major) {
        not_run(&format!(
            "GNOME Shell {major} gehört nicht zu dieser Variante ({majors:?})"
        ));
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("run");
    std::fs::create_dir(&runtime).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut env = Env { saved: Vec::new() };
    env.set("XDG_RUNTIME_DIR", Some(runtime.clone().into()));
    env.set("XDG_DATA_HOME", Some(root.path().join("data").into()));
    env.set("XDG_CONFIG_HOME", Some(root.path().join("config").into()));
    env.set("XDG_CURRENT_DESKTOP", Some("GNOME".into()));
    env.set("XDG_SESSION_TYPE", Some("wayland".into()));
    env.set("GSETTINGS_BACKEND", Some("keyfile".into()));
    env.set("DISPLAY", None);
    env.set("WAYLAND_DISPLAY", None);

    // The session bus of this check.
    let mut daemon = Command::new(tool("dbus-daemon"))
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| not_run(&format!("dbus-daemon: {error}")));
    let mut address = String::new();
    if let Some(stdout) = daemon.stdout.take() {
        let _ = BufReader::new(stdout).read_line(&mut address);
    }
    let _daemon = Kill(daemon);
    if address.trim().is_empty() {
        not_run("dbus-daemon meldet keine Adresse");
    }
    env.set("DBUS_SESSION_BUS_ADDRESS", Some(address.trim().into()));

    // The extension and its entry in `enabled-extensions` before the Shell
    // starts; the bus has no Shell yet, so the answer is "after next login".
    let paths = LinuxPaths::from_env();
    let report = install(&paths, &GnomeSettings::system(), &NoShellBus, variant)
        .unwrap_or_else(|error| not_run(&format!("Installation: {error}")));
    assert!(files_current(&extension_dir(&paths), variant));
    assert!(report.entries.len() >= 2, "{report:?}");

    let log = root.path().join("gnome-shell.log");
    let log_file = std::fs::File::create(&log).unwrap();
    let shell = Command::new(tool("gnome-shell"))
        .args(["--headless", "--wayland", "--virtual-monitor", "1920x1080"])
        .stdout(log_file.try_clone().unwrap())
        .stderr(log_file)
        .spawn()
        .unwrap_or_else(|error| not_run(&format!("gnome-shell startet nicht: {error}")));
    let mut shell = Kill(shell);
    let tail = || {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        text.lines().rev().take(15).collect::<Vec<_>>().join(" | ")
    };
    let up = wait_for(Duration::from_secs(120), || {
        shell.0.try_wait().ok().flatten().is_none() && SessionBus.shell_version().is_some()
    });
    if !up {
        not_run(&format!(
            "die kopflose GNOME Shell kommt nicht hoch: {}",
            tail()
        ));
    }

    // A21: the Shell that started after the installation has it enabled.
    let enabled = wait_for(Duration::from_secs(60), || {
        SessionBus.list_extensions().is_some_and(|list| {
            list.iter()
                .any(|(uuid, state)| uuid == GNOME_EXTENSION_UUID && *state == STATE_ENABLED)
        })
    });
    let answer = SessionBus.list_extensions();
    let listed = answer.clone().unwrap_or_default();
    assert!(
        enabled,
        "die Erweiterung ist nach dem Start der Shell nicht aktiv: {listed:?} \
         (Antwort der Shell: {}); Shell-Meldungen zu Erweiterungen: {}; {}",
        if answer.is_some() { "ja" } else { "keine" },
        extension_lines(&log),
        tail()
    );
    // The names of the desktop-icons extensions (A20), where the package is
    // installed.
    for (uuid, _) in &listed {
        if uuid.contains("ding@") {
            assert_eq!(uuid, "ding@rastersoft.com", "A20");
        }
    }

    // The check plays the se Desktop shell.
    let socket = std::fs::read_dir(&runtime)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .find(|name| name.to_string_lossy().starts_with("wayland-"));
    env.set("WAYLAND_DISPLAY", socket);
    let monitors = display_monitors().expect("Mutter nennt Monitore");
    assert!(!monitors.is_empty());
    let files = RuntimeFiles::from_env().expect("XDG_RUNTIME_DIR");
    files.new_token().unwrap();
    let listener = GnomeListener::bind(&files).unwrap();
    let gui = super::x11_task_env::gui_binary();
    let control = Control {
        protocol: PROTOCOL,
        exe: gui.to_string_lossy().into_owned(),
        generation: u64::try_from(unix_now()).unwrap_or(1),
        socket: listener.socket().to_string_lossy().into_owned(),
        monitors: monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| ControlMonitor {
                index: index as u32,
                key: monitor.key.clone(),
                x: monitor.rect[0],
                y: monitor.rect[1],
                width: monitor.rect[2] - monitor.rect[0],
                height: monitor.rect[3] - monitor.rect[1],
            })
            .collect(),
    };
    files.write_control(&control).unwrap();

    // The extension starts the surface, which connects with the token.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut accepted = None;
    while accepted.is_none() && Instant::now() < deadline {
        for event in listener.drain() {
            match event {
                ListenerEvent::Accepted(found) => accepted = Some(found),
                ListenerEvent::Rejected(why) => panic!("Fläche abgewiesen: {why}"),
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let Some(mut surface) = accepted else {
        let report = files.read_extension();
        not_run(&format!(
            "die Fläche verbindet sich nicht (kein Grafikadapter?): Bericht {report:?}; {}",
            tail()
        ));
    };
    assert_eq!(surface.index, 0);
    let key = monitors[0].key.clone();
    for message in [
        ShellMessage::Hello {
            protocol: PROTOCOL_VERSION,
            monitor: key.clone(),
        },
        config_with([40, 60, 90]),
        model_for(&key, &["a.txt"]),
    ] {
        write_frame(&mut surface.stream, &message).unwrap();
    }

    // The extension placed its window (`handled`) and says so.
    let placed = wait_for(Duration::from_secs(60), || {
        files
            .read_extension()
            .is_some_and(|report| report.handles_a_window())
    });
    let report = files.read_extension();
    assert!(
        placed,
        "kein behandeltes Fenster: {report:?}; Shell-Meldungen zu Erweiterungen: {}; {}",
        extension_lines(&log),
        tail()
    );
    let report = report.unwrap();
    assert_eq!(
        report.variant,
        if variant == GnomeVariant::A { "a" } else { "b" }
    );
    assert!(report.halted.is_none());
    assert_eq!(
        current_state(&probe(&SystemProbe)),
        Some("running"),
        "{}",
        tail()
    );

    // The shell ends: the control file goes and the extension ends the surface.
    files.remove_shell_files();
    let gone = wait_for(Duration::from_secs(30), || {
        files
            .read_extension()
            .is_none_or(|report| report.surfaces.is_empty())
    });
    assert!(gone, "die Erweiterung beendet die Flächen nicht");
    shell.0.kill().ok();
}

#[test]
#[ignore = "braucht eine kopflose GNOME Shell 45 bis 48 (Stufe desktop-gnome, Ubuntu 24.04)"]
fn desktop_task_gnome_headless_variant_a() {
    headless(GnomeVariant::A, 45..=48);
}

#[test]
#[ignore = "braucht eine kopflose GNOME Shell 49 bis 51 (Stufe desktop-gnome, Container fedora:43)"]
fn desktop_task_gnome_headless_variant_b() {
    headless(GnomeVariant::B, 49..=51);
}
