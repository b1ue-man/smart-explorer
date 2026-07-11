use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CARGO_SE_BINARY: &str = env!("CARGO_BIN_EXE_se");
static NEXT_SANDBOX: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Sandbox {
    pub(crate) root: PathBuf,
    home: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    pub(crate) fn new(name: &str) -> Self {
        let sequence = NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "smart-explorer-se-cli-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let home = root.join("home");
        let data = root.join("app-data");
        fs::create_dir_all(&home).expect("create isolated home directory");
        fs::create_dir_all(&data).expect("create isolated app-data directory");
        Self { root, home, data }
    }

    pub(crate) fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    pub(crate) fn app_data_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.data.join("smart_explorer").join(relative)
    }

    pub(crate) fn command(&self) -> Command {
        let binary = std::env::var_os("SMART_EXPLORER_SE_BINARY")
            .unwrap_or_else(|| OsString::from(CARGO_SE_BINARY));
        let mut command = Command::new(binary);
        command
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", self.data.join("config"))
            .env("APPDATA", &self.data)
            .env("LOCALAPPDATA", &self.data);

        // A terminal command must not accidentally inherit a graphical login's
        // credential bus. These removals make every subprocess representative of
        // SSH, CI, a service, or another genuinely display-less invocation.
        for name in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_RUNTIME_DIR",
            "GNOME_KEYRING_CONTROL",
            "SSH_AUTH_SOCK",
        ] {
            command.env_remove(name);
        }
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn run(command: &mut Command) -> Output {
    command.output().expect("launch the Cargo-built se binary")
}

#[cfg(target_os = "linux")]
pub(crate) fn run_with_stdin_bounded(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
) -> BoundedOutput {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("launch se with piped stdin");
    child
        .stdin
        .take()
        .expect("se stdin must be piped")
        .write_all(input)
        .expect("write se stdin fixture");
    collect_bounded(child, timeout)
}

pub(crate) struct BoundedOutput {
    pub(crate) output: Output,
    pub(crate) timed_out: bool,
}

pub(crate) fn run_bounded(command: &mut Command, timeout: Duration) -> BoundedOutput {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().expect("launch bounded se subprocess");
    collect_bounded(child, timeout)
}

pub(crate) fn collect_bounded(mut child: Child, timeout: Duration) -> BoundedOutput {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .expect("poll bounded se subprocess")
            .is_some()
        {
            return BoundedOutput {
                output: child
                    .wait_with_output()
                    .expect("collect bounded se subprocess output"),
                timed_out: false,
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return BoundedOutput {
                output: child
                    .wait_with_output()
                    .expect("collect killed se subprocess output"),
                timed_out: true,
            };
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn spawn_captured(command: &mut Command) -> Child {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch concurrent se subprocess")
}

pub(crate) fn assert_success(output: &Output) {
    assert_success_for("se", output);
}

pub(crate) fn assert_success_for(operation: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{operation} failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn assert_exit_code(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected se status\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("se stdout must be UTF-8")
}

pub(crate) fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("se stderr must be UTF-8")
}

#[cfg(target_os = "linux")]
pub(crate) fn assert_output_omits(output: &Output, secret: &str) {
    assert!(
        !output
            .stdout
            .windows(secret.len())
            .any(|part| part == secret.as_bytes()),
        "se wrote a secret to stdout"
    );
    assert!(
        !output
            .stderr
            .windows(secret.len())
            .any(|part| part == secret.as_bytes()),
        "se wrote a secret to stderr"
    );
}

pub(crate) fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub(crate) fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}
