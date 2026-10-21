//! DSK-M7 checks with a compositor and static checks of the build (suite
//! stage `linux-wayland: layer`, one test thread, `--include-ignored`,
//! `SE_DESKTOP_TASK_WAYLAND=1`; environment in `layer_task_env.rs`).
//! wgpu runs on Vulkan/lavapipe (`mesa-vulkan-drivers`); where the compositor
//! offers no presentation to it, the surface checks report "nicht ausgeführt"
//! with the reason the process gave (assumption A24).
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use super::layer_task_env::{
    config_with, gui_binary, hello, model_for, not_run, require_task, tool, wait_for,
    SurfaceProcess, Sway, MAGENTA,
};
use super::render::TEST_NO_ADAPTER;
use crate::desktop::core::protocol::SurfaceMessage;

const LONG: Duration = Duration::from_secs(30);

/// The headless Sway has no input device, so its seat offers no keyboard
/// (`seat_update_capabilities`) until `wtype` adds its virtual one, and the
/// layer process binds `wl_keyboard` only when that capability appears. A key
/// that `wtype` sends right after creating the device would arrive before the
/// process has its keyboard; `-s` keeps the device without a key for a second
/// first (`wtype` creates the device, then runs its arguments in order).
const KEY_DELAY: [&str; 2] = ["-s", "1000"];

/// The layer process on `sway`, greeted, with the test colour and one model
/// for each output; ends the check as "not run" when it cannot draw at all.
fn start_surface(sway: &Sway, extra: &[(&str, String)], files: &[&str]) -> SurfaceProcess {
    let mut surface = SurfaceProcess::start(&sway.env(), &["--desktop-surface", "--layer"], extra);
    let greeting = surface.wait_message(
        LONG,
        |message| matches!(message, SurfaceMessage::Hello { monitor, .. } if monitor == "layer"),
    );
    assert!(greeting.is_some(), "keine Begrüßung der Fläche");
    surface.send(&hello());
    surface.send(&config_with(MAGENTA));
    for output in sway.outputs() {
        surface.send(&model_for(&output, files));
    }
    surface
}

/// Waits until `output` shows the colour; the process ending on its own
/// (no adapter) is "not run", anything else a failure.
fn expect_color(sway: &Sway, surface: &mut SurfaceProcess, output: &str) {
    let seen = wait_for(LONG, || {
        if !surface.running() {
            return true;
        }
        sway.screenshot(output)
            .is_some_and(|shot| shot.contains(MAGENTA))
    });
    if !surface.running() {
        let status = surface.wait_exit(Duration::from_secs(1));
        let stderr = surface.stderr_text();
        if stderr.contains("keine Grafikbeschleunigung") {
            not_run(&format!("kein Adapter: {}", stderr.trim()));
        }
        panic!("Fläche beendet ({status:?}): {stderr}");
    }
    assert!(
        seen,
        "{output}: die Testfarbe erscheint nicht im Bildschirmfoto"
    );
}

/// The windows of `swaymsg -t get_tree` as `app_id/name` pairs: what a missing
/// test window looks like (another `app_id`, an empty tree).
fn tree_windows(sway: &Sway) -> String {
    fn walk(node: &serde_json::Value, found: &mut Vec<String>) {
        let name = node["name"].as_str().unwrap_or_default();
        if node["pid"].as_i64().is_some() || node["app_id"].is_string() {
            found.push(format!("{:?}/{name:?}", node["app_id"].as_str()));
        }
        for key in ["nodes", "floating_nodes"] {
            for child in node[key].as_array().into_iter().flatten() {
                walk(child, found);
            }
        }
    }
    let tree: serde_json::Value =
        serde_json::from_str(&sway.msg(&["-t", "get_tree", "-r"])).unwrap_or_default();
    let mut found = Vec::new();
    walk(&tree, &mut found);
    format!("[{}]", found.join(", "))
}

#[test]
#[ignore = "needs sway, grim and a GPU adapter (suite stage linux-wayland: layer)"]
fn desktop_task_layer_shell_bottom_surface() {
    require_task();
    let sway = Sway::start();
    let mut surface = start_surface(&sway, &[], &[]);
    let output = sway.outputs().into_iter().next().unwrap();
    expect_color(&sway, &mut surface, &output);

    // An ordinary window (the GUI program's windowed surface) lies above.
    // The headless Sway has no GPU: Mesa's OpenGL (the window's renderer)
    // must draw in software, as lavapipe does for the layer process.
    let mut window = SurfaceProcess::start(
        &sway.env(),
        &["--desktop-surface", "--monitor", "task-window"],
        &[
            ("SE_DESKTOP_TEST_WINDOWED", "1".to_string()),
            ("LIBGL_ALWAYS_SOFTWARE", "1".to_string()),
        ],
    );
    let listed = |sway: &Sway| {
        sway.msg(&["-t", "get_tree", "-r"])
            .contains("app.smartexplorer.desktop")
    };
    let found = wait_for(LONG, || !window.running() || listed(&sway));
    if !found || !listed(&sway) {
        // Ended or silent: what the process says is the reason.
        let status = window.stop();
        panic!(
            "das Testfenster fehlt in `swaymsg -t get_tree` (Prozess {status:?}): {}; \
             Fenster im Baum (app_id/name): {}",
            window.stderr_text().trim(),
            tree_windows(&sway)
        );
    }
    let covered = wait_for(LONG, || {
        sway.screenshot(&output).is_some_and(|shot| {
            let (x, y) = (shot.width / 2, shot.height / 2);
            shot.pixel(x, y) != MAGENTA && shot.contains(MAGENTA)
        })
    });
    assert!(
        covered,
        "das Testfenster liegt nicht über der Fläche (Mitte noch in der Testfarbe oder Fläche weg)"
    );
}

#[test]
#[ignore = "needs sway, grim and a GPU adapter (suite stage linux-wayland: layer)"]
fn desktop_task_layer_shell_two_outputs() {
    require_task();
    let sway = Sway::start();
    sway.msg(&["create_output"]);
    assert!(
        wait_for(Duration::from_secs(10), || sway.outputs().len() == 2),
        "swaymsg create_output: {:?}",
        sway.outputs()
    );
    let mut surface = start_surface(&sway, &[], &[]);
    for output in sway.outputs() {
        expect_color(&sway, &mut surface, &output);
    }
    // One process serves both outputs.
    assert!(surface.running());
}

#[test]
#[ignore = "needs sway, wtype and a GPU adapter (suite stage linux-wayland: layer)"]
fn desktop_task_layer_keyboard_on_demand() {
    require_task();
    let wtype = tool("wtype");
    let sway = Sway::start();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("keys.log");
    let mut surface = start_surface(
        &sway,
        &[("SE_DESKTOP_TEST_KEYLOG", log.to_string_lossy().into_owned())],
        &[],
    );
    let output = sway.outputs().into_iter().next().unwrap();
    expect_color(&sway, &mut surface, &output);
    // The surface takes the keyboard when it is clicked (`on_demand`), then
    // the key reaches it; its test log names the keysym (0x61 is `a`).
    let typed = wait_for(LONG, || {
        sway.click(300, 300);
        let _ = sway.command(&wtype).args(KEY_DELAY).arg("a").status();
        std::fs::read_to_string(&log).is_ok_and(|text| text.contains("press 0x61"))
    });
    assert!(typed, "der Tastendruck erreicht die Fläche nicht: {log:?}");
}

#[test]
#[ignore = "needs sway, wl-clipboard, wtype and a GPU adapter (suite stage linux-wayland: layer)"]
fn desktop_task_layer_clipboard_roundtrip_files() {
    require_task();
    let (wl_copy, wl_paste, wtype) = (tool("wl-copy"), tool("wl-paste"), tool("wtype"));
    let sway = Sway::start();
    let work = tempfile::tempdir().unwrap();
    let desktop = work.path().join("Desktop");
    let source = work.path().join("Quelle");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(desktop.join("Ziel.md"), "ziel").unwrap();
    std::fs::write(source.join("Kopie.txt"), "kopie").unwrap();
    std::fs::write(source.join("Zwei Wörter.txt"), "zwei").unwrap();
    let mut surface = start_surface(
        &sway,
        &[(
            "SE_DESKTOP_TEST_DESKTOP_DIR",
            desktop.to_string_lossy().into_owned(),
        )],
        &["Ziel.md"],
    );
    let output = sway.outputs().into_iter().next().unwrap();
    expect_color(&sway, &mut surface, &output);
    // A click gives the surface the keyboard; Ctrl+V pastes.
    let paste = |sway: &Sway| {
        sway.click(600, 600);
        let _ = sway
            .command(&wtype)
            .args(KEY_DELAY)
            .args(["-M", "ctrl", "v", "-m", "ctrl"])
            .status();
    };
    // `wl-copy` hands the selection to the compositor and returns.
    let offer = |mime: &str, body: String| {
        use std::io::Write;
        let mut child = sway
            .command(&wl_copy)
            .args(["--type", mime])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let _ = child.wait();
    };

    // Another program copies a file as `text/uri-list`: the surface pastes it
    // into the desktop folder.
    let first = source.join("Kopie.txt");
    offer(
        "text/uri-list",
        format!("{}\r\n", super::clipboard_uri::file_uri(&first)),
    );
    paste(&sway);
    assert!(
        wait_for(LONG, || desktop.join("Kopie.txt").is_file()),
        "text/uri-list: Datei nicht eingefügt"
    );
    assert!(first.is_file(), "ein Kopieren lässt die Quelle stehen");

    // `x-special/gnome-copied-files` with `cut` moves it.
    let second = source.join("Zwei Wörter.txt");
    offer(
        "x-special/gnome-copied-files",
        format!("cut\n{}", super::clipboard_uri::file_uri(&second)),
    );
    paste(&sway);
    assert!(
        wait_for(LONG, || desktop.join("Zwei Wörter.txt").is_file()),
        "x-special/gnome-copied-files: Datei nicht eingefügt"
    );
    assert!(
        wait_for(LONG, || !second.exists()),
        "ein Ausschneiden verschiebt"
    );

    // The other direction: the surface copies its tile, another program
    // reads the file list in both formats.
    let mut copied = String::new();
    let got = wait_for(LONG, || {
        sway.click(42, 42);
        let _ = sway
            .command(&wtype)
            .args(KEY_DELAY)
            .args(["-M", "ctrl", "c", "-m", "ctrl"])
            .status();
        copied = sway
            .command(&wl_paste)
            .args(["--type", "text/uri-list"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        copied.contains("Ziel.md")
    });
    if !got {
        // What the compositor's selection offers now tells a missing
        // selection (another program's or none) from a missing type.
        let types = sway
            .command(&wl_paste)
            .arg("--list-types")
            .output()
            .map(|out| {
                format!(
                    "{} {}",
                    String::from_utf8_lossy(&out.stdout).replace('\n', ","),
                    String::from_utf8_lossy(&out.stderr).trim()
                )
            })
            .unwrap_or_default();
        let running = surface.running();
        let ended = surface.stop();
        panic!(
            "text/uri-list der Fläche fehlt: {copied:?}; Auswahl bietet: {types}; \
             Fläche lief noch: {running} ({ended:?}); Stderr der Fläche: {}",
            surface.stderr_text().trim()
        );
    }
    assert_eq!(
        copied.trim_end(),
        super::clipboard_uri::file_uri(&desktop.join("Ziel.md"))
    );
    let gnome = sway
        .command(&wl_paste)
        .args(["--type", "x-special/gnome-copied-files"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&gnome.stdout).starts_with("copy\nfile://"));
    assert!(surface.running());
}

#[test]
#[ignore = "needs sway (suite stage linux-wayland: layer)"]
fn desktop_task_layer_no_gpu_exits_one() {
    require_task();
    let sway = Sway::start();
    let mut surface = SurfaceProcess::start(
        &sway.env(),
        &["--desktop-surface", "--layer"],
        &[(TEST_NO_ADAPTER, "1".to_string())],
    );
    let status = surface
        .wait_exit(LONG)
        .expect("the process must end without an adapter");
    assert_eq!(status.code(), Some(1));
    let stderr = surface.stderr_text();
    assert!(
        stderr.contains("se Desktop: Layer-Backend: keine Grafikbeschleunigung ("),
        "{stderr}"
    );
}

#[test]
#[ignore = "needs readelf and the GUI binary (suite stage linux-wayland: layer)"]
fn desktop_task_layer_no_buildtime_wayland_link() {
    require_task();
    let readelf = tool("readelf");
    for binary in [gui_binary(), std::env::current_exe().unwrap()] {
        let output = Command::new(&readelf)
            .arg("-d")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(output.status.success(), "readelf -d {}", binary.display());
        let text = String::from_utf8_lossy(&output.stdout);
        let needed: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("(NEEDED)"))
            .collect();
        assert!(
            !needed.is_empty(),
            "{}: keine NEEDED-Einträge gelesen",
            binary.display()
        );
        for line in needed {
            assert!(
                !line.contains("libwayland-client") && !line.contains("libxkbcommon"),
                "{}: {line}",
                binary.display()
            );
        }
    }
}

/// The names of the packages of a `Cargo.lock` with the names of their
/// dependencies.
fn lock_packages(text: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut packages = std::collections::BTreeMap::new();
    let mut current: Option<String> = None;
    let mut in_dependencies = false;
    for line in text.lines() {
        if line == "[[package]]" {
            current = None;
            in_dependencies = false;
        } else if let Some(name) = line.strip_prefix("name = \"") {
            let name = name.trim_end_matches('"').to_string();
            packages.entry(name.clone()).or_insert_with(Vec::new);
            current = Some(name);
        } else if line == "dependencies = [" {
            in_dependencies = true;
        } else if line == "]" {
            in_dependencies = false;
        } else if in_dependencies {
            let dependency = line.trim().trim_matches(|c| c == '"' || c == ',');
            let dependency = dependency.split(' ').next().unwrap_or_default().to_string();
            if let Some(name) = &current {
                packages.entry(name.clone()).or_default().push(dependency);
            }
        }
    }
    packages
}

/// The packages that `roots` pull in (themselves included).
fn closure(
    packages: &std::collections::BTreeMap<String, Vec<String>>,
    roots: &[&str],
) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut open: Vec<String> = roots.iter().map(|root| root.to_string()).collect();
    while let Some(name) = open.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        open.extend(packages.get(&name).cloned().unwrap_or_default());
    }
    seen
}

/// The packages the S5 dependencies of the plan may have added to the lock
/// against `da2646d2` (C8, DN22): the image decoders of the background
/// picture, together with S3 OE5's (A23).
const ALLOWED_NEW: [&str; 7] = [
    "zune-jpeg",
    "zune-core",
    "image-webp",
    "quick-error",
    "gif",
    "weezl",
    "color_quant",
];

#[test]
#[ignore = "needs git history and the lock file (suite stage linux-wayland: layer)"]
fn desktop_task_lock_only_expected_additions() {
    require_task();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let current = std::fs::read_to_string(manifest.join("Cargo.lock")).unwrap();
    let baseline = Command::new("git")
        .args(["show", "da2646d2:native/Cargo.lock"])
        .current_dir(&manifest)
        .output()
        .unwrap_or_else(|error| not_run(&format!("git: {error}")));
    if !baseline.status.success() {
        not_run("da2646d2 fehlt in der Historie (flacher Checkout)");
    }
    let before = lock_packages(&String::from_utf8_lossy(&baseline.stdout));
    let after = lock_packages(&current);
    // The S5 dependencies of the layer backend and the picture decoders.
    let reached = closure(
        &after,
        &[
            "smithay-client-toolkit",
            "wayland-client",
            "wayland-backend",
            "wayland-protocols",
            "wayland-protocols-wlr",
            "wayland-cursor",
            "calloop",
            "calloop-wayland-source",
            "xkbcommon-dl",
            "egui-wgpu",
            "raw-window-handle",
            "x11rb",
            "image",
        ],
    );
    let added: Vec<&String> = after
        .keys()
        .filter(|name| !before.contains_key(*name) && reached.contains(*name))
        .collect();
    let unexpected: Vec<&&String> = added
        .iter()
        .filter(|name| !ALLOWED_NEW.contains(&name.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "neue Lock-Pakete der S5-Abhängigkeiten außerhalb von C8: {unexpected:?}"
    );
}

#[test]
#[ignore = "needs KWin in the session (suite stage desktop-linux-kwin, assumption A14)"]
fn desktop_task_kwin_layer_namespace_desktop() {
    if std::env::var("SE_DESKTOP_TASK_KWIN").as_deref() != Ok("1") {
        not_run("SE_DESKTOP_TASK_KWIN=1 fehlt (KWin-Umgebung der S4-Stufe)");
    }
    let env: Vec<(&str, String)> = ["XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| (name, value)))
        .chain([("XDG_CURRENT_DESKTOP", "KDE".to_string())])
        .collect();
    let mut surface = SurfaceProcess::start(&env, &["--desktop-surface", "--layer"], &[]);
    // KWin accepts the namespace `desktop` and keeps the process alive; the
    // compositor closes a refused layer surface, which ends the process.
    assert!(surface
        .wait_message(LONG, |message| matches!(message, SurfaceMessage::Alive))
        .is_some());
    std::thread::sleep(Duration::from_secs(3));
    assert!(surface.running(), "{}", surface.stderr_text());
}
