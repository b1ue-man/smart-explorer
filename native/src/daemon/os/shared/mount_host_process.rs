use std::io;
use std::process::ExitStatus;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use super::mount_job::{MountHostChild, MountHostJob};

#[cfg(not(windows))]
use std::process::Child as MountHostChild;

#[cfg(any(windows, test))]
pub(super) const MOUNT_HOST_STDERR_LIMIT: usize = 16 * 1024;
const MOUNT_HOST_DETAIL_LIMIT: usize = 12 * 1024;
const STDERR_DRAIN_GRACE: Duration = Duration::from_millis(100);
const TRUNCATED_MARKER: &str = "[fruehere Host-Ausgabe gekuerzt]";
const LAUNCHER_PREFIXES: [&str; 2] = [
    "smart-explorer: internal mount host failed:",
    "se: internal mount host failed:",
];

pub(super) struct MountHostProcess {
    child: MountHostChild,
    #[cfg(windows)]
    _job: MountHostJob,
    stderr_capture: Arc<Mutex<CapturedStderr>>,
    stderr_done: Receiver<()>,
}

pub(super) struct MountHostExit {
    pub(super) status: ExitStatus,
    pub(super) detail: Option<String>,
}

impl MountHostProcess {
    #[cfg(windows)]
    pub(super) fn capture_piped_stderr(
        child: MountHostChild,
        stderr: std::fs::File,
        job: MountHostJob,
    ) -> io::Result<Self> {
        let mut child = child;
        let stderr_capture = Arc::new(Mutex::new(CapturedStderr::default()));
        let thread_capture = Arc::clone(&stderr_capture);
        let (done_send, stderr_done) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::Builder::new()
            .name("mount-host-stderr".into())
            .spawn(move || {
                drain_stderr(stderr, &thread_capture);
                let _ = done_send.send(());
            });
        if let Err(error) = reader {
            terminate_incomplete_child(&mut child);
            return Err(error);
        }
        Ok(Self {
            child,
            _job: job,
            stderr_capture,
            stderr_done,
        })
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<MountHostExit>> {
        match self.child.try_wait()? {
            Some(status) => Ok(Some(self.complete_exit(status))),
            None => Ok(None),
        }
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub(super) fn wait(&mut self) -> io::Result<MountHostExit> {
        let status = self.child.wait()?;
        Ok(self.complete_exit(status))
    }

    fn complete_exit(&mut self, status: ExitStatus) -> MountHostExit {
        // A descendant can accidentally inherit stderr. Never join its reader
        // while the manager lock is held; use the bytes available after a
        // short bounded drain grace and let the detached reader finish later.
        let _ = self.stderr_done.recv_timeout(STDERR_DRAIN_GRACE);
        let captured = self
            .stderr_capture
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_default();
        MountHostExit {
            status,
            detail: normalize_stderr(&captured.bytes, captured.truncated),
        }
    }
}

#[derive(Clone, Default)]
struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedStderr {
    #[cfg(any(windows, test))]
    fn push_tail(&mut self, bytes: &[u8]) {
        if bytes.len() >= MOUNT_HOST_STDERR_LIMIT {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - MOUNT_HOST_STDERR_LIMIT..]);
            self.truncated = true;
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(MOUNT_HOST_STDERR_LIMIT);
        if overflow > 0 {
            self.bytes.drain(..overflow);
            self.truncated = true;
        }
        self.bytes.extend_from_slice(bytes);
    }
}

#[cfg(windows)]
fn drain_stderr(mut stderr: std::fs::File, capture: &Arc<Mutex<CapturedStderr>>) {
    use std::io::Read;

    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                let Ok(mut captured) = capture.lock() else {
                    break;
                };
                captured.push_tail(&chunk[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn normalize_stderr(bytes: &[u8], capture_truncated: bool) -> Option<String> {
    let raw = String::from_utf8_lossy(bytes);
    let mut compact = String::new();
    let mut separator_pending = false;
    for character in raw.chars() {
        if character.is_whitespace() || character.is_control() {
            separator_pending |= !compact.is_empty();
            continue;
        }
        if separator_pending && !compact.is_empty() {
            compact.push(' ');
        }
        separator_pending = false;
        compact.push(character);
    }

    let launcher = LAUNCHER_PREFIXES
        .iter()
        .filter_map(|prefix| compact.rfind(prefix).map(|index| (index, *prefix)))
        .max_by_key(|(index, _)| *index);
    let authoritative = launcher.is_some();
    let selected = launcher
        .map(|(index, prefix)| &compact[index + prefix.len()..])
        .unwrap_or(&compact)
        .trim();
    if selected.is_empty() {
        return None;
    }

    let mut normalized = selected.to_string();
    let display_truncated = normalized.len() > MOUNT_HOST_DETAIL_LIMIT;
    let truncated = display_truncated || (capture_truncated && !authoritative);
    if truncated {
        let content_limit = MOUNT_HOST_DETAIL_LIMIT.saturating_sub(TRUNCATED_MARKER.len() + 1);
        retain_utf8_tail(&mut normalized, content_limit);
        normalized.insert_str(0, " ");
        normalized.insert_str(0, TRUNCATED_MARKER);
    }
    Some(normalized)
}

fn retain_utf8_tail(value: &mut String, limit: usize) {
    if value.len() <= limit {
        return;
    }
    let mut boundary = value.len() - limit;
    while boundary < value.len() && !value.is_char_boundary(boundary) {
        boundary += 1;
    }
    value.drain(..boundary);
}

#[cfg(windows)]
fn terminate_incomplete_child(child: &mut MountHostChild) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
#[path = "mount_host_process_task_tests.rs"]
mod task_tests;
