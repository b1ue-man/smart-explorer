//! Local-startup comparison and owned, bounded mounted PowerShell execution.
use super::OptimizationBackend;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct ProcessState { child: Option<Child>, cancelled: bool }

pub(super) struct ScriptProbe { state: Arc<Mutex<ProcessState>> }
pub(super) struct ScriptRunner { state: Arc<Mutex<ProcessState>> }

fn state(shared: &Mutex<ProcessState>) -> MutexGuard<'_, ProcessState> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ScriptProbe {
    pub(super) fn new() -> Self { Self { state: Arc::default() } }
    pub(super) fn runner(&self) -> ScriptRunner { ScriptRunner { state: Arc::clone(&self.state) } }

    pub(super) fn cancel(&self) {
        let mut state = state(&self.state);
        state.cancelled = true;
        if let Some(child) = state.child.as_mut() {
            eprintln!("[mount script] phase=termination-request pid={} result={:?}", child.id(), child.kill());
        }
    }

    // Normal errors reach here after coordinator unmount and worker join.
    // Do not continue to another runtime with an unconfirmed live child.
    pub(super) fn reap(&self) {
        let Some(mut child) = state(&self.state).child.take() else { return; };
        let _ = child.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("[mount script] phase=child-reaped pid={} status={status}", child.id());
                    return;
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
                result => {
                    eprintln!("[mount script] fatal cleanup: pid={} not reaped after unmount: {result:?}; no further runtime", child.id());
                    std::process::abort();
                }
            }
        }
    }
}

impl Drop for ScriptProbe {
    fn drop(&mut self) { self.cancel(); self.reap(); }
}

impl ScriptRunner {
    pub(super) fn check_active(&self) -> io::Result<()> {
        if state(&self.state).cancelled { Err(io::Error::other("script probe cancelled")) } else { Ok(()) }
    }

    pub(super) fn run(&self, root: &Path, output: &Path, private: bool) -> io::Result<()> {
        self.check_active()?;
        let directory = output.parent().ok_or_else(|| io::Error::other("script output has no parent"))?;
        let executable = powershell()?;
        let local_script = directory.join("powershell-startup.ps1");
        let local_output = directory.join("powershell-startup.txt");
        fs::write(&local_script, LOCAL_STARTUP)?;
        // This is a local -File control with the mount still live, not proof
        // that host startup cannot itself inspect registered mounted drives.
        self.run_file(&executable, &local_script, &local_output, directory, private, "local-startup")?;
        assert_eq!(fs::read(&local_output)?, b"local-startup-ok");
        self.run_file(&executable, &root.join("scripts/main.ps1"), output, directory, private, "mounted")?;
        assert_eq!(fs::read(output)?, b"mounted-script-ok");
        Ok(())
    }

    fn run_file(&self, executable: &Path, script: &Path, output: &Path,
        directory: &Path, private: bool, phase: &str) -> io::Result<()> {
        self.check_active()?;
        // A local script does not prevent PowerShell's provider startup from
        // inspecting mounted drive roots. Trace the owned PID in both phases.
        let _trace = super::super::script_callbacks::arm();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(30);
        eprintln!("[mount script] phase={phase}-launch private={private} executable={} script={} cwd={} output={} limit_seconds=30",
            executable.display(), script.display(), directory.display(), output.display());
        // -File is last among host options; following paths are literal script
        // arguments. Both probes use local cwd and inherit output without pipes.
        let child = Command::new(executable)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script).arg(output).current_dir(directory)
            .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn()?;
        let pid = child.id();
        super::super::script_callbacks::set_process(pid);
        {
            let mut state = state(&self.state);
            assert!(state.child.is_none(), "previous PowerShell process was not reaped");
            state.child = Some(child);
        }
        eprintln!("[mount script] phase={phase}-spawned private={private} pid={pid} elapsed_ms={}", started.elapsed().as_millis());
        loop {
            let status = {
                let mut state = state(&self.state);
                if state.cancelled { return Err(io::Error::other("script probe cancelled")); }
                let status = state.child.as_mut().expect("owned PowerShell child").try_wait()?;
                if status.is_some() { state.child.take(); }
                status
            };
            if let Some(status) = status {
                eprintln!("[mount script] phase={phase}-exit private={private} pid={pid} elapsed_ms={} status={status}", started.elapsed().as_millis());
                if !status.success() { return Err(io::Error::other(format!("{phase} PowerShell failed: {status}"))); }
                return Ok(());
            }
            if Instant::now() >= deadline {
                eprintln!("[mount script] phase={phase}-deadline private={private} pid={pid} elapsed_ms={}; child retained for coordinator unmount/reap", started.elapsed().as_millis());
                return Err(io::Error::new(io::ErrorKind::TimedOut, format!("{phase} PowerShell exceeded 30 seconds (pid={pid})")));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

fn powershell() -> io::Result<PathBuf> {
    let system = std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot absent"))?;
    let executable = PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if !executable.is_file() { return Err(io::Error::other("Windows PowerShell unavailable")); }
    Ok(executable)
}

pub(super) fn seed(backend: &OptimizationBackend) {
    backend.put("/scripts/data.txt", b"sibling-value");
    backend.put("/scripts/helper.ps1", HELPER);
    backend.put("/scripts/child.ps1", CHILD);
    backend.put("/scripts/main.ps1", MAIN);
}

const LOCAL_STARTUP: &[u8] = br#"param([string]$ResultPath)
$ErrorActionPreference = 'Stop'
[Console]::Error.WriteLine('[mount script] phase=local-enter pid=' + $PID + ' ps=' + $PSVersionTable.PSVersion + ' edition=' + $PSVersionTable.PSEdition + ' clr=' + $PSVersionTable.CLRVersion + ' cwd=' + [Environment]::CurrentDirectory)
[Console]::Error.Flush()
[Console]::Error.WriteLine('[mount script] phase=local-provider-begin')
[Console]::Error.Flush()
$mountTaskLocation = Get-Location
[Console]::Error.WriteLine('[mount script] phase=local-provider-end location=' + $mountTaskLocation.Path)
[Console]::Error.Flush()
[IO.File]::WriteAllText($ResultPath, 'local-startup-ok')
[Console]::Error.WriteLine('[mount script] phase=local-done')
[Console]::Error.Flush()
"#;

const MAIN: &[u8] = br#"param([string]$ResultPath)
$ErrorActionPreference = 'Stop'
[Console]::Error.WriteLine('[mount script] phase=main-enter pid=' + $PID + ' ps=' + $PSVersionTable.PSVersion + ' edition=' + $PSVersionTable.PSEdition + ' clr=' + $PSVersionTable.CLRVersion + ' root=' + $PSScriptRoot)
[Console]::Error.Flush()
function Trace-MountPhase([string]$Phase) {
    [Console]::Error.WriteLine('[mount script] phase=' + $Phase)
    [Console]::Error.Flush()
}
Trace-MountPhase 'helper-path-begin'
$mountTaskHelper = Join-Path $PSScriptRoot 'helper.ps1'
Trace-MountPhase 'helper-load-begin'
. $mountTaskHelper
Trace-MountPhase 'helper-load-end'
if ((Read-Sibling) -ne 'sibling-value') { throw 'mounted sibling read failed' }
Trace-MountPhase 'helper-value-verified'
Trace-MountPhase 'child-path-begin'
$mountTaskChild = Join-Path $PSScriptRoot 'child.ps1'
Trace-MountPhase 'child-load-begin'
$childResult = & $mountTaskChild
Trace-MountPhase 'child-load-end'
if ($childResult -ne 'child:sibling-value') { throw 'mounted child script failed' }
Trace-MountPhase 'result-write-begin'
[IO.File]::WriteAllText($ResultPath, 'mounted-script-ok')
Trace-MountPhase 'main-done'
"#;

const HELPER: &[u8] = br#"Trace-MountPhase 'helper-enter'
function Read-Sibling {
    Trace-MountPhase 'helper-data-path-begin'
    $mountTaskData = Join-Path $PSScriptRoot 'data.txt'
    Trace-MountPhase 'helper-data-read-begin'
    $mountTaskValue = [IO.File]::ReadAllText($mountTaskData)
    Trace-MountPhase 'helper-data-read-end'
    $mountTaskValue
}
"#;

const CHILD: &[u8] = br#"Trace-MountPhase 'child-enter'
Trace-MountPhase 'child-data-path-begin'
$mountTaskData = Join-Path $PSScriptRoot 'data.txt'
Trace-MountPhase 'child-data-read-begin'
$mountTaskValue = [IO.File]::ReadAllText($mountTaskData)
Trace-MountPhase 'child-data-read-end'
'child:' + $mountTaskValue
"#;
