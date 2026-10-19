//! Acceptance of `se screen windows` and `se screen snapshot` (SCR-M4): the
//! JSON contract of the window list, its text lines, the refusal of an
//! existing file, the atomic write and the command line.
use clap::Parser;
use serde_json::Value;

use super::snapshot::{check_target, snapshot_json, snapshot_text, write_png};
use super::windows::windows_text;
use super::{windows_value, ScreenCommand};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{AppEntry, ListSource, WindowEntry, WindowList};

#[derive(Parser)]
struct Wrapper {
    #[command(subcommand)]
    command: ScreenCommand,
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort_unstable();
    keys
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    names.sort_unstable();
    names
}

fn list() -> WindowList {
    WindowList {
        windows: vec![
            WindowEntry {
                id: "4242".into(),
                title: "Editor\t- ein\nTitel".into(),
                app: "notepad".into(),
                pid: Some(7),
                monitor: Some("DISPLAY1".into()),
                minimized: false,
                focused: true,
            },
            WindowEntry {
                id: "77".into(),
                title: "Hintergrund".into(),
                app: String::new(),
                pid: None,
                monitor: None,
                minimized: true,
                focused: false,
            },
        ],
        apps: vec![AppEntry {
            app_id: "org.example.app".into(),
            name: "Beispiel".into(),
            pid: None,
        }],
        list_source: ListSource::EnumWindows,
        limits: vec!["secure_desktop_paused".into(), "capture_border".into()],
    }
}

#[test]
fn screen_task_cli_windows_json_shape() {
    let value = windows_value(&list());
    assert_eq!(
        keys(&value),
        sorted(&["windows", "apps", "list_source", "limits"])
    );
    assert_eq!(value["list_source"], "enum_windows");
    let window = &value["windows"][0];
    assert_eq!(
        keys(window),
        sorted(&[
            "id",
            "title",
            "app",
            "pid",
            "monitor",
            "minimized",
            "focused"
        ])
    );
    assert_eq!(window["id"], "4242");
    assert_eq!(window["pid"], 7);
    assert_eq!(window["focused"], true);
    assert_eq!(value["windows"][1]["pid"], Value::Null);
    assert_eq!(value["windows"][1]["monitor"], Value::Null);
    assert_eq!(keys(&value["apps"][0]), sorted(&["app_id", "name", "pid"]));
    assert_eq!(value["limits"][1], "capture_border");
    // The text form: one row per entry, tabs and line breaks in names
    // replaced, `-` for what is missing.
    let text = windows_text(&list());
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[0], "list_source\tenum_windows");
    assert_eq!(
        rows[1],
        "window\t4242\tDISPLAY1\t-\tfocused\tnotepad\tEditor - ein Titel"
    );
    assert_eq!(rows[2], "window\t77\t-\tminimized\t-\t-\tHintergrund");
    assert_eq!(rows[3], "app\torg.example.app\tBeispiel\t-");
    assert_eq!(rows[4], "limit\tsecure_desktop_paused");
    assert_eq!(rows[5], "limit\tcapture_border");
    assert!(text.ends_with('\n'));
    // A list source of a newer host reads as `unknown`.
    let mut other = list();
    other.list_source = ListSource::Unknown;
    assert!(windows_text(&other).starts_with("list_source\tunknown\n"));
}

#[test]
fn screen_task_cli_snapshot_refuses_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("bild.png");
    std::fs::write(&target, b"old").unwrap();
    let refused = check_target(&target, false).unwrap_err();
    assert!(
        refused.ends_with("existiert schon; --force überschreibt"),
        "{refused}"
    );
    assert!(refused.contains("bild.png"), "{refused}");
    // Writing without --force leaves the file as it is, and no helper file.
    assert!(write_png(&target, b"new", false).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    // --force replaces it.
    write_png(&target, b"new", true).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    // A new file is written without a trace of the helper file.
    let fresh = dir.path().join("neu.png");
    check_target(&fresh, false).unwrap();
    write_png(&fresh, b"png", false).unwrap();
    assert_eq!(std::fs::read(&fresh).unwrap(), b"png");
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["bild.png", "neu.png"]);
    // A folder that does not exist is an error naming the target.
    let missing = dir.path().join("fehlt").join("x.png");
    assert!(write_png(&missing, b"png", false)
        .unwrap_err()
        .contains("x.png"));
}

#[test]
fn screen_task_cli_snapshot_output_shapes() {
    let path = std::path::Path::new("/tmp/bild.png");
    let size = FrameSize::new(1280, 720);
    let value = snapshot_json(path, size, 4096, "window:4242");
    assert_eq!(
        keys(&value),
        sorted(&["path", "width", "height", "bytes", "source"])
    );
    assert_eq!(value["width"], 1280);
    assert_eq!(value["height"], 720);
    assert_eq!(value["bytes"], 4096);
    assert_eq!(value["source"], "window:4242");
    assert_eq!(
        snapshot_text(path, size, 4096, "window:4242"),
        "snapshot\t/tmp/bild.png\t1280x720\t4096\twindow:4242\n"
    );
}

#[test]
fn screen_task_cli_windows_and_snapshot_parse() {
    let windows = Wrapper::try_parse_from(["se", "windows", "Laptop", "--json"]).unwrap();
    assert!(matches!(windows.command, ScreenCommand::Windows(_)));
    let snapshot = Wrapper::try_parse_from([
        "se",
        "snapshot",
        "Laptop",
        "--source",
        "window:4242",
        "--out",
        "bild.png",
        "--max-size",
        "1568x1568",
        "--force",
        "--json",
    ])
    .unwrap();
    assert!(matches!(snapshot.command, ScreenCommand::Snapshot(_)));
    // --out is required, and the size must be WIDTHxHEIGHT.
    assert!(Wrapper::try_parse_from(["se", "snapshot", "Laptop"]).is_err());
    assert!(Wrapper::try_parse_from([
        "se",
        "snapshot",
        "Laptop",
        "--out",
        "a.png",
        "--max-size",
        "gross"
    ])
    .is_err());
}
