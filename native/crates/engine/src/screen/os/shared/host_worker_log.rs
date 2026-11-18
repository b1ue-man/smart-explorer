//! The screen host log (plan 2026-11-17 P9). A Windows host process is a
//! GUI-subsystem program: its standard error went nowhere, so a capture that
//! fell back („Aufnahme: dxgi failed → Rückfall“) left no reason anywhere.
//! The daemon now reads the standard error of every host process it starts
//! (`host_worker_client`) and writes each line, with time and process id, to
//! `<app data>/logs/screen-host.log` (bounded by rotation, `RotatingLog`:
//! 4 MiB per file, four older files) and, as before, to its own standard
//! error. The daemon adds its own lines about the host processes (start,
//! failed start, an answer that did not come, the end) and, as a viewer,
//! about each failed attempt to open a remote screen
//! (`screen_ipc_retry_plan.rs`). `se doctor` names the file.
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::screen::core::rotating_log::RotatingLog;
use crate::screen::core::worker_wire::WorkerRequest;

/// The name of `request` in a log line (never its content: input stays out).
pub fn request_name(request: &WorkerRequest) -> &'static str {
    match request {
        WorkerRequest::CaptureControl(_) => "CAPTURE_CONTROL",
        WorkerRequest::OpenFamily(_) => "OPEN_FAMILY",
        WorkerRequest::Family(_) => "FAMILY",
        WorkerRequest::Release(_) => "RELEASE_INPUT",
        WorkerRequest::Open(_) => "OPEN",
        WorkerRequest::Produce(_) => "PRODUCE",
        WorkerRequest::Capture { .. } => "CAPTURE",
        WorkerRequest::Input(_) => "INPUT",
        WorkerRequest::SetCaptureActive(_) => "SET_CAPTURE_ACTIVE",
        WorkerRequest::Stream(_) => "STREAM",
        WorkerRequest::Close => "CLOSE",
    }
}

/// The file name below `<app data>/logs`.
pub const LOG_FILE: &str = "screen-host.log";
/// Size of one file before it rotates.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Older files kept beside the current one.
const KEEP: usize = 4;
/// A longer line is written in parts.
const MAX_LINE: u64 = 16 * 1024;

/// Where the screen host log of this account lives.
pub fn log_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("logs")
        .join(LOG_FILE)
}

fn log() -> &'static Mutex<RotatingLog> {
    static LOG: OnceLock<Mutex<RotatingLog>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(RotatingLog::new(log_path(), MAX_BYTES, KEEP)))
}

/// One line of the log: from host process `pid`, or of the daemon (`None`).
pub fn note(pid: Option<u32>, text: &str) {
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    let line = match pid {
        Some(pid) => format!("{stamp} host[{pid}] {text}"),
        None => format!("{stamp} daemon {text}"),
    };
    eprintln!("{line}");
    let written = log()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .append(&line);
    if let Err(error) = written {
        eprintln!("screen host log {}: {error}", log_path().display());
    }
}

/// Reads the standard error of host process `pid` into the log until the
/// process closes it. Without a thread (the daemon has none left) the pipe
/// closes at once; the daemon's own line says so.
pub fn forward(stderr: impl Read + Send + 'static, pid: u32) {
    let started = std::thread::Builder::new()
        .name("screen-host-log".into())
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                match (&mut reader).take(MAX_LINE).read_until(b'\n', &mut buffer) {
                    Ok(0) => return,
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&buffer);
                        let text = text.trim_end_matches(['\r', '\n']);
                        if !text.is_empty() {
                            note(Some(pid), text);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        note(
                            None,
                            &format!("host[{pid}] standard error unreadable: {error}"),
                        );
                        return;
                    }
                }
            }
        });
    if let Err(error) = started {
        note(
            None,
            &format!("host[{pid}] standard error not logged (no thread: {error})"),
        );
    }
}
