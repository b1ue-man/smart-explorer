use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(windows)]
use std::io::{self, Read};
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

pub(crate) fn collect_bounded(child: Child, timeout: Duration) -> BoundedOutput {
    #[cfg(windows)]
    {
        collect_bounded_windows(child, timeout)
    }

    #[cfg(not(windows))]
    {
        collect_bounded_portable(child, timeout)
    }
}

#[cfg(not(windows))]
fn collect_bounded_portable(mut child: Child, timeout: Duration) -> BoundedOutput {
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

#[cfg(windows)]
fn collect_bounded_windows(mut child: Child, timeout: Duration) -> BoundedOutput {
    use std::os::windows::io::AsRawHandle;

    const PIPE_CLOSE_GRACE: Duration = Duration::from_millis(250);
    const KILL_GRACE: Duration = Duration::from_secs(5);

    let mut stdout = child
        .stdout
        .take()
        .expect("bounded se stdout must be piped");
    let mut stderr = child
        .stderr
        .take()
        .expect("bounded se stderr must be piped");
    let stdout_handle = stdout.as_raw_handle();
    let stderr_handle = stderr.as_raw_handle();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut exited_at = None;
    let mut timed_out = false;
    let mut kill_deadline = None;

    loop {
        if stdout_open {
            stdout_open = drain_windows_pipe(&mut stdout, stdout_handle, &mut stdout_bytes)
                .expect("drain bounded se stdout");
        }
        if stderr_open {
            stderr_open = drain_windows_pipe(&mut stderr, stderr_handle, &mut stderr_bytes)
                .expect("drain bounded se stderr");
        }

        if status.is_none() {
            status = child.try_wait().expect("poll bounded se subprocess");
            if status.is_some() {
                exited_at = Some(Instant::now());
            }
        }

        if let Some(exit_status) = status {
            if !stdout_open && !stderr_open {
                return BoundedOutput {
                    output: Output {
                        status: exit_status,
                        stdout: stdout_bytes,
                        stderr: stderr_bytes,
                    },
                    timed_out,
                };
            }
            if exited_at.is_some_and(|exited| exited.elapsed() >= PIPE_CLOSE_GRACE) {
                // The direct child is gone, so an open pipe can only be held by
                // another process. Return the complete direct-child output and
                // flag the bounded run instead of blocking forever on pipe EOF.
                return BoundedOutput {
                    output: Output {
                        status: exit_status,
                        stdout: stdout_bytes,
                        stderr: stderr_bytes,
                    },
                    timed_out: true,
                };
            }
        } else if Instant::now() >= deadline && !timed_out {
            child.kill().expect("terminate timed-out se subprocess");
            timed_out = true;
            kill_deadline = Some(Instant::now() + KILL_GRACE);
        }

        if status.is_none() && kill_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            panic!("terminated se subprocess did not exit within the cleanup deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
fn drain_windows_pipe<T: Read>(
    pipe: &mut T,
    handle: std::os::windows::io::RawHandle,
    output: &mut Vec<u8>,
) -> io::Result<bool> {
    const MAX_DRAIN_PER_POLL: usize = 64 * 1024;
    let mut drained = 0usize;
    while drained < MAX_DRAIN_PER_POLL {
        let Some(available) = windows_pipe_available(handle)? else {
            return Ok(false);
        };
        if available == 0 {
            return Ok(true);
        }
        let mut buffer = [0u8; 8192];
        let wanted = available.min(buffer.len());
        let read = pipe.read(&mut buffer[..wanted])?;
        if read == 0 {
            return Ok(false);
        }
        output.extend_from_slice(&buffer[..read]);
        drained = drained.saturating_add(read);
    }
    Ok(true)
}

#[cfg(windows)]
fn windows_pipe_available(handle: std::os::windows::io::RawHandle) -> io::Result<Option<usize>> {
    use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_DATA};
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    let mut available = 0u32;
    let peeked = unsafe {
        PeekNamedPipe(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    if peeked != 0 {
        return Ok(Some(available as usize));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_NO_DATA as i32 => Ok(None),
        _ => Err(error),
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
