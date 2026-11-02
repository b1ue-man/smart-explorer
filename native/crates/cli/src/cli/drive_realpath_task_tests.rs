//! Signal `drive_task_windows_realpath_native_on_dokany` (strand S2,
//! assumption A3 of the plan 2026-10-17): Obsidian and VS Code resolve paths
//! through Node's `fs.realpathSync.native` and `fs.promises.realpath` (libuv
//! `uv_fs_realpath`, which asks `GetFinalPathNameByHandleW`). The check
//! measures both on a drive that the product mounts over Dokany and reports
//! what they return; another path is a finding for A3 and T2 of the plan,
//! not something this change repairs.
//!
//! The drive is the one the commands of the product mount without a server:
//! the device tree "Verbundene Geräte als Ordner" (`se drive devices on`),
//! whose root is the drive letter itself. The built `se` starts the
//! background worker and the drive host as a user's command does; the check
//! switches the tree off again but leaves the worker running (`se` has no
//! command that stops it). The host of the drive has no file to offer on a
//! runner without paired devices, so the path resolved is the root of the
//! drive. Everything that keeps the measurement from running (no Dokany, no
//! `node`, no `se`, a drive that does not appear) ends the check with
//! "nicht ausgeführt: <Grund>", which the suite reports as not run.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// The `se` of the build under test; the stage sets it
/// (`env=SMART_EXPLORER_DRIVE_SE_BIN={se}`).
const SE_BIN: &str = "SMART_EXPLORER_DRIVE_SE_BIN";
/// The path Node resolves.
const TARGET: &str = "SE_TASK_REALPATH_TARGET";
/// The longest one call of `se` or `node` may take; the first call of `se`
/// starts the background worker.
const CALL_LIMIT: Duration = Duration::from_secs(180);
/// How long the drive may need to appear.
const MOUNT_LIMIT: Duration = Duration::from_secs(120);

/// One program: the two realpath calls that matter and the JavaScript one
/// (no `GetFinalPathNameByHandleW`) as the reference; it prints one JSON line.
const SCRIPT: &str = concat!(
    "const fs = require('fs'); ",
    "const target = process.env.SE_TASK_REALPATH_TARGET; ",
    "const report = { node: process.version, libuv: process.versions.uv, target: target }; ",
    "const code = (error) => String(error && error.code ? error.code : error); ",
    "try { report.sync = fs.realpathSync.native(target); } ",
    "catch (error) { report.sync_error = code(error); } ",
    "try { report.js = fs.realpathSync(target); } ",
    "catch (error) { report.js_error = code(error); } ",
    "fs.promises.realpath(target).then(",
    "(value) => { report.promise = value; }, ",
    "(error) => { report.promise_error = code(error); }",
    ").then(() => { console.log(JSON.stringify(report)); });"
);

/// The check cannot run here: the suite reports it as not run, never as
/// passed or failed. The reason is one line.
fn not_run(reason: impl std::fmt::Display) -> ! {
    panic!(
        "nicht ausgeführt: {}",
        reason.to_string().replace(['\r', '\n'], " ")
    )
}

/// The value of a step the measurement needs; its failure is "not run".
fn required<T>(step: Result<T, String>) -> T {
    match step {
        Ok(value) => value,
        Err(reason) => not_run(reason),
    }
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// A temporary file for one output of a program. A pipe would stay open as
/// long as the background worker that `se` starts holds it; a file is read
/// when the program itself has ended.
fn capture() -> Result<(File, Stdio), String> {
    let file = tempfile::tempfile().map_err(|error| format!("temporary file: {error}"))?;
    let child_end = file
        .try_clone()
        .map_err(|error| format!("temporary file: {error}"))?;
    Ok((file, Stdio::from(child_end)))
}

fn read_back(mut file: File) -> String {
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(0)).is_ok() {
        let _ = file.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Runs the program to its end, at most `limit`.
fn run(command: &mut Command, limit: Duration) -> Result<Ran, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let (stdout_file, stdout) = capture()?;
    let (stderr_file, stderr) = capture()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| format!("{program} does not start: {error}"))?;
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} did not end within {} s",
                    limit.as_secs()
                ));
            }
            Err(error) => return Err(format!("{program}: {error}")),
        }
    };
    Ok(Ran {
        code: status.code(),
        stdout: read_back(stdout_file),
        stderr: read_back(stderr_file),
    })
}

/// `se <arguments>`, which prints JSON and ends with 0.
fn se_json(se: &Path, arguments: &[&str]) -> Result<Value, String> {
    let said = format!("se {}", arguments.join(" "));
    let ran = run(Command::new(se).args(arguments), CALL_LIMIT)?;
    if ran.code != Some(0) {
        return Err(format!(
            "{said} ended with {:?}: {} {}",
            ran.code,
            ran.stdout.trim(),
            ran.stderr.trim()
        ));
    }
    serde_json::from_str(&ran.stdout)
        .map_err(|error| format!("{said} printed no JSON ({error}): {}", ran.stdout.trim()))
}

/// Switches the device tree off again, whatever happened in between.
struct DeviceTreeOn<'a>(&'a Path);

impl Drop for DeviceTreeOn<'_> {
    fn drop(&mut self) {
        let _ = se_json(self.0, &["drive", "devices", "off", "--json"]);
    }
}

/// The root of the drive, once the tree reports it mounted.
fn await_mounted(se: &Path) -> Result<PathBuf, String> {
    let deadline = Instant::now() + MOUNT_LIMIT;
    loop {
        let status = se_json(se, &["drive", "devices", "status", "--json"])?;
        if status["mounted"] == true {
            if let Some(root) = status["root_path"].as_str() {
                return Ok(PathBuf::from(root));
            }
        }
        if status["state"] == "failed" || Instant::now() >= deadline {
            return Err(format!("the device tree did not mount: {status}"));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn node_version() -> Result<String, String> {
    let ran = run(
        Command::new("node").arg("--version"),
        Duration::from_secs(60),
    )?;
    if ran.code == Some(0) {
        Ok(ran.stdout.trim().to_string())
    } else {
        Err(format!(
            "node --version ended with {:?}: {}",
            ran.code,
            ran.stderr.trim()
        ))
    }
}

/// What Node resolves `target` to.
fn measure(target: &Path) -> Result<Value, String> {
    let ran = run(
        Command::new("node")
            .arg("-e")
            .arg(SCRIPT)
            .env(TARGET, target),
        CALL_LIMIT,
    )?;
    if ran.code != Some(0) {
        return Err(format!(
            "node ended with {:?}: {} {}",
            ran.code,
            ran.stdout.trim(),
            ran.stderr.trim()
        ));
    }
    let line = ran.stdout.lines().last().unwrap_or_default();
    serde_json::from_str(line)
        .map_err(|error| format!("node printed no JSON ({error}): {}", ran.stdout.trim()))
}

/// Windows spells a path with either slash and any letter case, and the
/// root of a drive with or without the last separator.
fn canonical(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[test]
#[ignore = "needs the pinned Dokany runtime, node and the built se (Windows task stage windows-core: drive-dokany-regression)"]
fn drive_task_windows_realpath_native_on_dokany() {
    let Some(se) = std::env::var_os(SE_BIN).map(PathBuf::from) else {
        not_run(format!("{SE_BIN} fehlt (Platzhalter {{se}} der Stufe)"));
    };
    if !se.is_file() {
        not_run(format!("{} ist keine Datei", se.display()));
    }
    if let Err(error) = crate::mount::drive_runtime_info() {
        not_run(format!("die Dokany-Laufzeit ist nicht bereit: {error}"));
    }
    let node = required(node_version().map_err(|error| format!("node fehlt: {error}")));

    // The check switches off only a tree it switched on.
    let before = required(se_json(&se, &["drive", "devices", "status", "--json"]));
    if before["enabled"] == true {
        not_run("der Gerätebaum ist schon eingeschaltet; die Prüfung schaltet keinen fremden aus");
    }
    let started = required(se_json(&se, &["drive", "devices", "on", "--json"]));
    let _switched_on = DeviceTreeOn(&se);
    let root = required(
        await_mounted(&se).map_err(|error| format!("{error} (nach `devices on`: {started})")),
    );
    let report = required(measure(&root));
    eprintln!(
        "[drive task] node {node} on the Dokany drive {}: {report}",
        root.display()
    );

    let expected = canonical(&root.to_string_lossy());
    for (key, call) in [
        ("sync", "fs.realpathSync.native"),
        ("promise", "fs.promises.realpath"),
    ] {
        let resolved = report[key].as_str();
        assert!(
            resolved.is_some_and(|resolved| canonical(resolved) == expected),
            "finding for A3: {call} on the Dokany drive {} returned {} (error {}), expected the path itself; report {report}",
            root.display(),
            report[key],
            report[format!("{key}_error")]
        );
    }
}
