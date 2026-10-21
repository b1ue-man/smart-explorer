//! Support of the stage tests that run the built `se` (stages `vault_cli`,
//! `vault_mcp`, `vault_klartext`, `vault_klartext_drive`; vault plan VLT-M9
//! and VLT-M14): one isolated environment (HOME, the XDG folders, the
//! runtime folder and TMPDIR below one root, so the real installation is
//! never touched), `se` called with a time limit, and the end of every
//! process those calls started, the background worker among them. Those
//! processes are found by a tag in their environment, which `/proc` shows,
//! so the module exists on Linux only.
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Environment variable with the path of the built `se` of the stage
/// (inventory option `env=SMART_EXPLORER_VAULT_SE_BIN={se}`).
pub(crate) const SE_BIN: &str = "SMART_EXPLORER_VAULT_SE_BIN";
/// The password of every vault the tests create.
pub(crate) const PASSWORD: &str = "Tresor-Passwort-2026";
/// Environment variable that tags every process one `TaskEnv` starts.
const RUN_TAG: &str = "SE_VAULT_TASK_RUN";
/// One call of `se` ends within this time or the test fails.
const CALL_LIMIT: Duration = Duration::from_secs(240);
/// After the process of a call ended, its output pipes are read this long.
/// A background worker started by the call may hold them open.
const PIPE_GRACE: Duration = Duration::from_secs(3);

/// The path as text; test paths are UTF-8.
pub(crate) fn text(path: &Path) -> &str {
    path.to_str().expect("Pfad ist UTF-8")
}

/// `vault://<site><path>` with `path` starting at the vault root (`/…`).
pub(crate) fn vault_url(site: &str, path: &str) -> String {
    format!("vault://{site}{path}")
}

/// The sorted keys of a JSON object.
pub(crate) fn keys(value: &Value) -> Vec<String> {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("kein JSON-Objekt: {value}"));
    let mut keys: Vec<String> = object.keys().cloned().collect();
    keys.sort();
    keys
}

/// Asserts that `value` is an object with exactly the keys `expected`.
pub(crate) fn assert_keys(value: &Value, expected: &[&str], what: &str) {
    let mut wanted: Vec<String> = expected.iter().map(|key| (*key).to_string()).collect();
    wanted.sort();
    assert_eq!(keys(value), wanted, "{what}: Schlüsselmenge");
}

/// The text value `key` of a JSON object.
pub(crate) fn field(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} fehlt oder ist kein Text in {value}"))
        .to_string()
}

/// Copies a folder tree.
pub(crate) fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("Zielordner");
    for entry in std::fs::read_dir(from).expect("Quellordner") {
        let entry = entry.expect("Eintrag");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("Art des Eintrags").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("Datei kopieren");
        }
    }
}

/// Calls `condition` every 250 ms until it holds (`true`) or `limit` is over
/// (`false`).
pub(crate) fn poll_until(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// The result of one call of `se`.
pub(crate) struct Out {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    what: String,
}

impl fmt::Display for Out {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} -> Exitcode {}\n--- stdout\n{}\n--- stderr\n{}",
            self.what, self.code, self.stdout, self.stderr
        )
    }
}

impl Out {
    /// Fails the test unless the call exited with 0.
    pub(crate) fn ok(self) -> Self {
        assert_eq!(self.code, 0, "{self}");
        self
    }

    /// Fails the test unless the call exited with 1 (failed, `se: <message>`
    /// on standard error).
    pub(crate) fn failed(self) -> Self {
        assert_eq!(self.code, 1, "{self}");
        self
    }

    /// The standard output as JSON.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("keine JSON-Ausgabe ({error}): {self}"))
    }
}

/// The output of one pipe, collected by a thread.
struct Collector {
    buffer: Arc<Mutex<Vec<u8>>>,
    handle: thread::JoinHandle<()>,
}

impl Collector {
    fn start(mut reader: impl Read + Send + 'static) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&buffer);
        let handle = thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        if let Ok(mut bytes) = shared.lock() {
                            bytes.extend_from_slice(&chunk[..count]);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Self { buffer, handle }
    }

    fn finished(&self) -> bool {
        self.handle.is_finished()
    }

    fn text(&self) -> String {
        let bytes = self.buffer.lock().map(|bytes| bytes.clone());
        String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned()
    }
}

/// Runs `command` to its end (at most `CALL_LIMIT`), feeding `stdin`; `what`
/// names the call in failure messages.
pub(crate) fn run(mut command: Command, stdin: Option<&str>, what: String) -> Out {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("{what}: Start gescheitert: {error}"));
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A few bytes: the pipe takes them at once; it closes on drop.
        let _ = pipe.write_all(text.as_bytes());
    }
    let stdout = Collector::start(child.stdout.take().expect("stdout-Pipe"));
    let stderr = Collector::start(child.stderr.take().expect("stderr-Pipe"));
    let deadline = Instant::now() + CALL_LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => panic!("{what}: Warten gescheitert: {error}"),
        }
    };
    let grace = Instant::now() + PIPE_GRACE;
    while Instant::now() < grace && !(stdout.finished() && stderr.finished()) {
        thread::sleep(Duration::from_millis(20));
    }
    let out = Out {
        code: status.and_then(|status| status.code()).unwrap_or(-1),
        stdout: stdout.text(),
        stderr: stderr.text(),
        what,
    };
    assert!(
        status.is_some(),
        "Zeitüberschreitung nach {} s: {out}",
        CALL_LIMIT.as_secs()
    );
    out
}

/// One isolated environment for calls of the built `se`.
pub(crate) struct TaskEnv {
    root: PathBuf,
    runtime: PathBuf,
    se: PathBuf,
    tag: String,
    _temp: Option<tempfile::TempDir>,
}

impl TaskEnv {
    /// A fresh environment below a temporary folder, removed with the value.
    pub(crate) fn isolated() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("vault-task-")
            .tempdir()
            .expect("temporary folder");
        let root = temp.path().to_path_buf();
        let runtime = root.join("run");
        Self::build(root, runtime, Some(temp))
    }

    /// The environment of the fixture `marker`: the root `SE_VAULT_TASK_ROOT`
    /// stays after the test, because the suite searches it afterwards. The
    /// runtime folder is the `XDG_RUNTIME_DIR` of the test process when
    /// `runtime_from_env` is set (fixture `runtime-tmpfs`), else `run` below
    /// the root.
    pub(crate) fn marker(runtime_from_env: bool) -> Self {
        let root =
            PathBuf::from(std::env::var_os("SE_VAULT_TASK_ROOT").expect(
                "SE_VAULT_TASK_ROOT fehlt: nur in den Stufen vault-klartext (fixture=marker)",
            ));
        let runtime = if runtime_from_env {
            PathBuf::from(
                std::env::var_os("XDG_RUNTIME_DIR")
                    .filter(|dir| !dir.is_empty())
                    .expect("XDG_RUNTIME_DIR fehlt: fixture=runtime-tmpfs"),
            )
        } else {
            root.join("run")
        };
        Self::build(root, runtime, None)
    }

    fn build(root: PathBuf, runtime: PathBuf, temp: Option<tempfile::TempDir>) -> Self {
        let se = PathBuf::from(std::env::var_os(SE_BIN).unwrap_or_else(|| {
            panic!("{SE_BIN} fehlt: nur in den Stufen mit env={SE_BIN}={{se}}")
        }));
        assert!(se.is_file(), "{SE_BIN}={} ist keine Datei", se.display());
        for name in ["home", "data", "config", "cache", "state", "tmp", "input"] {
            std::fs::create_dir_all(root.join(name)).expect("Ordner der Umgebung");
        }
        std::fs::create_dir_all(&runtime).expect("Laufzeitordner");
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
            .expect("Rechte des Laufzeitordners");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        Self {
            root,
            runtime,
            se,
            tag: format!("{:x}-{nanos:x}", std::process::id()),
            _temp: temp,
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The `XDG_RUNTIME_DIR` of the calls.
    pub(crate) fn runtime(&self) -> &Path {
        &self.runtime
    }

    /// `relative` below the root.
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// `se <args>` in this environment (not started yet).
    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.se);
        command.args(args);
        self.apply(&mut command);
        command
    }

    /// Points `command` at this environment.
    pub(crate) fn apply(&self, command: &mut Command) {
        command
            .env_remove("COMPLETE")
            .env("HOME", self.root.join("home"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("TMPDIR", self.root.join("tmp"))
            .env(RUN_TAG, &self.tag)
            .current_dir(&self.root);
    }

    /// Runs `se <args>` without standard input.
    pub(crate) fn se(&self, args: &[&str]) -> Out {
        run(self.command(args), None, format!("se {}", args.join(" ")))
    }

    /// Runs `se <args>` with `stdin` as standard input.
    pub(crate) fn se_in(&self, args: &[&str], stdin: &str) -> Out {
        run(
            self.command(args),
            Some(stdin),
            format!("se {}", args.join(" ")),
        )
    }

    /// `se vault create` of a new vault at `relative` below the root; the
    /// JSON the command printed.
    pub(crate) fn create_json(&self, relative: &str, label: &str) -> Value {
        let location = self.path(relative);
        self.se_in(
            &[
                "vault",
                "create",
                text(&location),
                "--label",
                label,
                "--password-stdin",
                "--json",
            ],
            PASSWORD,
        )
        .ok()
        .json()
    }

    /// A new vault at `relative` below the root; its site ID.
    pub(crate) fn create_vault(&self, relative: &str, label: &str) -> String {
        field(&self.create_json(relative, label), "site")
    }

    /// Unlocks `site` in the background worker (which the call starts).
    pub(crate) fn unlock(&self, site: &str) {
        let out = self
            .se_in(
                &["vault", "unlock", site, "--password-stdin", "--json"],
                PASSWORD,
            )
            .ok();
        assert_eq!(out.json()["state"], "unlocked", "{out}");
    }

    /// `se vault lock --all`.
    pub(crate) fn lock_all(&self) -> Out {
        self.se(&["vault", "lock", "--all"]).ok()
    }

    /// Copies the local file `local` to `path` in the vault `site`, replacing
    /// a file that is there.
    pub(crate) fn put(&self, local: &Path, site: &str, path: &str) {
        let target = vault_url(site, path);
        self.se(&["put", "-f", text(local), target.as_str()]).ok();
    }

    /// `se cat` of `path` in the vault `site`.
    pub(crate) fn cat(&self, site: &str, path: &str) -> Out {
        let target = vault_url(site, path);
        self.se(&["cat", target.as_str()])
    }

    /// The processes of this environment (the calls and what they started).
    fn processes(&self) -> Vec<u32> {
        let wanted = format!("{RUN_TAG}={}", self.tag).into_bytes();
        let own = std::process::id();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == own {
                continue;
            }
            let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
                continue;
            };
            if environ
                .split(|byte| *byte == 0)
                .any(|variable| variable == wanted.as_slice())
            {
                found.push(pid);
            }
        }
        found
    }

    fn signal_processes(&self, signal: libc::c_int) {
        for pid in self.processes() {
            if let Ok(pid) = libc::pid_t::try_from(pid) {
                // SAFETY: a plain signal to a process of this environment.
                let _ = unsafe { libc::kill(pid, signal) };
            }
        }
    }

    fn wait_for_end(&self) -> bool {
        poll_until(Duration::from_secs(5), || self.processes().is_empty())
    }

    /// Ends every process of this environment at once (SIGKILL), as a crash
    /// of the worker would.
    pub(crate) fn kill_processes(&self) {
        self.signal_processes(libc::SIGKILL);
        assert!(
            self.wait_for_end(),
            "Prozesse der Umgebung leben nach SIGKILL: {:?}",
            self.processes()
        );
    }

    /// Ends every process of this environment: SIGTERM, and SIGKILL for
    /// those that stay.
    pub(crate) fn stop_processes(&self) {
        self.signal_processes(libc::SIGTERM);
        if !self.wait_for_end() {
            self.signal_processes(libc::SIGKILL);
            let _ = self.wait_for_end();
        }
    }
}

impl Drop for TaskEnv {
    fn drop(&mut self) {
        self.stop_processes();
    }
}
