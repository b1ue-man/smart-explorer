//! Automatic installation of the codec runtimes this program fetches
//! instead of shipping them (2026-11-24, user: „was du anzeigst MUSS
//! funktionieren“, end to end): Cisco's OpenH264 2.6.0 on Windows
//! and Linux x86-64 (`openh264_install.rs`) and the pinned FFmpeg DLLs of
//! the Windows viewer (`ffmpeg_runtime.rs`).
//!
//! - The daemon starts both installers when it starts (`start_at_daemon`):
//!   a background thread each, never blocking the start. OpenH264 keeps
//!   watching its store (a switch back on after `disable`, a removed file);
//!   FFmpeg ends once its copy verifies.
//! - A failed attempt leaves `install.json` beside the runtime (reason,
//!   failures in a row, next attempt). The next attempt follows after 30 s,
//!   doubling per failure up to one hour (`retry_delay`); a success deletes
//!   the record. `install.lock` serializes the installations of all threads
//!   and processes of this user; while one holds it, the state is
//!   „wird installiert“.
//! - A session that needs OpenH264 installs it at once
//!   (`OpenH264Store::session_ready`) unless an attempt failed within the
//!   last 30 s (`session_gate`).
//! - `se screen codec` and the codec page show the state with these texts
//!   (`openh264_state_text`, `runtime_state_text`).
//!
//! Test builds (`cfg(test)`, feature `test-support`) start no download on
//! their own: crate tests share the runner's app data and must not depend on
//! the network.
use std::cell::Cell;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::screen::core::codec_registry::{OpenH264Report, OpenH264State};

/// The record of the last failed attempt, beside the runtime.
pub const RECORD_FILE: &str = "install.json";
/// The lock every installation of the runtime holds.
pub const LOCK_FILE: &str = "install.lock";
const MAX_RECORD_BYTES: u64 = 8192;
/// Pause after the first failed attempt; it doubles per further failure.
pub const RETRY_BASE: Duration = Duration::from_secs(30);
/// Longest pause between two attempts.
pub const RETRY_CEILING: Duration = Duration::from_secs(3600);
/// How often a background installer looks at its record and the switch.
pub const POLL: Duration = Duration::from_secs(30);
/// A session tries again at once unless an attempt failed this recently.
pub const SESSION_GAP: Duration = RETRY_BASE;

/// `install.json`: the last failed attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    /// Failed attempts in a row.
    pub failures: u32,
    /// Unix seconds of the last failed attempt.
    pub failed_at: i64,
    pub reason: String,
    /// Unix seconds of the next attempt of the background installer.
    pub retry_at: i64,
}

/// What started an attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The background installer of the daemon.
    Start,
    /// A session that needs the codec now.
    Session,
    /// The user (`enable`).
    User,
}

/// The pause after `failures` failed attempts in a row (at least one).
pub fn retry_delay(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    RETRY_BASE
        .saturating_mul(1u32 << doublings)
        .min(RETRY_CEILING)
}

/// The record after a failed attempt at `now` with `reason`.
pub fn next_record(previous: Option<&InstallRecord>, reason: &str, now: i64) -> InstallRecord {
    let failures = previous
        .map_or(0, |record| record.failures)
        .saturating_add(1);
    let pause = i64::try_from(retry_delay(failures).as_secs()).unwrap_or(i64::MAX);
    InstallRecord {
        failures,
        failed_at: now,
        reason: reason.to_string(),
        retry_at: now.saturating_add(pause),
    }
}

/// `Err` with the recorded reason when an attempt failed less than
/// `SESSION_GAP` before `now`: a session does not wait for another attempt
/// right after one failed.
pub fn session_gate(record: Option<&InstallRecord>, now: i64) -> Result<(), String> {
    let gap = i64::try_from(SESSION_GAP.as_secs()).unwrap_or(i64::MAX);
    match record {
        Some(record) if now.saturating_sub(record.failed_at) < gap => Err(record.reason.clone()),
        _ => Ok(()),
    }
}

/// The next step of a background installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Attempt,
    Wait(Duration),
    /// Installed: an installer that keeps watching waits `POLL`.
    Installed,
}

/// The step of a background installer: its first round (the start of the
/// daemon) attempts at once, later rounds only once the record's next
/// attempt is due; a runtime the user switched off waits for the switch.
pub fn background_step(
    first: bool,
    opted_out: bool,
    installed: bool,
    record: Option<&InstallRecord>,
    now: i64,
) -> Step {
    if opted_out {
        return Step::Wait(POLL);
    }
    if installed {
        return Step::Installed;
    }
    match record {
        Some(record) if !first && record.retry_at > now => {
            let left = Duration::from_secs(u64::try_from(record.retry_at - now).unwrap_or(0));
            Step::Wait(left.min(POLL))
        }
        _ => Step::Attempt,
    }
}

/// What a background installer finds before a round.
pub struct Seen {
    pub opted_out: bool,
    pub installed: bool,
    pub record: Option<InstallRecord>,
}

/// Runs a background installer: `look(first round)` before each round,
/// `attempt` when `background_step` says so (`at_once`: the first round
/// attempts without waiting for the record's next attempt). Returns after
/// a successful attempt or once installed, unless it keeps `watch`ing.
pub fn run_background(
    watch: bool,
    at_once: bool,
    mut look: impl FnMut(bool) -> Seen,
    mut attempt: impl FnMut() -> Result<(), String>,
) {
    let mut first = true;
    // The record of the last failure when the file could not be written.
    let mut last: Option<InstallRecord> = None;
    loop {
        let seen = look(first);
        let record = seen.record.or_else(|| last.clone());
        let step = background_step(
            first && at_once,
            seen.opted_out,
            seen.installed,
            record.as_ref(),
            unix_now(),
        );
        match step {
            Step::Installed if !watch => return,
            Step::Installed => std::thread::sleep(POLL),
            Step::Wait(pause) => std::thread::sleep(pause),
            Step::Attempt => match attempt() {
                Ok(()) if !watch => return,
                Ok(()) => last = None,
                Err(reason) => last = Some(next_record(record.as_ref(), &reason, unix_now())),
            },
        }
        first = false;
    }
}

pub fn read_record(dir: &Path) -> Option<InstallRecord> {
    let path = dir.join(RECORD_FILE);
    if !path.is_file() {
        return None;
    }
    let text = crate::support_dirs::read_private_text(&path, MAX_RECORD_BYTES).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_record(dir: &Path, record: &InstallRecord) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    crate::support_dirs::write_private_atomic(&dir.join(RECORD_FILE), &json)
}

pub fn clear_record(dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(dir.join(RECORD_FILE)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `install.lock` held for one installation; dropping it releases it.
pub struct InstallLock {
    _file: std::fs::File,
}

impl InstallLock {
    /// Waits until no other thread or process installs into `dir`.
    pub fn acquire(dir: &Path) -> io::Result<Self> {
        let file = crate::support_dirs::open_private_lock(&dir.join(LOCK_FILE))?;
        file.lock()?;
        Ok(Self { _file: file })
    }
}

/// Whether an installation into `dir` runs right now (this or another
/// process); creates nothing.
pub fn held_elsewhere(dir: &Path) -> bool {
    let path = dir.join(LOCK_FILE);
    if !path.is_file() {
        return false;
    }
    let Ok(file) = crate::support_dirs::open_private_lock(&path) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// The cancel rule of a download a session may wait for: it gives up when
/// its first attempt ended before a byte arrived (no connection, no
/// answer), and never once bytes arrived (a broken transfer goes on from
/// the bytes kept, without a limit on the whole transfer). It reads the
/// calls of `http_resume::fetch`: `cancelled` at the start of each attempt,
/// before each read (a read that keeps bytes reports `progress`) and every
/// 100 ms of the pause after an attempt that ended short; without bytes,
/// the third question in a row comes from that pause.
#[derive(Debug, Default)]
pub struct FirstContact {
    bytes: Cell<bool>,
    unanswered: Cell<u32>,
    gave_up: Cell<bool>,
}

impl FirstContact {
    pub fn progress(&self, kept: u64) {
        if kept > 0 {
            self.bytes.set(true);
        }
        self.unanswered.set(0);
    }

    pub fn cancelled(&self) -> bool {
        if self.bytes.get() {
            return false;
        }
        let asked = self.unanswered.get().saturating_add(1);
        self.unanswered.set(asked);
        if asked >= 3 {
            self.gave_up.set(true);
        }
        self.gave_up.get()
    }

    /// Whether the rule ended the download.
    pub fn gave_up(&self) -> bool {
        self.gave_up.get()
    }
}

/// State of a runtime without a switch (FFmpeg of the Windows viewer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Installed,
    Installing,
    /// Not fetched yet and no attempt failed.
    Absent,
    Failed,
}

/// `ffmpeg_runtime` of `se screen codec --json` (Windows).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReport {
    pub state: RuntimeState,
    /// The pinned release.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Unix seconds of the next attempt after a failed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<i64>,
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Local clock time of `at`, with the date when it is not the day of `now`.
pub fn retry_clock(at: i64, now: i64) -> String {
    use chrono::{DateTime, Local, Utc};
    let local = |seconds: i64| {
        DateTime::<Utc>::from_timestamp(seconds, 0).map(|time| time.with_timezone(&Local))
    };
    match (local(at), local(now)) {
        (Some(at), Some(now)) if at.date_naive() == now.date_naive() => {
            at.format("%H:%M:%S").to_string()
        }
        (Some(at), _) => at.format("%d.%m.%Y %H:%M:%S").to_string(),
        (None, _) => at.to_string(),
    }
}

fn failed_text(reason: Option<&str>, retry_at: Option<i64>, now: i64) -> String {
    let reason = reason.unwrap_or("unbekannter Grund");
    match retry_at {
        Some(at) if at > now => {
            format!(
                "Fehler: {reason} – neuer Versuch um {}",
                retry_clock(at, now)
            )
        }
        Some(_) => format!(
            "Fehler: {reason} – neuer Versuch bei der nächsten Sitzung oder im Hintergrunddienst"
        ),
        None => {
            format!("Fehler: {reason} – neuer Versuch beim nächsten Start des Hintergrunddienstes")
        }
    }
}

/// The German state of OpenH264 (`se screen codec`, codec page).
pub fn openh264_state_text(report: &OpenH264Report, now: i64) -> String {
    match report.state {
        OpenH264State::Absent => match &report.reason {
            Some(reason) => format!("nicht angeboten ({reason})"),
            None => "noch nicht installiert – wird beim Start des Hintergrunddienstes \
                     oder für die erste Sitzung mit H.264 installiert"
                .into(),
        },
        OpenH264State::Installing => "wird installiert".into(),
        OpenH264State::Enabled => "installiert".into(),
        OpenH264State::Disabled => "abgeschaltet".into(),
        OpenH264State::Failed => failed_text(report.reason.as_deref(), report.retry_at, now),
    }
}

/// The German state of the FFmpeg runtime (Windows).
pub fn runtime_state_text(report: &RuntimeReport, now: i64) -> String {
    let until = "; bis dahin dekodiert Media Foundation";
    match report.state {
        RuntimeState::Installed => "installiert".into(),
        RuntimeState::Installing => format!("wird installiert{until}"),
        RuntimeState::Absent => format!(
            "noch nicht geladen – wird beim Start des Hintergrunddienstes oder eines \
             Betrachters geladen{until}"
        ),
        RuntimeState::Failed => failed_text(report.reason.as_deref(), report.retry_at, now) + until,
    }
}

/// Starts the background installers of the daemon (`run_loop.rs`); a
/// second call in the same process does nothing.
/// Whether this process may download a codec runtime (OpenH264, FFmpeg):
/// always, unless `SE_CODEC_DOWNLOADS=off` (the program's integration tests
/// set it, so every sandboxed daemon does not fetch 80 MB of FFmpeg).
pub fn downloads_allowed() -> bool {
    std::env::var_os("SE_CODEC_DOWNLOADS").is_none_or(|value| value != "off")
}

pub fn start_at_daemon() {
    #[cfg(not(any(test, feature = "test-support")))]
    if downloads_allowed() {
        start_openh264();
        #[cfg(windows)]
        super::ffmpeg_runtime::ensure_at_daemon();
    }
}

#[cfg(all(
    any(windows, target_os = "linux"),
    not(any(test, feature = "test-support"))
))]
fn start_openh264() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if super::openh264_pins::PIN.is_none() || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let store = super::openh264_install::OpenH264Store::for_user();
    let _ = std::thread::Builder::new()
        .name("openh264-install".into())
        .spawn(move || store.install_in_background());
}

#[cfg(all(
    not(any(windows, target_os = "linux")),
    not(any(test, feature = "test-support"))
))]
fn start_openh264() {}

/// The state of this user's OpenH264 (the codec page polls it).
#[cfg(any(windows, target_os = "linux"))]
pub fn openh264_report() -> OpenH264Report {
    super::openh264_install::OpenH264Store::for_user().report()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn openh264_report() -> OpenH264Report {
    let reason = "OpenH264 is not offered for this platform".to_string();
    OpenH264Report::new(OpenH264State::Absent, Some(reason))
}

/// The FFmpeg runtime of the Windows viewer; `None` elsewhere.
#[cfg(windows)]
pub fn ffmpeg_status() -> Option<RuntimeReport> {
    Some(super::ffmpeg_runtime::status())
}

#[cfg(not(windows))]
pub fn ffmpeg_status() -> Option<RuntimeReport> {
    None
}

#[cfg(test)]
#[path = "codec_runtime_install_tests.rs"]
mod tests;
