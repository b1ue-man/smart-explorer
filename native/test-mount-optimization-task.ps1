#requires -Version 7.2
param(
    [string]$TestBinary = '',
    [string]$LogRoot = '',
    [string]$BinaryCacheRoot = '',
    [switch]$InstallRuntime,
    [string]$DependencyCacheRoot = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SMART_EXPLORER_REMOTE_RUNNER -ne '1') {
    throw 'This task suite may run only on the configured remote automation runner.'
}
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
    throw 'The mount task suite requires 64-bit PowerShell on the remote Windows runner.'
}
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('mount-optimization-task-' + [guid]::NewGuid().ToString('N'))
}
$LogRoot = [IO.Path]::GetFullPath($LogRoot)
[void][IO.Directory]::CreateDirectory($LogRoot)
$env:SMART_EXPLORER_MOUNT_TASK_LOG_ROOT = $LogRoot
$env:SMART_EXPLORER_MOUNT_CHECKER = Join-Path $nativeRoot 'verify-mount-windows.ps1'
$env:SMART_EXPLORER_MOUNT_VAULT_NODE_SCRIPT = Join-Path $nativeRoot 'mount-vault-node.cjs'
$env:RUST_BACKTRACE = '1'
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '1'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_TERM_COLOR = 'never'
Write-Host "Mount task diagnostics: $LogRoot"

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
                try { $process.Kill($true) } catch { Write-Warning "$Label termination is pending (PID $($process.Id))." }
                [void]$process.WaitForExit(1000)
                throw "$Label exceeded its $Seconds-second deadline (PID $($process.Id))."
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

function Assert-TaskHash {
    param([string]$Path, [string]$Expected)
    if (-not [IO.File]::Exists($Path)) { throw "Required runtime file missing: $Path" }
    $observed = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($observed -cne $Expected) { throw "Runtime identity mismatch: $Path SHA256=$observed" }
    return [pscustomobject]@{ path = $Path; sha256 = $observed }
}

# Exact standalone runtime, not whichever Electron/Node happens to be on PATH.
# SHA-256 from nodejs.org/dist/v24.20.0/SHASUMS256.txt, checked 2026-09-06.
$nodeRoot = if ([string]::IsNullOrWhiteSpace($DependencyCacheRoot)) {
    Join-Path ([IO.Path]::GetTempPath()) ('mount-vault-node-' + [guid]::NewGuid().ToString('N'))
} else { Join-Path $DependencyCacheRoot 'node-v24.20.0-x64' }
[void][IO.Directory]::CreateDirectory($nodeRoot)
$env:SMART_EXPLORER_MOUNT_NODE = Join-Path $nodeRoot 'node.exe'
if (-not [IO.File]::Exists($env:SMART_EXPLORER_MOUNT_NODE)) {
    Invoke-WebRequest -Uri 'https://nodejs.org/dist/v24.20.0/win-x64/node.exe' `
        -OutFile $env:SMART_EXPLORER_MOUNT_NODE -TimeoutSec 300
}
$nodeIdentity = Assert-TaskHash $env:SMART_EXPLORER_MOUNT_NODE `
    '5c976096e04e5c2c1f091938926234cc9fbebfe9787ddd149351b3b0ecc707b5'
$nodeVersion = Invoke-TaskProcess $env:SMART_EXPLORER_MOUNT_NODE @(
    '-p', 'JSON.stringify({node:process.versions.node,uv:process.versions.uv,arch:process.arch,platform:process.platform})'
) 30 'node-runtime'
if ($nodeVersion.Code -ne 0) { throw 'Pinned Node runtime did not start.' }
$nodeDetails = $nodeVersion.Output | ConvertFrom-Json
if ($nodeDetails.node -ne '24.20.0' -or $nodeDetails.uv -ne '1.52.1' -or
    $nodeDetails.arch -ne 'x64' -or $nodeDetails.platform -ne 'win32') {
    throw 'Node/libuv/architecture differs from the selected metadata API contract.'
}
[ordered]@{ node = $nodeDetails; executable = $nodeIdentity;
    windows = [Environment]::OSVersion.VersionString } | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath (Join-Path $LogRoot 'metadata-runtime.json')

# Parse the checked-in PowerShell code before invoking any subprocess. The
# actual checker is then exercised under Windows PowerShell 5.1 by the fixture.
$cacheHelper = Join-Path $nativeRoot 'mount-task-binary-cache.ps1'
foreach ($path in @($PSCommandPath, $cacheHelper, $env:SMART_EXPLORER_MOUNT_CHECKER,
        (Join-Path $nativeRoot 'fetch-dokany-runtime.ps1'),
        (Join-Path $nativeRoot 'prepare-dokany-private.ps1'))) {
    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) { throw "PowerShell syntax error in $path`: $($errors.Message -join '; ')" }
}
. $cacheHelper

$system = [Environment]::SystemDirectory
$dll = Join-Path $system 'dokan2.dll'
$driver = Join-Path $system 'drivers\dokan2.sys'
if ($InstallRuntime -and -not [IO.File]::Exists($dll)) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Installing the pinned runtime requires the administrator-enabled remote runner.'
    }
    $download = Invoke-TaskProcess (Get-Command pwsh.exe -CommandType Application | Select-Object -First 1).Source @(
        '-NoProfile', '-NonInteractive', '-File', (Join-Path $nativeRoot 'fetch-dokany-runtime.ps1')
    ) 300 'runtime-download'
    if ($download.Code -ne 0) { throw "Pinned runtime download failed: $($download.Error)" }
    $msi = $download.Output.Trim()
    if ((Get-AuthenticodeSignature -LiteralPath $msi).Status -ne 'Valid') {
        throw 'Pinned Dokany MSI signature did not validate.'
    }
    $install = Invoke-TaskProcess (Join-Path $system 'msiexec.exe') @(
        '/i', $msi, '/qn', '/norestart', 'ADDLOCAL=DokanDriverFeature', 'INSTALLDEVFILES=0',
        '/l*v', (Join-Path $LogRoot 'dokany-install.log')
    ) 600 'runtime-install'
    if ($install.Code -notin @(0, 3010)) { throw "Dokany MSI failed with exit $($install.Code)." }
}
$runtimeIdentity = @(
    Assert-TaskHash $dll '75600aba867acbdfdb85fcd142b524da769bdc611b855a760aeb0c6e2eaae17a'
    Assert-TaskHash $driver '9549a20e63c22a2b068e635600b65f6b55d8be5122a6623997b1274a1a1f6235'
)
$runtimeIdentity | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'runtime-identity.json')
$startDriver = Invoke-TaskProcess (Join-Path $system 'sc.exe') @('start', 'dokan2') 30 'driver-start'
if ($startDriver.Code -notin @(0, 1056)) { throw "Could not start Dokany driver: $($startDriver.Output)" }


# This follow-up must reuse the approved dependency. Its compiler recipe belongs
# to the preceding task; missing bytes are an input failure, not a rebuild trigger.
$approved = Join-Path $nativeRoot 'assets/dokany-private'
if (-not (Test-Path -LiteralPath $approved -PathType Container)) {
    throw 'Approved private-DLL inputs are required; the vault task never rebuilds them.'
}
$prepared = & (Join-Path $nativeRoot 'prepare-dokany-private.ps1') -ArtifactDirectory $approved -VerifyOnly -RequireApproved
$env:SMART_EXPLORER_DOKANY_DLL_DIR = $prepared.Directory
$env:SMART_EXPLORER_DOKANY_DLL_SHA256 = $prepared.DllSha256
$dependencyEvidence = Join-Path $LogRoot 'private-dokany'
[void][IO.Directory]::CreateDirectory($dependencyEvidence)
foreach ($path in @($prepared.DllPath, $prepared.ManifestPath, $prepared.SourcePackagePath)) {
    [IO.File]::Copy($path, (Join-Path $dependencyEvidence ([IO.Path]::GetFileName($path))), $false)
}
$dependencyManifest = [IO.File]::ReadAllText($prepared.ManifestPath) | ConvertFrom-Json
$dependencyIdentity = "$($prepared.DllSha256):$($dependencyManifest.source_package.sha256)"

$binaryCache = $null
$buildFingerprint = $null
$builtForTask = $false
if ([string]::IsNullOrWhiteSpace($TestBinary) -and -not [string]::IsNullOrWhiteSpace($BinaryCacheRoot)) {
    $binaryCache = Get-MountTaskCacheLocation $BinaryCacheRoot $repoRoot
    $sourceFingerprint = Get-MountTaskBuildFingerprint $repoRoot
    $buildFingerprint = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes("$sourceFingerprint`:$dependencyIdentity"))).ToLowerInvariant()
    $TestBinary = Get-MountTaskCachedBinary $binaryCache $buildFingerprint
    if ([string]::IsNullOrWhiteSpace($TestBinary)) {
        Write-Host 'No matching intact retained fixture; using the single incremental native build.'
    }
}
if ([string]::IsNullOrWhiteSpace($TestBinary)) {
    # This is the only build: one incremental native library test target, not
    # a workspace/all-target/release build. Discover its actual hashed filename.
    $build = Invoke-TaskProcess (Get-Command cargo.exe -CommandType Application | Select-Object -First 1).Source @(
        'test', '--locked', '--lib', '--no-run', '--message-format=json-render-diagnostics'
    ) 5400 'native-incremental'
    if ($build.Code -ne 0) { throw "Affected native target failed to build: $($build.Error)" }
    $executables = @(
        foreach ($line in ($build.Output -split '\r?\n')) {
            if (-not $line.StartsWith('{')) { continue }
            $message = ConvertFrom-Json $line
            if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'smart_explorer' -and
                $message.profile.test -and $null -ne $message.executable) {
                [string]$message.executable
            }
        }
    )
    if ($executables.Count -ne 1) { throw 'Cargo did not identify exactly one native library test executable.' }
    $TestBinary = $executables[0]
    $builtForTask = $true
}
$TestBinary = (Resolve-Path -LiteralPath $TestBinary).Path
if ($builtForTask -and $null -ne $binaryCache) {
    Save-MountTaskCachedBinary $binaryCache $buildFingerprint $TestBinary
}
[IO.File]::WriteAllText((Join-Path $LogRoot 'test-executable.txt'), $TestBinary)

$selected = Invoke-TaskProcess $TestBinary @('mount_vault_task', '--list', '--include-ignored') 30 'selected-cases'
if ($selected.Code -ne 0) { throw 'Could not enumerate the selected task cases.' }
foreach ($required in @(
    'listing_name_and_collision_safety',
    'metadata_authority_and_revision',
    'point_stat_coalescence_and_error_policy',
    'scheduler_uses_more_than_four_available_workers',
    'scheduler_stalled_sibling_allows_other_depth_and_refill',
    'scheduler_deduplicates_keyed_paths_with_boundary_ancestry',
    'scheduler_stop_joins_started_work_without_dispatching_children',
    'scheduler_work_errors_and_panics_join_and_do_not_starve',
    'scheduler_panicking_stop_predicate_wakes_and_joins_workers',
    'scheduler_foreground_refresh_satisfies_selected_revision',
    'actual_volume_metadata_apps_watchers',
    '50001_valid_children_enumerate_and_reuse_snapshot',
    'over_retention_demand_succeeds_and_shares_completed_flight',
    'pressure_sharing_does_not_publish_unnotifiable_image',
    'completed_flights_expire_and_reject_invalidated_revisions',
    'same_flight_listing_failures_share_without_persistent_error_cache',
    'expired_parent_stats_share_listing_and_preserve_point_precedence',
    'expired_parent_listing_denial_falls_back_to_exact_stat',
    'rooted_refresh_crosses_daemon_ttl',
    'framed_tcp_latency_diagnostic',
    'all_frame_variants_keep_exact_protocol_bytes',
    'directory_above_50000_real_entries_roundtrips',
    'directory_minimum_record_guards_reject_malformed_frames',
    'utf8_and_optional_md5_use_encoded_byte_lengths',
    'exact_64_mib_body_and_one_byte_over',
    'framed_writer_short_interrupt_and_error_semantics',
    'framed_reader_interrupt_eof_and_truncation_semantics',
    'daemon_waiters_share_snapshot_and_index_without_retention',
    'daemon_unrelated_snapshot_loads_overlap',
    'daemon_mutation_fences_persistent_and_waiter_authority',
    'daemon_waiters_share_errors_but_do_not_retain_them',
    'more_than_4096_small_directories_remain_reusable',
    'unchanged_parent_preserves_child_and_replacement_invalidates',
    'byte_lru_can_evict_root_without_evicting_speculatively',
    'notification_byte_pressure_retains_baseline_and_retries',
    'bounded_diff_comparison_resumes_after_empty_drain'
)) {
    if (-not $selected.Output.Contains($required)) { throw "Missing task acceptance case: $required" }
}
# One filter, one invocation, one incremental library output throughout the loop.
$run = Invoke-TaskProcess $TestBinary @(
    'mount_vault_task', '--include-ignored', '--nocapture', '--test-threads=1'
) 2400 'mount-vault'
Write-Host $run.Output
if ($run.Code -ne 0) { throw "The Windows vault metadata task failed: $($run.Error)" }
$null = Assert-TaskHash $dll '75600aba867acbdfdb85fcd142b524da769bdc611b855a760aeb0c6e2eaae17a'
$null = Assert-TaskHash $driver '9549a20e63c22a2b068e635600b65f6b55d8be5122a6623997b1274a1a1f6235'
$head = Invoke-TaskProcess (Get-Command git.exe -CommandType Application | Select-Object -First 1).Source @(
    '-C', $repoRoot, 'rev-parse', 'HEAD'
) 30 'approved-candidate'
if ($head.Code -ne 0 -or $head.Output.Trim() -notmatch '^[0-9a-f]{40}$') { throw 'Candidate identity unavailable.' }
$approval = [ordered]@{
    schema = 1
    candidate = $head.Output.Trim()
    dll_sha256 = $prepared.DllSha256
    source_sha256 = $dependencyManifest.source_package.sha256
    test_binary_sha256 = (Get-FileHash -LiteralPath $TestBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    outcome = 'PASS'
}
[IO.File]::WriteAllText((Join-Path $LogRoot 'approval.json'),
    ($approval | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
Write-Host 'Mount behavior and private dependency accepted; official runtime identity remains unchanged.'
