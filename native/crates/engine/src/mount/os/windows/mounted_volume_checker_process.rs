//! Process lifetime and ready/continue handoff for the synthetic fixture only.

use super::{io_context, path_context, MountedFixture};
use crate::mount::MountId;
use base64::Engine;
use std::{
    fs::{self, File},
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{CreateEventW, SetEvent, WaitForSingleObject},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum CheckerMode { Healthy, Stalled }

pub(super) fn run_checker(
    fixture: &MountedFixture, checker: &Path, mode: CheckerMode,
) -> io::Result<(ExitStatus, serde_json::Value)> {
    let (name, timeout, limit) = match mode {
        CheckerMode::Healthy => ("pass", 90, Duration::from_secs(100)),
        CheckerMode::Stalled => ("timeout", 20, Duration::from_secs(30)),
    };
    // One monotonic parent deadline covers setup, host startup, readiness,
    // checker execution and report collection; readiness never resets it.
    let deadline = Instant::now() + limit;
    let wall_deadline = SystemTime::now() + limit;
    let report = fixture.logs.join(format!("{name}.json"));
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other(format!("checker {name}: SystemRoot is required")))?;
    let powershell = PathBuf::from(system_root)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let stdout_path = fixture.logs.join(format!("{name}.stdout"));
    let stderr_path = fixture.logs.join(format!("{name}.stderr"));
    let stdout = File::create(&stdout_path)
        .map_err(|error| path_context("checker: create stdout capture", &stdout_path, error))?;
    let stderr = File::create(&stderr_path)
        .map_err(|error| path_context("checker: create stderr capture", &stderr_path, error))?;
    let drive = fixture.drive()?.get().to_string();
    let mut command = Command::new(&powershell);
    command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]);
    let handshake = match mode {
        CheckerMode::Healthy => {
            // Preserve the standalone user's -File/-ReportPath invocation.
            command.arg("-File").arg(checker).arg("-Drive").arg(&drive)
                .arg("-ReportPath").arg(&report)
                .arg("-TimeoutSeconds").arg(timeout.to_string());
            None
        }
        CheckerMode::Stalled => {
            let handshake = Handshake::new()?;
            let until = wall_deadline.duration_since(UNIX_EPOCH)
                .map_err(|error| io::Error::other(format!("checker timeout: wall deadline: {error}")))?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(
                BOOTSTRAP.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(),
            );
            command.arg("-EncodedCommand").arg(encoded)
                .env("SMART_EXPLORER_CHECKER_READY", &handshake.ready.name)
                .env("SMART_EXPLORER_CHECKER_CONTINUE", &handshake.continue_event.name)
                .env("SMART_EXPLORER_CHECKER_SCRIPT", checker)
                .env("SMART_EXPLORER_CHECKER_DRIVE", &drive)
                .env("SMART_EXPLORER_CHECKER_REPORT", &report)
                .env("SMART_EXPLORER_CHECKER_DEADLINE_MS", until.as_millis().to_string());
            Some(handshake)
        }
    };
    eprintln!("[mount fixture] checker {name} launch: mode={mode:?}, drive={drive}, timeout={timeout}s, executable={}, script={}, cwd={}, report={}",
        powershell.display(), checker.display(), fixture.temporary.path().display(), report.display());
    let mut child = CapturedChild(command.current_dir(fixture.temporary.path()).stdin(Stdio::null())
        .stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr)).spawn()
        .map_err(|error| io_context(format!(
            "checker {name}: spawn executable={} script={} cwd={}",
            powershell.display(), checker.display(), fixture.temporary.path().display(),
        ), error))?);
    let release = if let Some(handshake) = &handshake {
        handshake.wait_ready(&mut child, deadline)?;
        let release = fixture.backend.arm_stall();
        fixture.storage.context.engine.invalidate_metadata("/", true);
        eprintln!("[mount fixture] timeout branch: initialized host ready; backend stall armed and root cache invalidated");
        handshake.continue_event.signal()?;
        Some(release)
    } else { None };
    let result = wait_for_report(&mut child, &report, name, deadline, limit);
    // Also runs on report, process and parent-deadline errors before child
    // cleanup or fixture unmount. Early signal errors drop the local guard.
    drop(release);
    if matches!(mode, CheckerMode::Stalled) {
        eprintln!("[mount fixture] timeout branch: backend stall released");
    }
    result
}

fn wait_for_report(
    child: &mut CapturedChild, report: &Path, name: &str, deadline: Instant, limit: Duration,
) -> io::Result<(ExitStatus, serde_json::Value)> {
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!(
                "checker {name}: parent deadline after {limit:?}, pid={}", child.0.id(),
            )));
        }
        if let Some(status) = child.0.try_wait()
            .map_err(|error| io_context(format!("checker {name}: poll process exit"), error))? {
            eprintln!("[mount fixture] checker {name} exited: {status}");
            let bytes = fs::read(report)
                .map_err(|error| path_context("checker: read JSON report", report, error))?;
            let parsed: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::other(format!(
                    "checker {name}: parse JSON report {}: {error}", report.display(),
                )))?;
            eprintln!("[mount fixture] checker {name} result: outcome={}", parsed["outcome"]);
            return Ok((status, parsed));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

struct Handshake { ready: NamedEvent, continue_event: NamedEvent }

impl Handshake {
    fn new() -> io::Result<Self> {
        let id = MountId::new_random()
            .map_err(|error| io_context("checker timeout: generate event ID", error))?;
        let prefix = format!("Local\\SmartExplorerMountChecker-{}", id.as_str());
        Ok(Self {
            ready: NamedEvent::new(format!("{prefix}-ready"))?,
            continue_event: NamedEvent::new(format!("{prefix}-continue"))?,
        })
    }

    fn wait_ready(&self, child: &mut CapturedChild, deadline: Instant) -> io::Result<()> {
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut,
                    "checker timeout: host readiness exhausted the parent deadline"));
            }
            if self.ready.is_signaled()? { return Ok(()); }
            if let Some(status) = child.0.try_wait()
                .map_err(|error| io_context("checker timeout: poll unready host", error))? {
                return Err(io::Error::other(format!(
                    "checker timeout: host exited before readiness: {status}",
                )));
            }
            thread::sleep(Duration::from_millis(40));
        }
    }
}

struct NamedEvent { name: String, handle: OwnedHandle }

impl NamedEvent {
    fn new(name: String) -> io::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // NULL security attributes use the creator's default DACL and make
        // handles non-inheritable. Local\ confines names to this user session.
        let raw = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide.as_ptr()) };
        let error = unsafe { GetLastError() };
        if raw.is_null() {
            return Err(io_context(format!("checker handshake: CreateEventW {name}"),
                io::Error::from_raw_os_error(error as i32)));
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if error == ERROR_ALREADY_EXISTS {
            return Err(io_context(format!("checker handshake: reject existing event {name}"),
                io::Error::from_raw_os_error(error as i32)));
        }
        Ok(Self { name, handle })
    }

    fn is_signaled(&self) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => {
                let error = unsafe { GetLastError() };
                Err(io_context(format!("checker handshake: wait {}", self.name),
                    io::Error::from_raw_os_error(error as i32)))
            }
            result => Err(io::Error::other(format!(
                "checker handshake: unexpected wait result 0x{result:08x} for {}", self.name,
            ))),
        }
    }

    fn signal(&self) -> io::Result<()> {
        if unsafe { SetEvent(self.handle.as_raw_handle()) } == 0 {
            let error = unsafe { GetLastError() };
            return Err(io_context(format!("checker handshake: SetEvent {}", self.name),
                io::Error::from_raw_os_error(error as i32)));
        }
        Ok(())
    }
}

struct CapturedChild(Child);

impl Drop for CapturedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            // A kernel-blocked child need not terminate immediately. Never
            // wait indefinitely here; the released fixture subsequently closes.
            let _ = self.0.try_wait();
        }
    }
}

// Paths and event names are data in child-only environment variables, never
// interpolated into executable PowerShell. Only the same checker emits JSON.
const BOOTSTRAP: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$mountTaskReady = $null
$mountTaskContinue = $null
try {
    $null = Get-Location
    $mountTaskReady = [Threading.EventWaitHandle]::OpenExisting($env:SMART_EXPLORER_CHECKER_READY)
    $mountTaskContinue = [Threading.EventWaitHandle]::OpenExisting($env:SMART_EXPLORER_CHECKER_CONTINUE)
    [Console]::Error.WriteLine('[mount fixture bootstrap] filesystem provider initialized; ready')
    [Console]::Error.Flush()
    if (-not $mountTaskReady.Set()) { throw 'ready_signal_failed' }
    $mountTaskRemaining = [long]$env:SMART_EXPLORER_CHECKER_DEADLINE_MS - [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $mountTaskWait = [int][Math]::Max(0, [Math]::Min(30000, $mountTaskRemaining))
    if ($mountTaskWait -eq 0 -or -not $mountTaskContinue.WaitOne($mountTaskWait)) { throw 'continue_deadline' }
    [Console]::Error.WriteLine('[mount fixture bootstrap] continue received; invoking checker')
    [Console]::Error.Flush()
    $mountTaskOutput = & $env:SMART_EXPLORER_CHECKER_SCRIPT -Drive $env:SMART_EXPLORER_CHECKER_DRIVE -TimeoutSeconds 20
    $mountTaskCode = $LASTEXITCODE
    $mountTaskJson = [string]($mountTaskOutput -join [Environment]::NewLine)
    [Console]::Out.WriteLine($mountTaskJson)
    [Console]::Out.Flush()
    [IO.File]::WriteAllText($env:SMART_EXPLORER_CHECKER_REPORT, $mountTaskJson, [Text.UTF8Encoding]::new($false))
    exit $mountTaskCode
} catch {
    [Console]::Error.WriteLine('[mount fixture bootstrap] failed: ' + $_.Exception.GetType().FullName)
    [Console]::Error.Flush()
    exit 3
} finally {
    if ($null -ne $mountTaskContinue) { $mountTaskContinue.Dispose() }
    if ($null -ne $mountTaskReady) { $mountTaskReady.Dispose() }
}
"#;
