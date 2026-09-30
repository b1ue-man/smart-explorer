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
    throw 'The mount-recovery-cache task requires 64-bit Windows PowerShell 7.'
}
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('mount-recovery-cache-task-' + [guid]::NewGuid().ToString('N'))
}
$LogRoot = [IO.Path]::GetFullPath($LogRoot)
[void][IO.Directory]::CreateDirectory($LogRoot)
$env:SMART_EXPLORER_MOUNT_RECOVERY_CACHE_TASK = '1'
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
$filters = @(
    'mount_recovery_cache_task',
    'mount_optimization_task_reopen_reuses_contents_but_revalidates',
    'mount_optimization_task_short_and_overlong_streams_never_enter_cache',
    'mount_optimization_task_pins_bridge_close_and_reopen_without_double_disposal',
    'mount_optimization_task_failed_upload_and_restart_preserve_dirty_bytes',
    'mount_optimization_task_conflict_and_delete_pending_are_not_disposable',
    'mount_optimization_task_space_reclaims_clean_but_never_unsaved_data',
    'mount_optimization_task_pending_growth_is_reserved_between_callers',
    'mount_optimization_task_idle_byte_limits_lru_zero_and_generation_age',
    'mount_optimization_task_lazy_destination_survives_atomic_replace',
    'mount_vault_task_retirement_preserves_dirty_and_recovery_referenced_spools'
)
$selected = Invoke-TaskProcess $TestBinary ($filters + @('--list')) 60 'selected-cases'
if ($selected.Code -ne 0) { throw 'Could not enumerate task acceptance cases.' }
foreach ($required in @(
    'mount_recovery_cache_task_missing_payload_keeps_siblings_and_restored_retry',
    'mount_recovery_cache_task_absent_or_invalid_journal_preserves_payloads',
    'mount_recovery_cache_task_incomplete_audit_cannot_clean_from_drop',
    'mount_recovery_cache_task_failed_eviction_does_not_starve_other_clean_files',
    'mount_recovery_cache_task_large_read_offsets_eof_and_no_disk_spool',
    'mount_recovery_cache_task_range_rejects_short_or_changed_data_and_retries',
    'mount_recovery_cache_task_fallback_and_rw_keep_whole_file_semantics',
    'mount_recovery_cache_task_range_crosses_rooted_tcp_proxy_without_ids_or_escape',
    'mount_recovery_cache_task_proxy_fallback_keeps_unseekable_backends_readable',
    'mount_recovery_cache_task_windows_locked_clean_file_is_accounted_and_retryable',
    'mount_recovery_cache_task_startup_shows_details_and_later_failures_still_alert',
    'mount_optimization_task_reopen_reuses_contents_but_revalidates',
    'mount_optimization_task_short_and_overlong_streams_never_enter_cache',
    'mount_optimization_task_pins_bridge_close_and_reopen_without_double_disposal',
    'mount_optimization_task_failed_upload_and_restart_preserve_dirty_bytes',
    'mount_optimization_task_conflict_and_delete_pending_are_not_disposable',
    'mount_optimization_task_space_reclaims_clean_but_never_unsaved_data',
    'mount_optimization_task_pending_growth_is_reserved_between_callers',
    'mount_optimization_task_idle_byte_limits_lru_zero_and_generation_age',
    'mount_optimization_task_lazy_destination_survives_atomic_replace',
    'mount_vault_task_retirement_preserves_dirty_and_recovery_referenced_spools'
)) {
    if (-not $selected.Output.Contains($required)) { throw "Missing task acceptance case: $required" }
}
$result = Invoke-TaskProcess $TestBinary ($filters + @('--nocapture', '--test-threads=1')) 1800 'mount-recovery-cache'
Write-Host $result.Output
if ($result.Code -ne 0) { throw "Mount recovery/cache task failed: $($result.Error)" }
[ordered]@{
    schema = 1
    candidate = $candidate
    outcome = 'PASS'
    test_binary_sha256 = (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    build_inputs_sha256 = $fingerprint
    windows = [Environment]::OSVersion.VersionString
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'approval.json')
