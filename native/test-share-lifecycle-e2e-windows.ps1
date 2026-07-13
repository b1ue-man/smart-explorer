[CmdletBinding()]
param(
    [string]$SeBinary = (Join-Path $PSScriptRoot "target\debug\se.exe"),
    [string]$ShareServerBinary = (Join-Path $PSScriptRoot "..\share-server\target\debug\se-share-server.exe")
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

if ($null -eq ("SmartExplorerE2E.NativeProcess" -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace SmartExplorerE2E
{
    public static class NativeProcess
    {
        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool GetExitCodeProcess(IntPtr process, out UInt32 exitCode);
    }
}
'@
}

$script:SeBinary = [System.IO.Path]::GetFullPath($SeBinary)
$script:ShareServerBinary = [System.IO.Path]::GetFullPath($ShareServerBinary)
$script:RelayUrl = $null
$script:TestNamespacePrefix = "share_" + [Guid]::NewGuid().ToString("N").Substring(0, 24)

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Success {
    param(
        [psobject]$Result,
        [string]$Context
    )

    if ($Result.ExitCode -ne 0) {
        throw "$Context failed with exit code $($Result.ExitCode).`nstdout:`n$($Result.Stdout)`nstderr:`n$($Result.Stderr)"
    }
}

function Convert-CommandJson {
    param(
        [psobject]$Result,
        [string]$Context
    )

    Assert-Success $Result $Context
    try {
        return ($Result.Stdout | ConvertFrom-Json -ErrorAction Stop)
    }
    catch {
        throw "$Context returned invalid JSON.`nstdout:`n$($Result.Stdout)`nstderr:`n$($Result.Stderr)"
    }
}

function New-IsolatedClient {
    param([string]$Root)

    foreach ($name in @("home", "data", "config", "runtime", "roaming", "local")) {
        New-Item -ItemType Directory -Path (Join-Path $Root $name) -Force | Out-Null
    }
}

function Invoke-Client {
    param(
        [string]$ClientRoot,
        [string[]]$Arguments,
        [int]$TimeoutSeconds = 90
    )

    $commandId = [Guid]::NewGuid().ToString("N")
    $stdoutPath = Join-Path $ClientRoot "command-$commandId.stdout"
    $stderrPath = Join-Path $ClientRoot "command-$commandId.stderr"
    $invocation = Start-ClientProcess -ClientRoot $ClientRoot -Arguments $Arguments `
        -StdoutPath $stdoutPath -StderrPath $stderrPath
    return Wait-ClientProcess $invocation $TimeoutSeconds "se $($Arguments -join ' ')"
}

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') {
        return $Value
    }

    # Inverse of CommandLineToArgvW for one argv value. Start-Process accepts
    # only a command-line string on Windows PowerShell 5.1, so preserve literal
    # backslashes and quotes instead of relying on its lossy array re-quoting.
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $slashes++
            continue
        }
        if ($character -eq '"') {
            if ($slashes -gt 0) {
                [void]$builder.Append((('\' * ($slashes * 2)) -join ''))
            }
            [void]$builder.Append('\"')
            $slashes = 0
            continue
        }
        if ($slashes -gt 0) {
            [void]$builder.Append((('\' * $slashes) -join ''))
            $slashes = 0
        }
        [void]$builder.Append($character)
    }
    if ($slashes -gt 0) {
        [void]$builder.Append((('\' * ($slashes * 2)) -join ''))
    }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Start-ClientProcess {
    param(
        [string]$ClientRoot,
        [string[]]$Arguments,
        [string]$StdoutPath,
        [string]$StderrPath
    )

    $clientName = [System.IO.Path]::GetFileName($ClientRoot)
    $environment = @{
        HOME = (Join-Path $ClientRoot "home")
        USERPROFILE = (Join-Path $ClientRoot "home")
        XDG_DATA_HOME = (Join-Path $ClientRoot "data")
        XDG_CONFIG_HOME = (Join-Path $ClientRoot "config")
        XDG_RUNTIME_DIR = (Join-Path $ClientRoot "runtime")
        APPDATA = (Join-Path $ClientRoot "roaming")
        LOCALAPPDATA = (Join-Path $ClientRoot "local")
        SE_SHARE_RELAY_URL = $script:RelayUrl
        SMART_EXPLORER_E2E_TEST_NAMESPACE = "$($script:TestNamespacePrefix)_$clientName"
    }
    $previous = @{}
    foreach ($name in $environment.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
        [Environment]::SetEnvironmentVariable($name, $environment[$name], "Process")
    }

    try {
        $stdinPath = Join-Path $ClientRoot "background-empty.stdin"
        [System.IO.File]::WriteAllBytes($stdinPath, [byte[]]@())
        $nativeArguments = (($Arguments | ForEach-Object {
            ConvertTo-NativeArgument $_
        }) -join ' ')
        $process = Start-Process -FilePath $script:SeBinary `
            -ArgumentList $nativeArguments `
            -PassThru `
            -RedirectStandardInput $stdinPath `
            -RedirectStandardOutput $StdoutPath `
            -RedirectStandardError $StderrPath `
            -WindowStyle Hidden
    }
    finally {
        foreach ($name in $environment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process")
        }
    }

    return [pscustomobject]@{
        Process = $process
        StdoutPath = $StdoutPath
        StderrPath = $StderrPath
    }
}

function Wait-ClientProcess {
    param(
        [psobject]$Invocation,
        [int]$TimeoutSeconds,
        [string]$Context
    )

    $process = $Invocation.Process
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        try { $process.Kill() } catch { }
        try { $process.WaitForExit(10000) | Out-Null } catch { }
        $stdout = if (Test-Path -LiteralPath $Invocation.StdoutPath) {
            [System.IO.File]::ReadAllText($Invocation.StdoutPath)
        }
        else { "" }
        $stderr = if (Test-Path -LiteralPath $Invocation.StderrPath) {
            [System.IO.File]::ReadAllText($Invocation.StderrPath)
        }
        else { "" }
        throw "$Context did not exit within $TimeoutSeconds seconds.`nstdout:`n$stdout`nstderr:`n$stderr"
    }
    # A second wait lets Start-Process finish closing its redirected file
    # handles before this test reads the two files.
    $process.WaitForExit()
    $process.Refresh()
    if (-not $process.HasExited) {
        throw "$Context reported WaitForExit success while the process was still active"
    }
    # Windows PowerShell 5.1 returned an empty adapted ExitCode for a fast,
    # redirected child in CI even after both waits. The retained native process
    # handle is the authoritative source and also distinguishes STILL_ACTIVE.
    [uint32]$nativeExitCode = 0
    if (-not [SmartExplorerE2E.NativeProcess]::GetExitCodeProcess(
        $process.Handle,
        [ref]$nativeExitCode
    )) {
        $win32 = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "$Context could not read its native exit code (Win32 $win32)"
    }
    if ($nativeExitCode -eq 259) {
        throw "$Context returned STILL_ACTIVE after both waits"
    }
    $stdout = if (Test-Path -LiteralPath $Invocation.StdoutPath) {
        [System.IO.File]::ReadAllText($Invocation.StdoutPath)
    }
    else { "" }
    $stderr = if (Test-Path -LiteralPath $Invocation.StderrPath) {
        [System.IO.File]::ReadAllText($Invocation.StderrPath)
    }
    else { "" }
    return [pscustomobject]@{
        ExitCode = [int]$nativeExitCode
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Wait-FileSignal {
    param(
        [string]$Path,
        [psobject]$Invocation,
        [int]$TimeoutSeconds,
        [string]$Context
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            return
        }
        if ($Invocation.Process.HasExited) {
            $result = Wait-ClientProcess $Invocation 1 $Context
            throw "$Context exited before its child-ready signal with code $($result.ExitCode).`nstdout:`n$($result.Stdout)`nstderr:`n$($result.Stderr)"
        }
        Start-Sleep -Milliseconds 50
    }
    throw "$Context did not create its child-ready signal within $TimeoutSeconds seconds: $Path"
}

function Test-LocalPortAvailable {
    param([int]$Port)

    $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, $Port)
    try {
        $listener.Start()
        return $true
    }
    catch {
        return $false
    }
    finally {
        $listener.Stop()
    }
}

function Get-LocalPortPair {
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $candidate = Get-Random -Minimum 31000 -Maximum 43000
        if ((Test-LocalPortAvailable $candidate) -and (Test-LocalPortAvailable ($candidate + 1))) {
            return $candidate
        }
    }
    throw "Could not find two consecutive local TCP ports for the Share server"
}

function Wait-LocalPort {
    param(
        [int]$Port,
        [System.Diagnostics.Process]$Server
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Server.HasExited) {
            throw "Share server exited before port $Port became ready"
        }
        $client = New-Object System.Net.Sockets.TcpClient
        try {
            $client.Connect("127.0.0.1", $Port)
            return
        }
        catch {
            Start-Sleep -Milliseconds 100
        }
        finally {
            $client.Dispose()
        }
    }
    throw "Share server port $Port did not become ready"
}

function Wait-RequestState {
    param(
        [string]$ClientRoot,
        [string]$RequestId,
        [scriptblock]$Predicate,
        [string]$Description
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "request", "show", $RequestId, "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $request = $lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop
                if (& $Predicate $request) {
                    return $request
                }
            }
            catch {
                # The worker may replace its snapshot while a retry is in flight.
            }
        }
        Start-Sleep -Milliseconds 250
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "Request $RequestId never reached $Description.`n$diagnostic"
}

function Wait-PendingRequestInbox {
    param([string]$ClientRoot)

    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "request", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $inbox = $lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop
                $requests = @($inbox.requests)
                if ([int]$inbox.count -eq 1 -and
                    $requests.Count -eq 1 -and
                    $requests[0].direction -eq "incoming" -and
                    $requests[0].delivery.state -eq "received" -and
                    $requests[0].decision.state -eq "pending" -and
                    -not [bool]$requests[0].authorization.active -and
                    $inbox.next_command -eq "se share request accept") {
                    return $inbox
                }
            }
            catch {
                # Retry while the worker durably commits the incoming request.
            }
        }
        Start-Sleep -Milliseconds 250
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "No received pending request appeared in B's bare inbox.`n$diagnostic"
}

function Wait-EmptyRequestInbox {
    param([string]$ClientRoot)

    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "request", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $inbox = $lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop
                if ([int]$inbox.count -eq 0 -and @($inbox.requests).Count -eq 0) {
                    return $inbox
                }
            }
            catch {
                # Retry through durable tombstone publication.
            }
        }
        Start-Sleep -Milliseconds 250
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "Pending request inbox did not become empty.`n$diagnostic"
}

function Wait-ExecHistory {
    param(
        [string]$ClientRoot,
        [string]$Direction,
        [string]$State
    )

    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "exec", "history", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $history = @($lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop)
                $match = @($history | Where-Object {
                    $_.direction -eq $Direction -and $_.job.state -eq $State
                })
                if ($match.Count -gt 0) {
                    return $match[$match.Count - 1]
                }
            }
            catch {
                # Retry until the daemon has durably published the terminal state.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "No $Direction Exec reached $State.`n$diagnostic"
}

function Wait-ExecState {
    param(
        [string]$ClientRoot,
        [string]$Direction,
        [string]$State,
        [int]$TimeoutSeconds = 60
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "exec", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $jobs = @($lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop)
                $matches = @($jobs | Where-Object {
                    $_.direction -eq $Direction -and $_.job.state -eq $State
                })
                if ($matches.Count -gt 1) {
                    throw "Ambiguous $Direction/$State active Exec set"
                }
                if ($matches.Count -eq 1) {
                    return $matches[0]
                }
            }
            catch {
                if ($_.Exception.Message -like "Ambiguous * active Exec set") {
                    throw
                }
                # Retry until both workers expose the active execution.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "No $Direction Exec reached active state $State.`n$diagnostic"
}

function Wait-ExecHistoryId {
    param(
        [string]$ClientRoot,
        [string]$Direction,
        [string]$State,
        [string]$ExecId,
        [int]$TimeoutSeconds = 60
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "exec", "history", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $history = @($lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop)
                $match = @($history | Where-Object {
                    $_.direction -eq $Direction -and
                        $_.job.state -eq $State -and
                        $_.job.exec_id -eq $ExecId
                })
                if ($match.Count -gt 0) {
                    return $match[$match.Count - 1]
                }
            }
            catch {
                # Retry until the daemon has durably published this exact job.
            }
        }
        Start-Sleep -Milliseconds 100
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "Exec $ExecId never reached $Direction/$State history.`n$diagnostic"
}

function Wait-WorkerConnected {
    param(
        [string]$ClientRoot,
        [int]$TimeoutSeconds = 60
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastResult = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $lastResult = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
            "share", "status", "--json"
        ) -TimeoutSeconds 10
        if ($lastResult.ExitCode -eq 0) {
            try {
                $status = $lastResult.Stdout | ConvertFrom-Json -ErrorAction Stop
                if ([bool]$status.running -and [bool]$status.connected) {
                    return $status
                }
            }
            catch {
                # Retry through worker restart and relay reconnection.
            }
        }
        Start-Sleep -Milliseconds 250
    }
    $diagnostic = if ($null -eq $lastResult) { "no command result" } else {
        "stdout:`n$($lastResult.Stdout)`nstderr:`n$($lastResult.Stderr)"
    }
    throw "Share worker did not become running and connected for $ClientRoot.`n$diagnostic"
}

function Get-ClientWorkerProcess {
    param(
        [string]$ClientRoot,
        [int]$TimeoutSeconds = 30
    )

    $syncDirectory = Join-Path $ClientRoot "roaming\smart_explorer\sync"
    $addressPath = Join-Path $syncDirectory "daemon.ipc"
    $generationPath = Join-Path $syncDirectory "daemon.generation"
    $expectedExecutable = [System.IO.Path]::GetFullPath($script:SeBinary)
    $quotedCommandLine = '"' + $expectedExecutable + '" --sync-daemon'
    $bareCommandLine = $expectedExecutable + ' --sync-daemon'
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastError = "worker publication was not readable"

    while ([DateTime]::UtcNow -lt $deadline) {
        $boundProcess = $null
        try {
            $addressBeforeAuth = [System.IO.File]::ReadAllText($addressPath).Trim()
            $generationBeforeAuth = [System.IO.File]::ReadAllText($generationPath).Trim()
            # Authenticate through the exact isolated token while requiring
            # the publication tuple to remain stable around that round trip.
            $status = Convert-CommandJson (Invoke-Client -ClientRoot $ClientRoot -Arguments @(
                "share", "status", "--json"
            ) -TimeoutSeconds 10) "worker authentication for $ClientRoot"
            $address = [System.IO.File]::ReadAllText($addressPath).Trim()
            $generation = [System.IO.File]::ReadAllText($generationPath).Trim()
            if ($addressBeforeAuth -ne $address -or $generationBeforeAuth -ne $generation) {
                throw "daemon publication changed during authenticated status"
            }
            if (-not [bool]$status.running) {
                throw "authenticated Share status did not report a running worker"
            }
            if ($address -notmatch '^127\.0\.0\.1:([0-9]{1,5})$') {
                throw "invalid loopback daemon address: $address"
            }
            $port = [int]$Matches[1]
            if ($port -lt 1 -or $port -gt 65535) {
                throw "invalid daemon port: $port"
            }
            if ($generation -notmatch '^[0-9a-fA-F]{32}$') {
                throw "invalid daemon generation: $generation"
            }

            $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction Stop |
                Where-Object { $_.LocalAddress -eq "127.0.0.1" })
            if ($listeners.Count -ne 1) {
                throw "expected one 127.0.0.1:$port listener, found $($listeners.Count)"
            }
            $workerPid = [int]$listeners[0].OwningProcess
            if ($workerPid -le 0) {
                throw "listener did not expose an owning PID"
            }

            $cimProcess = Get-CimInstance -ClassName Win32_Process `
                -Filter "ProcessId = $workerPid" -ErrorAction Stop
            if ($null -eq $cimProcess) {
                throw "listener PID $workerPid has no Win32_Process record"
            }
            if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
                [string]$cimProcess.ExecutablePath,
                $expectedExecutable
            )) {
                throw "listener PID $workerPid executable does not match the isolated se binary"
            }
            $commandLine = [string]$cimProcess.CommandLine
            if (-not [StringComparer]::OrdinalIgnoreCase.Equals($commandLine, $quotedCommandLine) -and
                -not [StringComparer]::OrdinalIgnoreCase.Equals($commandLine, $bareCommandLine)) {
                throw "listener PID $workerPid command line was not exactly se --sync-daemon"
            }

            # Opening Handle pins this exact process object while the endpoint
            # and generation are revalidated, preventing a PID-reuse race.
            $boundProcess = [System.Diagnostics.Process]::GetProcessById($workerPid)
            $heldHandle = $boundProcess.Handle
            if ($heldHandle -eq [IntPtr]::Zero -or $boundProcess.HasExited) {
                throw "listener PID $workerPid exited while its handle was acquired"
            }
            $mainModulePath = [System.IO.Path]::GetFullPath($boundProcess.MainModule.FileName)
            if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
                $mainModulePath,
                $expectedExecutable
            )) {
                throw "listener PID $workerPid main module changed during binding"
            }

            $addressAfter = [System.IO.File]::ReadAllText($addressPath).Trim()
            $generationAfter = [System.IO.File]::ReadAllText($generationPath).Trim()
            $listenersAfter = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction Stop |
                Where-Object { $_.LocalAddress -eq "127.0.0.1" })
            if ($addressAfter -ne $address -or $generationAfter -ne $generation -or
                $listenersAfter.Count -ne 1 -or
                [int]$listenersAfter[0].OwningProcess -ne $workerPid -or
                $boundProcess.HasExited) {
                throw "daemon publication changed while binding worker PID $workerPid"
            }
            return $boundProcess
        }
        catch {
            $lastError = $_.Exception.Message
            if ($null -ne $boundProcess) {
                $boundProcess.Dispose()
            }
            Start-Sleep -Milliseconds 100
        }
    }
    throw "Could not bind the exact isolated worker for ${ClientRoot}: $lastError"
}

function Stop-BoundWorkerHard {
    param(
        [System.Diagnostics.Process]$Worker,
        [string]$Context
    )

    if ($Worker.HasExited) {
        throw "$Context worker exited before the hard-kill step"
    }
    $Worker.Kill()
    if (-not $Worker.WaitForExit(10000)) {
        throw "$Context worker did not exit within 10 seconds after Kill()"
    }
}

function Stop-ClientWorker {
    param([string]$ClientRoot)

    return Invoke-Client -ClientRoot $ClientRoot -Arguments @("share", "worker", "stop")
}

function Assert-RemotePathAbsent {
    param(
        [string]$ClientRoot,
        [string]$Path,
        [string]$Context
    )

    $result = Invoke-Client -ClientRoot $ClientRoot -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
        'if (Test-Path -LiteralPath $args[0]) { exit 9 }', $Path
    )
    Assert-Success $result $Context
}

if (-not (Test-Path -LiteralPath $script:SeBinary -PathType Leaf)) {
    throw "Windows se test binary is missing: $($script:SeBinary)"
}
if (-not (Test-Path -LiteralPath $script:ShareServerBinary -PathType Leaf)) {
    throw "Windows Share server test binary is missing: $($script:ShareServerBinary)"
}
if ($null -eq (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue)) {
    throw "Windows Share lifecycle E2E requires Get-NetTCPConnection for exact worker binding"
}
if ($null -eq (Get-Command Get-CimInstance -ErrorAction SilentlyContinue)) {
    throw "Windows Share lifecycle E2E requires Get-CimInstance for exact worker verification"
}

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("se-share-lifecycle-windows-" + [Guid]::NewGuid().ToString("N"))
$clientA = Join-Path $root "a"
$clientB = Join-Path $root "b"
$clientC = Join-Path $root "c"
$clientD = Join-Path $root "d"
$serverStdout = Join-Path $root "share-server.stdout"
$serverStderr = Join-Path $root "share-server.stderr"
$server = $null
$workersStopped = $false
$succeeded = $false

New-Item -ItemType Directory -Path $root -Force | Out-Null
New-IsolatedClient $clientA
New-IsolatedClient $clientB
New-IsolatedClient $clientC
New-IsolatedClient $clientD

# The remote parent creates a real descendant and waits for it. The child does
# no work until the test hard-kills one exact Share worker and creates Trigger;
# a later marker therefore proves that the process escaped worker containment.
$crashParentScriptContent = @'
param(
    [Parameter(Mandatory = $true)][string]$ChildScript,
    [Parameter(Mandatory = $true)][string]$TriggerPath,
    [Parameter(Mandatory = $true)][string]$MarkerPath,
    [Parameter(Mandatory = $true)][string]$ReadyPath,
    [Parameter(Mandatory = $true)][int]$DelaySeconds
)

$ErrorActionPreference = "Stop"
if ($ChildScript.Contains('"')) {
    throw "child script path contains an invalid quote"
}
[Environment]::SetEnvironmentVariable("SE_E2E_CRASH_TRIGGER", $TriggerPath, "Process")
[Environment]::SetEnvironmentVariable("SE_E2E_CRASH_MARKER", $MarkerPath, "Process")
[Environment]::SetEnvironmentVariable("SE_E2E_CRASH_DELAY", $DelaySeconds.ToString(), "Process")
$powerShell = Join-Path $PSHOME "powershell.exe"
$childArguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $ChildScript + '"'
$child = Start-Process -FilePath $powerShell -ArgumentList $childArguments -PassThru -WindowStyle Hidden
[System.IO.File]::WriteAllText($ReadyPath, "ready")
$child.WaitForExit()
exit $child.ExitCode
'@
$crashChildScriptContent = @'
$ErrorActionPreference = "Stop"
$triggerPath = [Environment]::GetEnvironmentVariable("SE_E2E_CRASH_TRIGGER", "Process")
$markerPath = [Environment]::GetEnvironmentVariable("SE_E2E_CRASH_MARKER", "Process")
$delaySeconds = [int][Environment]::GetEnvironmentVariable("SE_E2E_CRASH_DELAY", "Process")
if ([string]::IsNullOrWhiteSpace($triggerPath) -or
    [string]::IsNullOrWhiteSpace($markerPath) -or
    $delaySeconds -lt 1) {
    throw "worker crash fixture environment is incomplete"
}
while (-not (Test-Path -LiteralPath $triggerPath -PathType Leaf)) {
    Start-Sleep -Milliseconds 50
}
Start-Sleep -Seconds $delaySeconds
[System.IO.File]::WriteAllText($markerPath, "escaped")
'@

try {
    $signalPort = Get-LocalPortPair
    $script:RelayUrl = "http://127.0.0.1:$($signalPort + 1)"
    $server = Start-Process -FilePath $script:ShareServerBinary `
        -ArgumentList @("127.0.0.1:$signalPort") `
        -PassThru `
        -RedirectStandardOutput $serverStdout `
        -RedirectStandardError $serverStderr `
        -WindowStyle Hidden
    Wait-LocalPort $signalPort $server
    Wait-LocalPort ($signalPort + 1) $server

    # B is intentionally not configured yet. A may only report an offline relay
    # queue until B has received the signed request in this test run.
    $identityBResult = Invoke-Client -ClientRoot $clientB -Arguments @("share", "identity", "--json")
    $identityB = Convert-CommandJson $identityBResult "B identity"
    $directCodeB = [string]$identityB.direct_code
    Assert-True (-not [string]::IsNullOrWhiteSpace($directCodeB)) "B identity did not emit a direct invite code"

    $configureA = Invoke-Client -ClientRoot $clientA -Arguments @(
        "share", "configure", "--server", "127.0.0.1:$signalPort"
    )
    Assert-Success $configureA "A configure"

    $add = Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "add-peer", "--code", $directCodeB, "--name", "WindowsTarget", "--json"
    )) "A add-peer"
    $peerSelector = [string]$add.selector
    $directEndpoint = [string]$add.endpoint
    $requestId = [string]$add.request_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($peerSelector)) "add-peer emitted an empty peer selector"
    Assert-True (-not [string]::IsNullOrWhiteSpace($directEndpoint)) "add-peer emitted an empty direct endpoint"
    Assert-True ($directEndpoint.StartsWith("share://direct/", [StringComparison]::OrdinalIgnoreCase)) "add-peer emitted an invalid direct endpoint"
    Assert-True (-not [string]::IsNullOrWhiteSpace($requestId)) "empty request ID"
    Assert-True ($add.request.request_id -eq $requestId) "add-peer nested request ID did not match its selector"
    Assert-True ($add.request.direction -eq "outgoing") "add-peer nested request direction was not outgoing"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$add.request.delivery.state)) "add-peer omitted nested delivery state"
    Assert-True ($null -ne $add.request.relay.PSObject.Properties["outcome"]) "add-peer omitted nested relay outcome"
    Assert-True ($add.request.decision.state -eq "pending") "add-peer nested decision was not pending"
    Assert-True (-not [bool]$add.request.authorization.active) "add-peer activated authorization before acceptance"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$add.worker_refresh.state)) "add-peer omitted nested worker refresh state"
    Assert-True ($null -ne $add.worker_refresh.PSObject.Properties["error"]) "add-peer omitted nested worker refresh error"

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.relay.outcome -eq "target_offline" -and
            $request.peer_receipt.request.state -eq "unconfirmed"
    } "offline relay queue without a peer receipt" | Out-Null

    $configureB = Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "configure", "--server", "127.0.0.1:$signalPort"
    )
    Assert-Success $configureB "B configure"
    $retry = Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "share", "request", "retry", "--json"
    )) "A context-free request retry"
    Assert-True ($retry.request.request_id -eq $requestId) "retry selected a request not discovered by add-peer"

    # B first discovers the request through its own bare inbox. No request ID,
    # device ID, or fingerprint from A is supplied to any target-side command.
    $inbox = Wait-PendingRequestInbox $clientB
    $inboxRequests = @($inbox.requests)
    Assert-True ($inboxRequests.Count -eq 1) "B inbox did not expose exactly one pending request"
    $inboxRequest = $inboxRequests[0]
    $inboxRequestId = [string]$inboxRequest.request_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($inboxRequestId)) "B inbox emitted an empty request ID"
    Assert-True ($inboxRequestId -eq $requestId) "B inbox request does not match A add-peer output"
    Assert-True ($inboxRequest.direction -eq "incoming") "B inbox request was not incoming"
    Assert-True ($inboxRequest.delivery.state -eq "received") "B inbox request was not durably received"
    Assert-True ($inboxRequest.decision.state -eq "pending") "B inbox request was not pending"
    Assert-True (-not [bool]$inboxRequest.authorization.active) "B inbox request was active before acceptance"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$inboxRequest.peer.fingerprint)) "B inbox omitted the signed requester fingerprint"
    Assert-True ($inbox.next_command -eq "se share request accept") "B inbox did not emit the bare accept command"

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.delivery.state -eq "received" -and
            $request.peer_receipt.request.state -eq "received"
    } "durably received with signed request receipt on requester" | Out-Null

    # The pending inbox must survive a full target-worker restart.
    $stopPendingB = Stop-ClientWorker $clientB
    Assert-Success $stopPendingB "B pending worker stop"
    Wait-WorkerConnected $clientB 60 | Out-Null
    $restartedInbox = Wait-PendingRequestInbox $clientB
    Assert-True ([string]$restartedInbox.requests[0].request_id -eq $inboxRequestId) "B lost its pending request across worker restart"

    # A is offline while B accepts. B must retain and retry its signed
    # decision until A restarts and returns the decision receipt.
    $stopBeforeAcceptA = Stop-ClientWorker $clientA
    Assert-Success $stopBeforeAcceptA "A worker stop before B acceptance"

    $accepted = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "accept", "--json"
    )) "B bare request accept"
    Assert-True ($accepted.request.decision.state -eq "accepted") "B did not persist accepted state"
    Assert-True ([bool]$accepted.request.authorization.active) "B authorization did not become active"

    Wait-WorkerConnected $clientA 60 | Out-Null

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.decision.state -eq "accepted" -and
            [bool]$request.authorization.active -and
            $request.connectivity.state -eq "available"
    } "accepted/active/available on requester" | Out-Null
    Wait-RequestState $clientB $inboxRequestId {
        param($request)
        $request.decision.state -eq "accepted" -and
            [bool]$request.authorization.active -and
            $request.peer_receipt.decision.state -eq "received"
    } "accepted/active with decision receipt on target" | Out-Null

    # Accepted history is the signed authorization basis and cannot be erased
    # while it is active. The refusal itself is context-free as there is one.
    $deleteActive = Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "delete", "--json"
    )
    Assert-True ($deleteActive.ExitCode -ne 0) "active accepted request history was deletable"
    $stillAccepted = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "show", "--json"
    )) "B request after refused active delete"
    Assert-True ($stillAccepted.request_id -eq $inboxRequestId) "refused delete lost the accepted request"
    Assert-True ([bool]$stillAccepted.authorization.active) "refused delete deactivated authorization"

    # The target discovers its sole exact-device Exec grant locally. No device,
    # fingerprint, contact, or request selector is supplied to enable/disable.
    $execGrants = @(Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "grants", "exec", "--json"
    )) "B exec grant list")
    Assert-True ($execGrants.Count -eq 1) "B did not expose exactly one exact-device Exec grant"
    Assert-True (-not [bool]$execGrants[0].enabled) "Exec grant was enabled before explicit consent"
    Assert-True (-not [string]::IsNullOrWhiteSpace([string]$execGrants[0].target)) "Exec grant list omitted its selector"

    $enabled = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "grants", "exec", "enable", "--yes", "--json"
    )) "B bare Exec enable"
    Assert-True ([bool]$enabled.persisted -and [bool]$enabled.applied) "Exec grant enable was not persisted and applied"

    # Learn B's home through the remote CLI before deriving any target-side
    # fixture path. The harness layout is never used as remote path knowledge.
    $remoteHomeResult = Invoke-Client -ClientRoot $clientA -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
        "Write-Output `$env:USERPROFILE"
    )
    Assert-Success $remoteHomeResult "remote Windows home discovery"
    $remoteHome = $remoteHomeResult.Stdout.Trim()
    Assert-True ([System.IO.Path]::IsPathRooted($remoteHome)) "remote home was not an absolute path"
    $crashParentScript = Join-Path $remoteHome "worker-crash-parent.ps1"
    $crashChildScript = Join-Path $remoteHome "worker-crash-child.ps1"
    [System.IO.File]::WriteAllText($crashParentScript, $crashParentScriptContent)
    [System.IO.File]::WriteAllText($crashChildScript, $crashChildScriptContent)

    # This is a real remote Windows argv execution through daemon IPC + Iroh.
    # Its non-zero process exit code must be returned by the local CLI.
    $exec = Invoke-Client -ClientRoot $clientA -Arguments @(
        "exec", "--", "cmd.exe", "/d", "/s", "/c", "echo WINDOWS_EXEC_OK & exit /b 7"
    )
    Assert-True ($exec.ExitCode -eq 7) "remote cmd.exe exit code was $($exec.ExitCode), expected 7`n$($exec.Stderr)"
    Assert-True ($exec.Stdout -match "(?m)^WINDOWS_EXEC_OK\s*$") "remote cmd.exe stdout was not returned"
    $outgoingExec = Wait-ExecHistory $clientA "outgoing" "exited"
    $incomingExec = Wait-ExecHistory $clientB "incoming" "exited"
    Assert-True ($outgoingExec.job.exec_id -eq $incomingExec.job.exec_id) "Exec history did not converge on both endpoints"
    Assert-True ([int]$outgoingExec.job.terminal.exit_code -eq 7) "outgoing Exec history lost the remote exit code"

    # A healthy command may remain completely silent beyond the 20-second
    # peer-liveness deadline. Authenticated Ping/Pong keeps it alive without
    # imposing a runtime or output-idle limit on the command itself.
    $silentWatch = [System.Diagnostics.Stopwatch]::StartNew()
    $silentExec = Invoke-Client -ClientRoot $clientA -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
        "Start-Sleep -Seconds 25; Write-Output WINDOWS_HEARTBEAT_OK"
    )
    $silentWatch.Stop()
    Assert-Success $silentExec "silent Windows Exec beyond heartbeat deadline"
    Assert-True ($silentWatch.Elapsed.TotalSeconds -ge 24) "silent Exec returned before its command completed"
    Assert-True ($silentExec.Stdout -match "(?m)^WINDOWS_HEARTBEAT_OK\s*$") "silent Exec lost its terminal output"

    # B discovers and cancels its sole active incoming command without an ID
    # supplied by A. Both endpoints must persist the same Cancelled lifecycle.
    $cancelTrigger = Join-Path $remoteHome "cancel.trigger"
    $cancelMarker = Join-Path $remoteHome "cancel-escaped.marker"
    $cancelReady = Join-Path $remoteHome "cancel.ready"
    $cancelInvocation = Start-ClientProcess -ClientRoot $clientA -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive",
        "-ExecutionPolicy", "Bypass", "-File", $crashParentScript,
        "-ChildScript", $crashChildScript,
        "-TriggerPath", $cancelTrigger,
        "-MarkerPath", $cancelMarker,
        "-ReadyPath", $cancelReady,
        "-DelaySeconds", "5"
    ) -StdoutPath (Join-Path $root "cancel.stdout") `
        -StderrPath (Join-Path $root "cancel.stderr")
    Wait-FileSignal $cancelReady $cancelInvocation 15 "cancel fixture"
    $cancelOutgoing = Wait-ExecState $clientA "outgoing" "running" 30
    $cancelIncoming = Wait-ExecState $clientB "incoming" "running" 30
    Assert-True ([string]$cancelOutgoing.job.exec_id -eq [string]$cancelIncoming.job.exec_id) "cancel Exec IDs did not converge"
    $cancelled = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "exec", "cancel", "--json"
    )) "B bare Exec cancel"
    Assert-True ([bool]$cancelled.cancel_requested) "B did not request cancellation"
    Assert-True ([string]$cancelled.exec_id -eq [string]$cancelIncoming.job.exec_id) "B cancelled a different Exec"
    $cancelResult = Wait-ClientProcess $cancelInvocation 30 "cancelled Windows Exec CLI"
    Assert-True ($cancelResult.ExitCode -eq 130) "cancelled Windows Exec returned $($cancelResult.ExitCode), expected 130"
    [System.IO.File]::WriteAllText($cancelTrigger, "trigger")
    Wait-ExecHistoryId $clientA "outgoing" "cancelled" ([string]$cancelOutgoing.job.exec_id) 30 | Out-Null
    Wait-ExecHistoryId $clientB "incoming" "cancelled" ([string]$cancelIncoming.job.exec_id) 30 | Out-Null
    Start-Sleep -Seconds 6
    Assert-RemotePathAbsent $clientA $cancelMarker "cancelled remote marker verification"

    # A hard target-worker crash must close the Windows Job Object and its Iroh
    # connection. Each endpoint independently discovers the active Exec ID via
    # its own CLI before the exact target daemon is bound and killed.
    $targetCrashTrigger = Join-Path $remoteHome "target-worker-crash.trigger"
    $targetCrashMarker = Join-Path $remoteHome "target-worker-crash-escaped.marker"
    $targetCrashReady = Join-Path $remoteHome "target-worker-crash.ready"
    $targetCrashInvocation = Start-ClientProcess -ClientRoot $clientA -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive",
        "-ExecutionPolicy", "Bypass", "-File", $crashParentScript,
        "-ChildScript", $crashChildScript,
        "-TriggerPath", $targetCrashTrigger,
        "-MarkerPath", $targetCrashMarker,
        "-ReadyPath", $targetCrashReady,
        "-DelaySeconds", "5"
    ) -StdoutPath (Join-Path $root "target-worker-crash.stdout") `
        -StderrPath (Join-Path $root "target-worker-crash.stderr")
    Wait-FileSignal $targetCrashReady $targetCrashInvocation 15 "target-worker crash fixture"
    $targetCrashOutgoing = Wait-ExecState $clientA "outgoing" "running" 30
    $targetCrashIncoming = Wait-ExecState $clientB "incoming" "running" 30
    $targetCrashOutgoingId = [string]$targetCrashOutgoing.job.exec_id
    $targetCrashIncomingId = [string]$targetCrashIncoming.job.exec_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($targetCrashOutgoingId)) "A did not discover the target-crash Exec ID"
    Assert-True (-not [string]::IsNullOrWhiteSpace($targetCrashIncomingId)) "B did not discover the target-crash Exec ID"
    Assert-True ($targetCrashOutgoingId -eq $targetCrashIncomingId) "target-crash Exec IDs did not converge"

    $targetWorker = Get-ClientWorkerProcess $clientB 30
    try {
        Stop-BoundWorkerHard $targetWorker "target-crash"
    }
    finally {
        $targetWorker.Dispose()
    }
    [System.IO.File]::WriteAllText($targetCrashTrigger, "trigger")
    $targetCrashResult = Wait-ClientProcess $targetCrashInvocation 30 "target-worker crash CLI"
    Assert-True ($targetCrashResult.ExitCode -eq 125) "target-worker crash CLI returned $($targetCrashResult.ExitCode), expected 125`n$($targetCrashResult.Stderr)"
    Wait-ExecHistoryId $clientA "outgoing" "disconnected" $targetCrashOutgoingId 30 | Out-Null
    Start-Sleep -Seconds 6
    Assert-True (-not (Test-Path -LiteralPath $targetCrashMarker)) "target-worker crash left an escaped remote descendant"
    Wait-WorkerConnected $clientB 60 | Out-Null
    Assert-RemotePathAbsent $clientA $targetCrashMarker "target-worker remote marker verification"

    # A hard requester-worker crash closes its local IPC immediately. B must
    # independently detect missing authenticated Pings, mark this exact job as
    # disconnected, and kill the full process tree before its delayed marker.
    $requesterCrashTrigger = Join-Path $remoteHome "requester-worker-crash.trigger"
    $requesterCrashMarker = Join-Path $remoteHome "requester-worker-crash-escaped.marker"
    $requesterCrashReady = Join-Path $remoteHome "requester-worker-crash.ready"
    $requesterCrashInvocation = Start-ClientProcess -ClientRoot $clientA -Arguments @(
        "exec", "--", "powershell.exe", "-NoProfile", "-NonInteractive",
        "-ExecutionPolicy", "Bypass", "-File", $crashParentScript,
        "-ChildScript", $crashChildScript,
        "-TriggerPath", $requesterCrashTrigger,
        "-MarkerPath", $requesterCrashMarker,
        "-ReadyPath", $requesterCrashReady,
        "-DelaySeconds", "30"
    ) -StdoutPath (Join-Path $root "requester-worker-crash.stdout") `
        -StderrPath (Join-Path $root "requester-worker-crash.stderr")
    Wait-FileSignal $requesterCrashReady $requesterCrashInvocation 15 "requester-worker crash fixture"
    $requesterCrashOutgoing = Wait-ExecState $clientA "outgoing" "running" 30
    $requesterCrashIncoming = Wait-ExecState $clientB "incoming" "running" 30
    $requesterCrashOutgoingId = [string]$requesterCrashOutgoing.job.exec_id
    $requesterCrashIncomingId = [string]$requesterCrashIncoming.job.exec_id
    Assert-True (-not [string]::IsNullOrWhiteSpace($requesterCrashOutgoingId)) "A did not discover the requester-crash Exec ID"
    Assert-True (-not [string]::IsNullOrWhiteSpace($requesterCrashIncomingId)) "B did not discover the requester-crash Exec ID"
    Assert-True ($requesterCrashOutgoingId -eq $requesterCrashIncomingId) "requester-crash Exec IDs did not converge"

    $requesterWorker = Get-ClientWorkerProcess $clientA 30
    try {
        Stop-BoundWorkerHard $requesterWorker "requester-crash"
    }
    finally {
        $requesterWorker.Dispose()
    }
    [System.IO.File]::WriteAllText($requesterCrashTrigger, "trigger")
    $requesterCrashResult = Wait-ClientProcess $requesterCrashInvocation 15 "requester-worker crash CLI"
    Assert-True ($requesterCrashResult.ExitCode -eq 125) "requester-worker crash CLI returned $($requesterCrashResult.ExitCode), expected 125`n$($requesterCrashResult.Stderr)"
    Wait-ExecHistoryId $clientB "incoming" "disconnected" $requesterCrashIncomingId 30 | Out-Null
    Start-Sleep -Seconds 31
    Assert-True (-not (Test-Path -LiteralPath $requesterCrashMarker)) "requester-worker crash left an escaped remote descendant"
    Wait-WorkerConnected $clientA 60 | Out-Null
    Assert-RemotePathAbsent $clientA $requesterCrashMarker "requester-worker remote marker verification"

    $disabled = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "grants", "exec", "disable", "--json"
    )) "B bare Exec disable"
    Assert-True ([bool]$disabled.persisted -and [bool]$disabled.applied) "Exec grant disable was not persisted and applied"
    $denied = Invoke-Client -ClientRoot $clientA -Arguments @(
        "exec", "--", "cmd.exe", "/d", "/s", "/c", "echo DENIED_EXEC_RAN"
    )
    Assert-True ($denied.ExitCode -eq 125) "post-disable Exec returned $($denied.ExitCode), expected denial exit 125"
    Assert-True ($denied.Stdout -notmatch "DENIED_EXEC_RAN") "post-disable payload executed"

    $revoked = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "grants", "revoke", "--json"
    )) "B bare signed base revoke"
    Assert-True ($revoked.request.request_id -eq $inboxRequestId) "base revoke selected an undiscovered authorization"
    Assert-True ($revoked.request.decision.state -eq "revoked") "B did not persist revoked state"
    Assert-True (-not [bool]$revoked.request.authorization.active) "B authorization remained active after revoke"

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.decision.state -eq "revoked" -and -not [bool]$request.authorization.active
    } "revoked/inactive on requester" | Out-Null
    Wait-RequestState $clientB $inboxRequestId {
        param($request)
        $request.decision.state -eq "revoked" -and
            -not [bool]$request.authorization.active -and
            $request.peer_receipt.decision.state -eq "received"
    } "revoked/inactive with receipt on target" | Out-Null

    $deleted = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "delete", "--json"
    )) "B context-free post-revoke delete"
    Assert-True ($deleted.action -eq "deleted") "post-revoke request history was not deleted"
    Assert-True ($deleted.request_id -eq $inboxRequestId) "post-revoke delete selected an undiscovered request"
    Assert-True ([bool]$deleted.persisted) "post-revoke delete was not persisted"

    # The list exposes the same full values returned by add-peer, but removal of
    # the sole peer itself needs no selector or otherwise pre-known identifier.
    $connections = @(Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "list", "--json"
    )) "A connections before bare peer removal")
    $directConnections = @($connections | Where-Object { $_.kind -eq "direct" })
    Assert-True ($directConnections.Count -eq 1) "A did not list exactly one direct peer before removal"
    Assert-True ([string]$directConnections[0].selector -eq $peerSelector) "connections list changed the add-peer selector"
    Assert-True ([string]$directConnections[0].endpoint -eq $directEndpoint) "connections list changed the add-peer endpoint"

    $removed = Invoke-Client -ClientRoot $clientA -Arguments @("connections", "remove-peer")
    Assert-Success $removed "A bare peer removal"
    $afterRemove = @(Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "list", "--json"
    )) "A connections after bare peer removal")
    $remainingDirect = @($afterRemove | Where-Object { $_.kind -eq "direct" })
    Assert-True ($remainingDirect.Count -eq 0) "A still listed a direct peer after bare removal"

    # A fresh third device avoids inheriting B's intentionally revoked grant
    # and exercises rejection from a genuinely pending state.
    $identityC = Convert-CommandJson (Invoke-Client -ClientRoot $clientC -Arguments @(
        "share", "identity", "--json"
    )) "C identity"
    $directCodeC = [string]$identityC.direct_code
    Assert-Success (Invoke-Client -ClientRoot $clientC -Arguments @(
        "share", "configure", "--server", "127.0.0.1:$signalPort"
    )) "C configure"
    $rejectAdd = Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "add-peer", "--code", $directCodeC, "--name", "RejectTarget", "--json"
    )) "A rejection add-peer"
    $rejectRequestId = [string]$rejectAdd.request_id
    $rejectInbox = Wait-PendingRequestInbox $clientC
    $rejectInboxId = [string]$rejectInbox.requests[0].request_id
    Assert-True ($rejectInboxId -eq $rejectRequestId) "rejection inbox ID did not match add-peer"
    Wait-RequestState $clientA $rejectRequestId {
        param($request)
        $request.delivery.state -eq "received" -and
            $request.peer_receipt.request.state -eq "received"
    } "rejection request receipt" | Out-Null
    $rejected = Convert-CommandJson (Invoke-Client -ClientRoot $clientC -Arguments @(
        "share", "request", "reject", "--json"
    )) "B bare request reject"
    Assert-True ($rejected.request.request_id -eq $rejectInboxId) "B rejected a different request"
    Assert-True ($rejected.request.decision.state -eq "rejected") "B did not persist rejection"
    Assert-True (-not [bool]$rejected.request.authorization.active) "rejection activated authorization"
    Wait-RequestState $clientA $rejectRequestId {
        param($request)
        $request.decision.state -eq "rejected" -and -not [bool]$request.authorization.active
    } "rejected/inactive on requester" | Out-Null
    Wait-RequestState $clientC $rejectInboxId {
        param($request)
        $request.decision_delivery.state -eq "received" -and
            $request.peer_receipt.decision.state -eq "received"
    } "rejection receipt on target" | Out-Null
    $rejectDeleted = Convert-CommandJson (Invoke-Client -ClientRoot $clientC -Arguments @(
        "share", "request", "delete", "--json"
    )) "B rejected history delete"
    Assert-True ($rejectDeleted.request_id -eq $rejectInboxId) "B deleted a different rejected request"
    Assert-Success (Invoke-Client -ClientRoot $clientA -Arguments @("connections", "remove-peer")) "A reject peer removal"

    # A fourth fresh device supplies a genuinely pending request. Two complete
    # worker restarts prove its local dismissal tombstone remains durable.
    $identityD = Convert-CommandJson (Invoke-Client -ClientRoot $clientD -Arguments @(
        "share", "identity", "--json"
    )) "D identity"
    $directCodeD = [string]$identityD.direct_code
    Assert-Success (Invoke-Client -ClientRoot $clientD -Arguments @(
        "share", "configure", "--server", "127.0.0.1:$signalPort"
    )) "D configure"
    $pendingAdd = Convert-CommandJson (Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "add-peer", "--code", $directCodeD, "--name", "TombstoneTarget", "--json"
    )) "A tombstone add-peer"
    $pendingRequestId = [string]$pendingAdd.request_id
    $pendingInbox = Wait-PendingRequestInbox $clientD
    $pendingInboxId = [string]$pendingInbox.requests[0].request_id
    Assert-True ($pendingInboxId -eq $pendingRequestId) "pending-delete inbox ID did not match add-peer"
    $pendingDeleted = Convert-CommandJson (Invoke-Client -ClientRoot $clientD -Arguments @(
        "share", "request", "delete", "--json"
    )) "B pending request delete"
    Assert-True ($pendingDeleted.request_id -eq $pendingInboxId) "B deleted a different pending request"
    Assert-True ([bool]$pendingDeleted.persisted) "pending request tombstone was not persisted"
    Assert-Success (Stop-ClientWorker $clientD) "D tombstone worker stop"
    Wait-WorkerConnected $clientD 60 | Out-Null
    Wait-EmptyRequestInbox $clientD | Out-Null
    Assert-Success (Stop-ClientWorker $clientD) "D second tombstone worker stop"
    Wait-WorkerConnected $clientD 60 | Out-Null
    Wait-EmptyRequestInbox $clientD | Out-Null
    Assert-Success (Invoke-Client -ClientRoot $clientA -Arguments @("connections", "remove-peer")) "A tombstone peer removal"

    $stopA = Stop-ClientWorker $clientA
    Assert-Success $stopA "A worker stop"
    $stopB = Stop-ClientWorker $clientB
    Assert-Success $stopB "B worker stop"
    $stopC = Stop-ClientWorker $clientC
    Assert-Success $stopC "C worker stop"
    $stopD = Stop-ClientWorker $clientD
    Assert-Success $stopD "D worker stop"
    $workersStopped = $true
    $succeeded = $true
    Write-Host "Windows Share/Exec CLI lifecycle E2E passed: $requestId"
}
finally {
    if (-not $workersStopped) {
        try { Stop-ClientWorker $clientA | Out-Null } catch { }
        try { Stop-ClientWorker $clientB | Out-Null } catch { }
        try { Stop-ClientWorker $clientC | Out-Null } catch { }
        try { Stop-ClientWorker $clientD | Out-Null } catch { }
    }
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        $server.WaitForExit(10000) | Out-Null
    }
    if ($succeeded) {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
    else {
        Write-Error "Windows Share/Exec CLI lifecycle E2E failed; diagnostics: $root" -ErrorAction Continue
    }
}
