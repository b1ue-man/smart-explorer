#requires -Version 7.2
param(
    [string]$TestBinary = '',
    [string]$TestBinarySha256 = '',
    [string]$TestBinarySourceSha = '',
    [string]$LogRoot = '',
    [string]$BinaryCacheRoot = '',
    [switch]$InstallRuntime
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SMART_EXPLORER_REMOTE_RUNNER -ne '1') {
    throw 'This entrypoint runs only on the configured remote automation runner.'
}
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
    throw 'The Windows remote regression task requires 64-bit Windows PowerShell 7.'
}
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('windows-remote-task-' + [guid]::NewGuid().ToString('N'))
}
$LogRoot = [IO.Path]::GetFullPath($LogRoot)
[void][IO.Directory]::CreateDirectory($LogRoot)
$fixtureProfile = Join-Path $LogRoot ('profile-' + [guid]::NewGuid().ToString('N'))
foreach ($directory in @('roaming', 'local')) {
    [void][IO.Directory]::CreateDirectory((Join-Path $fixtureProfile $directory))
}
$env:RUST_BACKTRACE = '1'
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '1'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_TERM_COLOR = 'never'
. (Join-Path $nativeRoot 'windows-remote-task-diagnostics.ps1')

function Invoke-TaskProcess {
    param([string]$File, [string[]]$Arguments, [int]$Seconds, [string]$Label, [switch]$Fixture)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $File
    $start.WorkingDirectory = $nativeRoot
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    if ($Fixture) {
        foreach ($key in @($start.Environment.Keys)) {
            if ($key -match '(?i)(TOKEN|SECRET|PASSWORD|CREDENTIAL)') { [void]$start.Environment.Remove($key) }
        }
        $start.Environment['SMART_EXPLORER_COPY_PASTE_TASK'] = '1'
        $start.Environment['SMART_EXPLORER_WINDOWS_REMOTE_TASK'] = '1'
        $start.Environment['APPDATA'] = Join-Path $fixtureProfile 'roaming'
        $start.Environment['LOCALAPPDATA'] = Join-Path $fixtureProfile 'local'
        [void]$start.Environment.Remove('SE_SHARE_RELAY_URL')
        [void]$start.Environment.Remove('SE_SHARE_RELAY_ONLY')
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $stdout = $null
    $stderr = $null
    $monitor = $null
    $didStart = $false
    $started = [datetime]::Now
    try {
        if (-not $process.Start()) { throw "$Label did not start." }
        $didStart = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if ($Fixture -and $Label -eq 'windows-remote') {
            $monitor = Start-WindowsRemoteTaskDumpMonitor $dumpTool $process.Id $LogRoot
        }
        $timer = [Diagnostics.Stopwatch]::StartNew()
        while (-not ($process.WaitForExit(0) -and $stdout.IsCompleted -and $stderr.IsCompleted)) {
            if ($timer.Elapsed.TotalSeconds -ge $Seconds) {
                try { $process.Kill($true) } catch { Write-Warning "$Label termination pending (PID $($process.Id))." }
                [void]$process.WaitForExit(1000)
                throw "$Label exceeded its $Seconds-second deadline. Inspect runner child processes before retrying."
            }
            Start-Sleep -Milliseconds 200
        }
        Save-WindowsRemoteTaskExit $process.ExitCode $process.Id $started $Label $LogRoot $Fixture.IsPresent
        return [pscustomobject]@{ Code = $process.ExitCode; Output = $stdout.Result; Error = $stderr.Result }
    } finally {
        if ($didStart -and -not $process.HasExited) {
            $process.Kill($true)
            [void]$process.WaitForExit(5000)
        }
        Stop-WindowsRemoteTaskDumpMonitor $monitor $LogRoot
        foreach ($entry in @(@('stdout', $stdout), @('stderr', $stderr))) {
            $task = $entry[1]
            if ($null -ne $task -and $task.Status -eq [Threading.Tasks.TaskStatus]::RanToCompletion) {
                [IO.File]::WriteAllText((Join-Path $LogRoot "$Label.$($entry[0]).log"), [string]$task.Result)
            }
        }
        $process.Dispose()
    }
}

# Use the committed, approved private Dokany payload; never rebuild it here.
# The pinned official driver is installed only on the disposable remote runner.
$system = [Environment]::SystemDirectory
$dll = Join-Path $system 'dokan2.dll'
$driver = Join-Path $system 'drivers/dokan2.sys'
if ($InstallRuntime -and -not [IO.File]::Exists($dll)) {
    $download = Invoke-TaskProcess (Get-Command pwsh.exe -CommandType Application | Select-Object -First 1).Source @(
        '-NoProfile', '-NonInteractive', '-File', (Join-Path $nativeRoot 'fetch-dokany-runtime.ps1')
    ) 300 'runtime-download'
    if ($download.Code -ne 0) { throw "Pinned runtime download failed: $($download.Error)" }
    $msi = $download.Output.Trim()
    if ((Get-AuthenticodeSignature -LiteralPath $msi).Status -ne 'Valid') { throw 'Invalid Dokany MSI signature.' }
    $install = Invoke-TaskProcess (Join-Path $system 'msiexec.exe') @(
        '/i', $msi, '/qn', '/norestart', 'ADDLOCAL=DokanDriverFeature', 'INSTALLDEVFILES=0',
        '/l*v', (Join-Path $LogRoot 'dokany-install.log')
    ) 600 'runtime-install'
    if ($install.Code -notin @(0, 3010)) { throw "Dokany MSI failed: $($install.Code)" }
}
foreach ($item in @(
    @($dll, '75600aba867acbdfdb85fcd142b524da769bdc611b855a760aeb0c6e2eaae17a'),
    @($driver, '9549a20e63c22a2b068e635600b65f6b55d8be5122a6623997b1274a1a1f6235')
)) {
    if (-not [IO.File]::Exists($item[0]) -or
        (Get-FileHash -LiteralPath $item[0] -Algorithm SHA256).Hash -ine $item[1]) {
        throw "Pinned official runtime mismatch: $($item[0])"
    }
}
$startDriver = Invoke-TaskProcess (Join-Path $system 'sc.exe') @('start', 'dokan2') 30 'driver-start'
if ($startDriver.Code -notin @(0, 1056)) { throw "Could not start Dokany: $($startDriver.Output)" }
if (-not [IO.File]::Exists((Join-Path $nativeRoot 'assets/dokany-private/dokan2.dll'))) {
    throw 'The committed approved private Dokany payload is required.'
}

# One affected library build, or a source/hash-bound existing development binary.
$cacheHelper = Join-Path $nativeRoot 'mount-task-binary-cache.ps1'
foreach ($path in @($PSCommandPath, $cacheHelper, (Join-Path $nativeRoot 'windows-remote-task-diagnostics.ps1'))) {
    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) { throw "PowerShell syntax error in $path`: $($errors.Message -join '; ')" }
}
. $cacheHelper
$git = (Get-Command git.exe -CommandType Application | Select-Object -First 1).Source
$head = Invoke-TaskProcess $git @('-C', $repoRoot, 'rev-parse', 'HEAD') 30 'candidate'
if ($head.Code -ne 0 -or $head.Output.Trim() -notmatch '^[0-9a-f]{40}$') { throw 'Candidate identity unavailable.' }
$candidate = $head.Output.Trim()
$cache = $null
$fingerprint = Get-MountTaskBuildFingerprint $repoRoot
$built = $false
if (-not [string]::IsNullOrWhiteSpace($TestBinary)) {
    if ($TestBinarySourceSha -cne $candidate -or $TestBinarySha256 -notmatch '^[a-fA-F0-9]{64}$' -or
        (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash -ine $TestBinarySha256) {
        throw 'An explicitly supplied development binary requires this source SHA and exact SHA-256.'
    }
} elseif (-not [string]::IsNullOrWhiteSpace($BinaryCacheRoot)) {
    $cache = Get-MountTaskCacheLocation $BinaryCacheRoot $repoRoot
    $TestBinary = Get-MountTaskCachedBinary $cache $fingerprint
}
if ([string]::IsNullOrWhiteSpace($TestBinary)) {
    $build = Invoke-TaskProcess (Get-Command cargo.exe -CommandType Application | Select-Object -First 1).Source @(
        'test', '--locked', '--lib', '--no-run', '--message-format=json-render-diagnostics'
    ) 7200 'native-incremental'
    if ($build.Code -ne 0) {
        Write-Host $build.Error
        throw 'Affected library target failed to build; native-incremental.stderr.log contains the diagnostics.'
    }
    $executables = @(
        foreach ($line in ($build.Output -split '\r?\n')) {
            if (-not $line.StartsWith('{')) { continue }
            $message = ConvertFrom-Json $line
            if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'smart_explorer' -and
                $message.profile.test -and $null -ne $message.executable) { [string]$message.executable }
        }
    )
    if ($executables.Count -ne 1) { throw 'Cargo did not identify exactly one library fixture executable.' }
    $TestBinary = $executables[0]
    $built = $true
}
$TestBinary = (Resolve-Path -LiteralPath $TestBinary).Path
if ($built -and $null -ne $cache) { Save-MountTaskCachedBinary $cache $fingerprint $TestBinary }
$selected = Invoke-TaskProcess $TestBinary @('windows_remote_task_', '--list', '--include-ignored') 30 'selected-cases' -Fixture
if ($selected.Code -ne 0) { throw 'Could not enumerate the selected task cases.' }
$requiredCases = @(
    'windows_remote_task_peer_names_are_stable_exact_and_reversible',
    'windows_remote_task_peer_aliases_preserve_extensions_and_staging',
    'windows_remote_task_peer_mount_routes_collisions_and_keeps_permissions',
    'windows_remote_task_actual_drive_mounts_case_colliding_share_roots',
    'windows_remote_task_repair_from_plain_thread_releases_transition',
    'windows_remote_task_handshake_queue_obeys_deadline_and_recovers',
    'windows_remote_task_live_peer_reconnects_using_fresh_lan_evidence',
    'windows_remote_task_panicked_repair_clears_running_and_allows_retry',
    'windows_remote_task_live_routes_follow_lan_renewal_and_keep_pins',
    'windows_remote_task_signal_frames_survive_timeout_and_split_utf8',
    'windows_remote_task_signal_frames_reject_truncation_limits_and_stalls',
    'windows_remote_task_analysis_matches_local_through_gui_worker_and_cache',
    'windows_remote_task_analysis_ipc_cancellation_reaches_active_worker',
    'windows_remote_task_analysis_transport_keeps_partial_states_and_rejects_corruption',
    'windows_remote_task_analysis_codec_streams_large_and_deep_local_results',
    'windows_remote_task_analysis_progress_retains_stalls_and_actual_counters',
    'windows_remote_task_analysis_ui_keeps_evidence_until_worker_completion',
    'windows_remote_task_analysis_combines_export_roots_and_rejects_escape',
    'windows_remote_task_analysis_cancel_closes_queued_peer_request',
    'windows_remote_task_local_scan_keeps_partial_results_and_live_counts',
    'windows_remote_task_analysis_preserves_junction_boundary_and_plain_directories'
)
foreach ($required in $requiredCases) {
    if (-not $selected.Output.Contains("$required`: test")) { throw "Missing task acceptance case: $required" }
}
$dumpKey = Enable-WindowsRemoteTaskDump $TestBinary $LogRoot
try {
    $dumpTool = Get-WindowsRemoteTaskDumpTool $LogRoot
    $result = Invoke-TaskProcess $TestBinary @('windows_remote_task_', '--include-ignored', '--nocapture', '--test-threads=1') 1800 'windows-remote' -Fixture
} finally {
    Remove-Item -LiteralPath $dumpKey -Recurse -Force
}
Write-Host $result.Output
foreach ($line in ($result.Error -split '\r?\n')) {
    if ($line.StartsWith('ANALYSIS_TIMING ')) { Write-Host $line }
}
if ($result.Code -ne 0) {
    Write-Host $result.Error
    throw 'Windows remote task failed; windows-remote.stderr.log contains the diagnostics.'
}
[ordered]@{
    schema = 1
    candidate = $candidate
    outcome = 'PASS'
    test_binary_sha256 = (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    build_inputs_sha256 = $fingerprint
    windows = [Environment]::OSVersion.VersionString
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'approval.json')
