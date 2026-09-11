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
    throw 'The clipboard and Share task requires 64-bit Windows PowerShell 7.'
}
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('copy-paste-task-' + [guid]::NewGuid().ToString('N'))
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
        $start.Environment['SMART_EXPLORER_COPY_PASTE_TASK'] = '1'
        $start.Environment['APPDATA'] = Join-Path $fixtureProfile 'roaming'
        $start.Environment['LOCALAPPDATA'] = Join-Path $fixtureProfile 'local'
        [void]$start.Environment.Remove('SE_SHARE_RELAY_URL')
        [void]$start.Environment.Remove('SE_SHARE_RELAY_ONLY')
    }
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

# Reuse the source-bound development-library cache. This installs no mount
# runtime and invokes neither an unrelated suite nor a release-artifact build.
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
$selected = Invoke-TaskProcess $TestBinary @('copy_paste_task_', '--list', '--include-ignored') 30 'selected-cases' -Fixture
if ($selected.Code -ne 0) { throw 'Could not enumerate the selected task cases.' }
$requiredCases = @(
    'copy_paste_task_cancel_and_restart_never_readmit_an_old_result',
    'copy_paste_task_external_sequence_supersedes_a_pending_result',
    'copy_paste_task_generation_exhaustion_cannot_reuse_an_old_stamp',
    'copy_paste_task_gui_clipboard_lifecycle_and_real_share_routing',
    'copy_paste_task_keyboard_blocked_queue_is_consumed_not_replayed',
    'copy_paste_task_pending_admission_requires_current_generation_and_sequence',
    'copy_paste_task_provider_drive_409_reconciles_only_owned_identity',
    'copy_paste_task_provider_drive_concurrent_name_collision_never_adopts_or_replays',
    'copy_paste_task_provider_drive_create_ignores_stale_path_and_pending_ids',
    'copy_paste_task_provider_drive_lost_upload_ack_reconciles_own_content',
    'copy_paste_task_provider_drive_occupied_or_duplicate_stage_never_mutates',
    'copy_paste_task_provider_drive_wrong_metadata_keeps_pending_read_only',
    'copy_paste_task_provider_webdav_conditional_create_and_abort',
    'copy_paste_task_provider_webdav_conflict_kind_survives_repeat_flush',
    'copy_paste_task_provider_webdav_edit_put_remains_unconditional',
    'copy_paste_task_provider_webdav_pending_status_is_not_commit',
    'copy_paste_task_share_cross_export_nested_unicode_and_empty',
    'copy_paste_task_share_operation_whitespace_is_exact',
    'copy_paste_task_share_rename_conflict_and_export_authority',
    'copy_paste_task_share_typed_errors_keep_legacy_compatibility',
    'copy_paste_task_transfer_cancellation_counts_acknowledged_commit',
    'copy_paste_task_transfer_changed_upload_source_is_not_published',
    'copy_paste_task_transfer_download_collisions_and_read_size',
    'copy_paste_task_transfer_filtered_hierarchy_and_invalid_pairs',
    'copy_paste_task_transfer_private_clipboard_bulk_and_honest_failure',
    'copy_paste_task_transfer_same_backend_uses_protected_bridge',
    'copy_paste_task_transfer_stage_failures_preserve_foreign_data',
    'copy_paste_task_transfer_tree_roundtrip_without_final_bulk',
    'copy_paste_task_transfer_upload_collisions_and_explicit_saveback',
    'copy_paste_task_vfs_cache_forwards_private_copy_and_read_contracts',
    'copy_paste_task_vfs_serial_reader_finishes_before_destination_access',
    'copy_paste_task_windows_ansi_hdrop_conversion',
    'copy_paste_task_windows_contention_and_recovery',
    'copy_paste_task_windows_guarded_sequence',
    'copy_paste_task_windows_malformed_dropfiles_and_recovery',
    'copy_paste_task_windows_roundtrip_copy_move',
    'copy_paste_task_windows_undersized_effect_and_recovery'
)
foreach ($required in $requiredCases) {
    if (-not $selected.Output.Contains("$required`: test")) { throw "Missing task acceptance case: $required" }
}
$result = Invoke-TaskProcess $TestBinary @('copy_paste_task_', '--include-ignored', '--nocapture', '--test-threads=1') 1800 'copy-paste' -Fixture
Write-Host $result.Output
if ($result.Code -ne 0) { throw "Clipboard and cross-remote task failed: $($result.Error)" }
[ordered]@{
    schema = 1
    candidate = $candidate
    outcome = 'PASS'
    test_binary_sha256 = (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    build_inputs_sha256 = $fingerprint
    windows = [Environment]::OSVersion.VersionString
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'approval.json')
