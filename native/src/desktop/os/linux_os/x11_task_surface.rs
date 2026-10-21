//! The GUI program as a surface process on the X11 checks' display
//! (`x11_task_env`): the check plays the shell on its channel, greets it
//! and gives it a colour and a model.
use std::io::Read;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::x11_task_env::{gui_binary, not_run, XServer};
use crate::desktop::core::config::{Background, BackgroundKind, DesktopConfig, SystemItems};
use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::layout::{Cell, ItemId, MonitorLayout};
use crate::desktop::core::protocol::{
    read_frame, write_frame, ShellMessage, SurfaceMessage, SurfaceModel, PROTOCOL_VERSION,
};
use crate::desktop::core::tiles::{project, ShareState, TileInput};
use crate::share::ShareProfiles;

/// The GUI program as a surface process; the check plays the shell.
pub(super) struct SurfaceProc {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<SurfaceMessage>,
    /// Everything the process wrote on stderr so far (a thread drains the
    /// pipe, so a chatty process never blocks on a full pipe and a running
    /// one can be asked what it said).
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_reader: Option<JoinHandle<()>>,
    _data: tempfile::TempDir,
}

impl SurfaceProc {
    pub(super) fn start(x: &XServer, args: &[&str], extra: &[(&str, &str)]) -> SurfaceProc {
        let data = tempfile::tempdir().unwrap();
        let mut command = Command::new(gui_binary());
        command
            .args(args)
            .env("DISPLAY", &x.display)
            .env("XDG_RUNTIME_DIR", x.runtime.path())
            .env("XDG_DATA_HOME", data.path().join("data"))
            .env("XDG_CONFIG_HOME", data.path().join("config"))
            .env_remove("WAYLAND_DISPLAY")
            .env(
                "SMART_EXPLORER_E2E_TEST_NAMESPACE",
                format!("x11_task_{}", std::process::id()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in extra {
            command.env(name, value);
        }
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
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_reader = child.stderr.take().map(|mut pipe| {
            let sink = stderr.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(read) = pipe.read(&mut chunk) {
                    if read == 0 {
                        return;
                    }
                    if let Ok(mut text) = sink.lock() {
                        text.extend_from_slice(&chunk[..read]);
                    }
                }
            })
        });
        SurfaceProc {
            stdin: child.stdin.take(),
            stderr,
            stderr_reader,
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

    /// What the process wrote on stderr so far; one that ended is read to
    /// its end first.
    pub(super) fn stderr_text(&mut self) -> String {
        if !self.running() {
            if let Some(reader) = self.stderr_reader.take() {
                let _ = reader.join();
            }
        }
        self.stderr
            .lock()
            .map(|text| String::from_utf8_lossy(&text).into_owned())
            .unwrap_or_default()
    }

    /// For the message of a failed check: whether the process runs or how it
    /// ended, and its stderr.
    pub(super) fn describe(&mut self) -> String {
        let state = match self.child.try_wait() {
            Ok(None) => "läuft".to_string(),
            Ok(Some(status)) => format!("beendet ({status})"),
            Err(error) => format!("Zustand unbekannt ({error})"),
        };
        format!(
            "Fläche pid {} {state}; stderr: {:?}",
            self.child.id(),
            self.stderr_text()
        )
    }

    /// Greeting, colour and an empty model for `monitor`; the process ending
    /// on its own because it cannot draw is "not run".
    pub(super) fn greet(&mut self, monitor: &str, files: &[&str]) {
        let greeting = self.wait_message(
            Duration::from_secs(30),
            |message| matches!(message, SurfaceMessage::Hello { monitor: name, .. } if name == monitor),
        );
        assert!(greeting.is_some(), "keine Begrüßung der Fläche {monitor}");
        self.send(&ShellMessage::Hello {
            protocol: PROTOCOL_VERSION,
            monitor: monitor.into(),
        });
        self.send(&config_with([40, 60, 90]));
        self.send(&model_for(monitor, files));
    }

    /// Ends the check as "not run" when the process ended because it has no
    /// graphics.
    pub(super) fn not_run_if_ended(&mut self) {
        if self.running() {
            return;
        }
        let status = self.wait_exit(Duration::from_secs(1));
        let stderr = self.stderr_text();
        if stderr.contains("keine Grafikbeschleunigung") || stderr.contains("Adapter") {
            not_run(&format!("kein Grafikadapter: {}", stderr.trim()));
        }
        panic!("Fläche beendet ({status:?}): {stderr}");
    }
}

impl Drop for SurfaceProc {
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

/// The model of one monitor: with the named desktop files from the first
/// cell on, one below the other.
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
        monitor: None,
        model: Box::new(SurfaceModel {
            tiles,
            files: desktop_files,
            system_items: Vec::new(),
            screens: false,
            layout,
        }),
    }
}
