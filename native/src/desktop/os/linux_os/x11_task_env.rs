//! Environment of the X11 checks (suite stage `linux-desktop: x11`,
//! `SE_DESKTOP_TASK_X11=1`, one test thread, `--include-ignored`): an Xvfb
//! that the check starts and ends itself (`DISPLAY` and a private
//! `XDG_RUNTIME_DIR` are set for this process meanwhile), window managers and
//! tools on it, and the real GUI program as a surface process whose channel
//! the check plays the shell on. A missing tool stops the check with
//! "nicht ausgeführt: <Grund>", which the suite reports as not run.
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::x11_facts::{intern, values32, OWN_CLASS};

pub(super) fn not_run(reason: &str) -> ! {
    panic!("nicht ausgeführt: {reason}")
}

/// Only the suite's X11 stage runs these (`--include-ignored`).
pub(super) fn require_task() {
    if std::env::var("SE_DESKTOP_TASK_X11").as_deref() != Ok("1") {
        not_run("SE_DESKTOP_TASK_X11=1 fehlt (Stufe linux-desktop: x11)");
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

/// An Xvfb and what runs on it.
pub(super) struct XServer {
    server: Child,
    helpers: Vec<Child>,
    saved: Vec<(&'static str, Option<OsString>)>,
    pub(super) display: String,
    pub(super) runtime: tempfile::TempDir,
}

impl XServer {
    /// `geometry` is the screen as Xvfb takes it (`1280x800x24`).
    pub(super) fn start(geometry: &str) -> XServer {
        let runtime = tempfile::Builder::new().prefix("x11run").tempdir().unwrap();
        let mut server = Command::new(tool("Xvfb"))
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                geometry,
                "-nolisten",
                "tcp",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| not_run(&format!("Xvfb startet nicht: {error}")));
        let mut line = String::new();
        if let Some(stdout) = server.stdout.take() {
            let _ = BufReader::new(stdout).read_line(&mut line);
        }
        let Ok(number) = line.trim().parse::<u32>() else {
            let _ = server.kill();
            not_run("Xvfb meldet keine Anzeige");
        };
        let display = format!(":{number}");
        let mut saved = Vec::new();
        let env: [(&'static str, Option<OsString>); 3] = [
            ("DISPLAY", Some(display.clone().into())),
            ("XDG_RUNTIME_DIR", Some(runtime.path().into())),
            ("WAYLAND_DISPLAY", None),
        ];
        for (name, value) in env {
            saved.push((name, std::env::var_os(name)));
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let x = XServer {
            server,
            helpers: Vec::new(),
            saved,
            display,
            runtime,
        };
        if !wait_for(Duration::from_secs(10), || x.try_connect().is_some()) {
            not_run("Xvfb nimmt keine Verbindung an");
        }
        x
    }

    fn try_connect(&self) -> Option<(RustConnection, Window)> {
        let (conn, screen) = RustConnection::connect(Some(&self.display)).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        Some((conn, root))
    }

    pub(super) fn connect(&self) -> (RustConnection, Window) {
        self.try_connect().expect("Verbindung zum Xvfb")
    }

    /// A command on this display with no stream of ours.
    pub(super) fn command(&self, program: &str) -> Command {
        let mut command = Command::new(tool(program));
        command
            .env("DISPLAY", &self.display)
            .env("XDG_RUNTIME_DIR", self.runtime.path())
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Starts a program that stays until the check ends.
    pub(super) fn spawn(&mut self, program: &str, args: &[&str]) -> usize {
        let child = self
            .command(program)
            .args(args)
            .spawn()
            .unwrap_or_else(|error| not_run(&format!("{program} startet nicht: {error}")));
        self.helpers.push(child);
        self.helpers.len() - 1
    }

    pub(super) fn helper_running(&mut self, index: usize) -> bool {
        matches!(self.helpers[index].try_wait(), Ok(None))
    }

    /// A short-lived tool; whether it ended with success.
    pub(super) fn run(&self, program: &str, args: &[&str]) -> bool {
        self.command(program)
            .args(args)
            .status()
            .unwrap_or_else(|error| not_run(&format!("{program}: {error}")))
            .success()
    }

    /// What a short-lived tool wrote on its standard output (for the message
    /// of a failed check); a tool that cannot run reads as its error.
    pub(super) fn output(&self, program: &str, args: &[&str]) -> String {
        let mut command = self.command(program);
        command.args(args).stdout(Stdio::piped());
        match command.output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
            Err(error) => format!("{program}: {error}"),
        }
    }

    /// A window manager that announces `_NET_SUPPORTED` within ten seconds.
    pub(super) fn start_window_manager(&mut self, program: &str, args: &[&str]) {
        self.spawn(program, args);
        let (conn, root) = self.connect();
        let atom = intern(&conn, "_NET_SUPPORTING_WM_CHECK").unwrap();
        let found = wait_for(Duration::from_secs(10), || {
            !values32(&conn, root, atom, AtomEnum::WINDOW).is_empty()
        });
        if !found {
            not_run(&format!("{program} meldet sich nicht als Fenstermanager"));
        }
    }
}

impl Drop for XServer {
    fn drop(&mut self) {
        for helper in &mut self.helpers {
            let _ = helper.kill();
            let _ = helper.wait();
        }
        let _ = self.server.kill();
        let _ = self.server.wait();
        for (name, value) in self.saved.drain(..) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// What the check reads from a window.
#[derive(Debug)]
pub(super) struct WindowInfo {
    pub(super) window: Window,
    pub(super) types: Vec<String>,
    pub(super) state: Vec<String>,
    pub(super) desktop: Option<u32>,
}

fn atom_names(conn: &RustConnection, atoms: &[Atom]) -> Vec<String> {
    atoms
        .iter()
        .filter_map(|atom| conn.get_atom_name(*atom).ok()?.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
        .collect()
}

fn atoms_of(conn: &RustConnection, window: Window, property: &str) -> Vec<String> {
    intern(conn, property)
        .map(|atom| values32(conn, window, atom, AtomEnum::ATOM))
        .map(|atoms| atom_names(conn, &atoms))
        .unwrap_or_default()
}

pub(super) fn window_info(conn: &RustConnection, window: Window) -> WindowInfo {
    WindowInfo {
        window,
        types: atoms_of(conn, window, "_NET_WM_WINDOW_TYPE"),
        state: atoms_of(conn, window, "_NET_WM_STATE"),
        desktop: intern(conn, "_NET_WM_DESKTOP")
            .map(|atom| values32(conn, window, atom, AtomEnum::CARDINAL))
            .and_then(|values| values.first().copied()),
    }
}

/// The windows the window manager lists, bottom to top when `stacking`.
pub(super) fn client_windows(conn: &RustConnection, root: Window, stacking: bool) -> Vec<Window> {
    let property = if stacking {
        "_NET_CLIENT_LIST_STACKING"
    } else {
        "_NET_CLIENT_LIST"
    };
    intern(conn, property)
        .map(|atom| values32(conn, root, atom, AtomEnum::WINDOW))
        .unwrap_or_default()
}

fn class_of(conn: &RustConnection, window: Window) -> String {
    conn.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .unwrap_or_default()
}

fn title_of(conn: &RustConnection, window: Window) -> String {
    conn.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
        .unwrap_or_default()
}

/// The windows below `window` (a window manager reparents its clients).
fn below(conn: &RustConnection, window: Window, depth: u8, found: &mut Vec<Window>) {
    let Some(tree) = conn
        .query_tree(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    for child in tree.children {
        found.push(child);
        if depth > 0 {
            below(conn, child, depth - 1, found);
        }
    }
}

/// The surface windows, the listed clients first (a window of type DESKTOP
/// may be missing from the lists): `(window, title)`.
pub(super) fn surface_windows(conn: &RustConnection, root: Window) -> Vec<(Window, String)> {
    let mut candidates = client_windows(conn, root, false);
    below(conn, root, 2, &mut candidates);
    candidates.sort_unstable();
    candidates.dedup();
    candidates
        .into_iter()
        .filter(|window| class_of(conn, *window).contains(OWN_CLASS))
        .map(|window| (window, title_of(conn, window)))
        .collect()
}

/// The root position of a client window.
pub(super) fn root_position(
    conn: &RustConnection,
    root: Window,
    window: Window,
) -> Option<(i16, i16)> {
    let reply = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((reply.dst_x, reply.dst_y))
}
