#requires -Version 7.2
param(
    [string]$TestBinary = '',
    [string]$TestBinarySha256 = '',
    [string]$TestBinarySourceSha = '',
    [string]$LogRoot = '',
    [string]$BinaryCacheRoot = ''
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SMART_EXPLORER_REMOTE_RUNNER -ne '1') {
    throw 'This entrypoint runs only on the configured remote automation runner.'
}
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
    throw 'The storage-access task requires 64-bit Windows PowerShell 7.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'The remote ACL fixture requires an administrator-enabled runner with SeBackupPrivilege.'
}
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('analytics-access-task-' + [guid]::NewGuid().ToString('N'))
}
$LogRoot = [IO.Path]::GetFullPath($LogRoot)
[void][IO.Directory]::CreateDirectory($LogRoot)
$env:SMART_EXPLORER_ANALYTICS_TASK = '1'
$env:RUST_BACKTRACE = '1'
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '1'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_TERM_COLOR = 'never'

function Invoke-TaskProcess {
    param([string]$File, [string[]]$Arguments, [int]$Seconds, [string]$Label)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $File
    $start.WorkingDirectory = $nativeRoot
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $stdout = $null
    $stderr = $null
    try {
        if (-not $process.Start()) { throw "$Label did not start." }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $timer = [Diagnostics.Stopwatch]::StartNew()
        while (-not ($process.WaitForExit(0) -and $stdout.IsCompleted -and $stderr.IsCompleted)) {
            if ($timer.Elapsed.TotalSeconds -ge $Seconds) {
                try { $process.Kill($true) } catch { Write-Warning "$Label termination pending (PID $($process.Id))." }
                [void]$process.WaitForExit(1000)
                throw "$Label exceeded its $Seconds-second deadline. Inspect runner child processes before retrying."
            }
            Start-Sleep -Milliseconds 200
        }
        return [pscustomobject]@{ Code = $process.ExitCode; Output = $stdout.Result; Error = $stderr.Result }
    } finally {
        foreach ($entry in @(@('stdout', $stdout), @('stderr', $stderr))) {
            $task = $entry[1]
            if ($null -ne $task -and $task.Status -eq [Threading.Tasks.TaskStatus]::RanToCompletion) {
                [IO.File]::WriteAllText((Join-Path $LogRoot "$Label.$($entry[0]).log"), [string]$task.Result)
            }
        }
        $process.Dispose()
    }
}

# Reuse the established source-bound library-binary cache and process contract;
# this does not execute a mount suite or install any filesystem runtime.
$cacheHelper = Join-Path $nativeRoot 'mount-task-binary-cache.ps1'
foreach ($path in @($PSCommandPath, $cacheHelper)) {
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
$privileges = Invoke-TaskProcess (Join-Path ([Environment]::SystemDirectory) 'whoami.exe') @('/priv', '/fo', 'csv') 30 'runner-privileges'
if ($privileges.Code -ne 0 -or -not $privileges.Output.Contains('SeBackupPrivilege')) {
    throw 'Runner token is missing SeBackupPrivilege; resolve runner setup before compiling.'
}

$cache = $null
$fingerprint = Get-MountTaskBuildFingerprint $repoRoot
$built = $false
if (-not [string]::IsNullOrWhiteSpace($TestBinary)) {
    if ($TestBinarySourceSha -cne $candidate -or $TestBinarySha256 -notmatch '^[a-fA-F0-9]{64}$' -or
        (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash -ine $TestBinarySha256) {
        throw 'An explicitly supplied development binary requires this source SHA and its exact SHA-256.'
    }
} elseif (-not [string]::IsNullOrWhiteSpace($BinaryCacheRoot)) {
    $cache = Get-MountTaskCacheLocation $BinaryCacheRoot $repoRoot
    $TestBinary = Get-MountTaskCachedBinary $cache $fingerprint
}
if ([string]::IsNullOrWhiteSpace($TestBinary)) {
    $build = Invoke-TaskProcess (Get-Command cargo.exe -CommandType Application | Select-Object -First 1).Source @(
        'test', '--locked', '--lib', '--no-run', '--message-format=json-render-diagnostics'
    ) 5400 'native-incremental'
    if ($build.Code -ne 0) { throw "Affected library target failed to build: $($build.Error)" }
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
$selected = Invoke-TaskProcess $TestBinary @('analytics_access_task', '--list', '--include-ignored') 30 'selected-cases'
if ($selected.Code -ne 0) { throw 'Could not enumerate the selected task cases.' }
foreach ($required in @(
    'sizes_and_counts', 'missing_local_root_is_failed_not_empty_success',
    'backend_child_error_is_partial_and_root_error_is_failed', 'existing_budget_stops_honestly',
    'diagnostics_keep_denial_identity_when_report_is_full', 'cancellation_never_becomes_partial_success',
    'exact_startup_admission', 'report_lists_each_retained_path_and_omissions',
    'new_scan_resets_prompt_without_canceling_consented_launch',
    'remote_access_never_requests_local_privileges',
    'sdk_layout_and_native_names', 'directory_decoder_rejects_malformed_records',
    'full_record_fallback_and_reparse_classification', 'uac_arguments_and_canceled_consent',
    'startup_binds_current_image_hash', 'real_denied_directory_locked_files_and_unchanged_acl',
    'restricted_identity_never_falls_back_to_process_authority', 'redirect_children_are_traversal_boundaries'
)) {
    if (-not $selected.Output.Contains("analytics_access_task_$required")) { throw "Missing task acceptance case: $required" }
}
$result = Invoke-TaskProcess $TestBinary @('analytics_access_task', '--include-ignored', '--nocapture', '--test-threads=1') 1200 'analytics-access'
Write-Host $result.Output
if ($result.Code -ne 0) { throw "Storage-access task failed: $($result.Error)" }
[ordered]@{
    schema = 1
    candidate = $candidate
    outcome = 'PASS'
    test_binary_sha256 = (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    build_inputs_sha256 = $fingerprint
    windows = [Environment]::OSVersion.VersionString
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'approval.json')
