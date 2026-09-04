#requires -Version 7.2
param(
    [string]$TestBinary = '',
    [string]$LogRoot = '',
    [string]$BinaryCacheRoot = '',
    [switch]$InstallRuntime
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$nativeRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $nativeRoot
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
    throw 'The mount task suite requires 64-bit PowerShell on the remote Windows runner.'
}
if ([string]::IsNullOrWhiteSpace($LogRoot)) {
    $LogRoot = Join-Path ([IO.Path]::GetTempPath()) ('mount-batching-task-' + [guid]::NewGuid().ToString('N'))
}
$LogRoot = [IO.Path]::GetFullPath($LogRoot)
[void][IO.Directory]::CreateDirectory($LogRoot)
$env:SMART_EXPLORER_MOUNT_TASK_LOG_ROOT = $LogRoot
$env:SMART_EXPLORER_MOUNT_CHECKER = Join-Path $nativeRoot 'verify-mount-windows.ps1'
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

# Parse the checked-in PowerShell code before invoking any subprocess. The
# actual checker is then exercised under Windows PowerShell 5.1 by the fixture.
$cacheHelper = Join-Path $nativeRoot 'mount-task-binary-cache.ps1'
foreach ($path in @($PSCommandPath, $cacheHelper, $env:SMART_EXPLORER_MOUNT_CHECKER,
        (Join-Path $nativeRoot 'fetch-dokany-runtime.ps1'))) {
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

$binaryCache = $null
$buildFingerprint = $null
$builtForTask = $false
if ([string]::IsNullOrWhiteSpace($TestBinary) -and -not [string]::IsNullOrWhiteSpace($BinaryCacheRoot)) {
    $binaryCache = Get-MountTaskCacheLocation $BinaryCacheRoot $repoRoot
    $buildFingerprint = Get-MountTaskBuildFingerprint $repoRoot
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
$selected = Invoke-TaskProcess $TestBinary @('mount_batching_task', '--list', '--include-ignored') 30 'selected-cases'
if ($selected.Code -ne 0 -or $selected.Output -notmatch 'mount_batching_task_real_driver_navigation_and_checker') {
    throw 'The selected executable does not contain the actual mounted-volume task fixture.'
}
$run = Invoke-TaskProcess $TestBinary @(
    'mount_batching_task', '--include-ignored', '--nocapture', '--test-threads=1'
) 600 'mounted-volume'
Write-Host $run.Output
if ($run.Code -ne 0) { throw "The Windows mount task failed: $($run.Error)" }
Write-Host 'Verified non-batched real-driver navigation, option preservation, checker outcomes and clean teardown.'
