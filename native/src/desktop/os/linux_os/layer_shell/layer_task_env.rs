//! Environment of the DSK-M7 checks with a compositor (suite stage
//! `linux-wayland: layer`): a headless Sway 1.9 (`WLR_BACKENDS=headless`,
//! `WLR_RENDERER=pixman`) that the check starts and ends itself, `grim` for
//! screenshots, and the real GUI program as the layer process with a
//! channel the check plays the shell on. Every tool is found on `PATH`; a
//! missing one stops the check with "nicht ausgeführt: <Grund>", which the
//! suite reports as not run, never as passed.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::desktop::core::config::{Background, BackgroundKind, DesktopConfig, SystemItems};
use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::layout::{Cell, ItemId, MonitorLayout};
use crate::desktop::core::protocol::{
    read_frame, write_frame, ShellMessage, SurfaceMessage, SurfaceModel, PROTOCOL_VERSION,
};
use crate::desktop::core::tiles::{project, ShareState, TileInput};
use crate::share::ShareProfiles;

/// The test background colour of the surface.
pub(super) const MAGENTA: [u8; 3] = [255, 0, 255];

pub(super) fn not_run(reason: &str) -> ! {
    panic!("nicht ausgeführt: {reason}")
}

/// Only the suite's compositor stage runs these (`--include-ignored`).
pub(super) fn require_task() {
    if std::env::var("SE_DESKTOP_TASK_WAYLAND").as_deref() != Ok("1") {
        not_run("SE_DESKTOP_TASK_WAYLAND=1 fehlt (Stufe linux-wayland: layer)");
    }
}

pub(super) fn tool(name: &str) -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| not_run(&format!("{name} nicht gefunden")))
}

pub(super) fn gui_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("SE_DESKTOP_TASK_GUI") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap();
    let target = exe.parent().and_then(Path::parent).unwrap();
    let gui = target.join("smart_explorer");
    if !gui.is_file() {
        not_run(&format!(
            "{} fehlt (cargo build --bin smart_explorer)",
            gui.display()
        ));
    }
    gui
}

pub(super) fn wait_for(limit: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ready()
}

/// A screenshot (`grim -t ppm`).
pub(super) struct Shot {
    pub(super) width: usize,
    pub(super) height: usize,
    pixels: Vec<u8>,
}

impl Shot {
    fn parse(bytes: &[u8]) -> Option<Shot> {
        // "P6\n<width> <height>\n255\n" and the raw RGB bytes.
        let mut fields = Vec::new();
        let mut index = 0;
        while fields.len() < 4 && index < bytes.len() {
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            fields.push(std::str::from_utf8(&bytes[start..index]).ok()?);
        }
        let [magic, width, height, _max] = fields.try_into().ok()?;
        if magic != "P6" {
            return None;
        }
        Some(Shot {
            width: width.parse().ok()?,
            height: height.parse().ok()?,
            pixels: bytes.get(index + 1..)?.to_vec(),
        })
    }

    pub(super) fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * self.width + x) * 3;
        [self.pixels[at], self.pixels[at + 1], self.pixels[at + 2]]
    }

    pub(super) fn contains(&self, color: [u8; 3]) -> bool {
        self.pixels.chunks_exact(3).any(|pixel| pixel == color)
    }
}

/// A headless Sway with its sockets in a private runtime folder.
pub(super) struct Sway {
    child: Child,
    runtime: tempfile::TempDir,
    display: String,
    socket: PathBuf,
}

impl Sway {
    pub(super) fn start() -> Sway {
        let sway = tool("sway");
        let runtime = tempfile::Builder::new()
            .prefix("sway-run")
            .tempdir()
            .unwrap();
        let config = runtime.path().join("config");
        // The test window floats at its own size, so the layer surface stays
        // visible around it (a tiled window would fill the whole output).
        std::fs::write(
            &config,
            "default_border none\nfor_window [app_id=\"app.smartexplorer.desktop\"] floating enable\n",
        )
        .unwrap();
        let child = Command::new(sway)
            .arg("-c")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("SWAYSOCK")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| not_run(&format!("sway startet nicht: {error}")));
        let mut sway = Sway {
            child,
            runtime,
            display: String::new(),
            socket: PathBuf::new(),
        };
        let found = wait_for(Duration::from_secs(30), || {
            let names: Vec<String> = std::fs::read_dir(sway.runtime.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            let display = names
                .iter()
                .find(|name| name.starts_with("wayland-") && !name.ends_with(".lock"));
            let socket = names
                .iter()
                .find(|name| name.starts_with("sway-ipc.") && name.ends_with(".sock"));
            if let (Some(display), Some(socket)) = (display, socket) {
                sway.display = display.clone();
                sway.socket = sway.runtime.path().join(socket);
                return true;
            }
            false
        });
        if !found {
            not_run("sway legt seine Sockets nicht an");
        }
        sway
    }

    /// The environment a Wayland client of this compositor needs.
    pub(super) fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "XDG_RUNTIME_DIR",
                self.runtime.path().to_string_lossy().into_owned(),
            ),
            ("WAYLAND_DISPLAY", self.display.clone()),
            ("SWAYSOCK", self.socket.to_string_lossy().into_owned()),
            ("XDG_CURRENT_DESKTOP", "sway".into()),
        ]
    }

    pub(super) fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.envs(self.env()).env_remove("DISPLAY");
        command
    }

    /// `swaymsg <args>`: its output.
    pub(super) fn msg(&self, args: &[&str]) -> String {
        let output = self
            .command(&tool("swaymsg"))
            .args(args)
            .output()
            .unwrap_or_else(|error| not_run(&format!("swaymsg: {error}")));
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The names of the outputs (`HEADLESS-1`, …).
    pub(super) fn outputs(&self) -> Vec<String> {
        let json: serde_json::Value =
            serde_json::from_str(&self.msg(&["-t", "get_outputs", "-r"])).unwrap_or_default();
        let mut names: Vec<String> = json
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|output| output["name"].as_str().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    pub(super) fn screenshot(&self, output: &str) -> Option<Shot> {
        let result = self
            .command(&tool("grim"))
            .args(["-t", "ppm", "-o", output, "-"])
            .output()
            .ok()?;
        Shot::parse(&result.stdout)
    }

    /// Moves the pointer to a point of the layout and presses the left button.
    pub(super) fn click(&self, x: i32, y: i32) {
        self.msg(&["seat", "-", "cursor", "set", &x.to_string(), &y.to_string()]);
        self.msg(&["seat", "-", "cursor", "press", "button1"]);
        self.msg(&["seat", "-", "cursor", "release", "button1"]);
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The GUI program as a surface process; the check plays the shell.
pub(super) struct SurfaceProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<SurfaceMessage>,
    stderr: Option<std::process::ChildStderr>,
    _data: tempfile::TempDir,
}

impl SurfaceProcess {
    /// `smart_explorer <args>` in the compositor of `env`, with data and
    /// configuration folders of its own.
    pub(super) fn start(
        env: &[(&str, String)],
        args: &[&str],
        extra: &[(&str, String)],
    ) -> SurfaceProcess {
        let data = tempfile::tempdir().unwrap();
        let mut command = Command::new(gui_binary());
        command
            .args(args)
            .envs(env.iter().map(|(name, value)| (*name, value)))
            .envs(extra.iter().map(|(name, value)| (*name, value)))
            .env("XDG_DATA_HOME", data.path().join("data"))
            .env("XDG_CONFIG_HOME", data.path().join("config"))
            .env(
                "SMART_EXPLORER_E2E_TEST_NAMESPACE",
                format!("layer_task_{}", std::process::id()),
            )
            .env_remove("DISPLAY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| not_run(&format!("GUI-Programm startet nicht: {error}")));
        let mut stdout = child.stdout.take().unwrap();
        let (sender, frames) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(Some(message)) = read_frame::<_, SurfaceMessage>(&mut stdout) {
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        SurfaceProcess {
            stdin: child.stdin.take(),
            stderr: child.stderr.take(),
            child,
            frames,
            _data: data,
        }
    }

    pub(super) fn send(&mut self, message: &ShellMessage) {
        if let Some(stdin) = self.stdin.as_mut() {
            write_frame(stdin, message).unwrap();
        }
    }

    /// The first message the surface sends that `wanted` accepts.
    pub(super) fn wait_message(
        &self,
        limit: Duration,
        wanted: impl Fn(&SurfaceMessage) -> bool,
    ) -> Option<SurfaceMessage> {
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.frames.recv_timeout(left) {
                Ok(message) if wanted(&message) => return Some(message),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    pub(super) fn wait_exit(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub(super) fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ends the process (if it still runs) and gives its exit status.
    pub(super) fn stop(&mut self) -> Option<ExitStatus> {
        let _ = self.child.kill();
        self.child.wait().ok()
    }

    /// What the process wrote on stderr (read after it ended).
    pub(super) fn stderr_text(&mut self) -> String {
        let mut text = String::new();
        if let Some(stderr) = self.stderr.as_mut() {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    }
}

impl Drop for SurfaceProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The configuration with a solid background colour.
pub(super) fn config_with(color: [u8; 3]) -> ShellMessage {
    let mut config = DesktopConfig::initial(SystemItems::initial(None, None));
    config.background = Background {
        kind: BackgroundKind::Color,
        value: Some(format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])),
    };
    ShellMessage::Config { config }
}

/// The model of one output: an empty one, or with the named desktop files
/// from the first cell on.
pub(super) fn model_for(key: &str, files: &[&str]) -> ShellMessage {
    let profiles = ShareProfiles::default();
    let (paths, state) = (std::collections::BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&profiles),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    let mut layout = MonitorLayout::new(key, 8).unwrap();
    let desktop_files: Vec<DesktopFile> = files
        .iter()
        .enumerate()
        .map(|(row, name)| {
            layout.cells.insert(
                ItemId::File {
                    name: (*name).to_string(),
                },
                Cell {
                    col: 0,
                    row: row as u32,
                },
            );
            DesktopFile {
                name: (*name).to_string(),
                is_dir: false,
                public: false,
                launcher: false,
                hidden: false,
            }
        })
        .collect();
    ShellMessage::Model {
        rev: 1,
        monitor: Some(key.to_string()),
        model: Box::new(SurfaceModel {
            tiles,
            files: desktop_files,
            system_items: Vec::new(),
            screens: false,
            layout,
        }),
    }
}

pub(super) fn hello() -> ShellMessage {
    ShellMessage::Hello {
        protocol: PROTOCOL_VERSION,
        monitor: "layer".into(),
    }
}
