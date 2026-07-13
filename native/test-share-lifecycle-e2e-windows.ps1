[CmdletBinding()]
param(
    [string]$SeBinary = (Join-Path $PSScriptRoot "target\debug\se.exe"),
    [string]$ShareServerBinary = (Join-Path $PSScriptRoot "..\share-server\target\debug\se-share-server.exe")
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

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
        [string[]]$Arguments
    )

    $commandId = [Guid]::NewGuid().ToString("N")
    $stdoutPath = Join-Path $ClientRoot "command-$commandId.stdout"
    $stderrPath = Join-Path $ClientRoot "command-$commandId.stderr"
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
        & $script:SeBinary @Arguments 1> $stdoutPath 2> $stderrPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        foreach ($name in $environment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previous[$name], "Process")
        }
    }

    $stdout = if (Test-Path -LiteralPath $stdoutPath) {
        [System.IO.File]::ReadAllText($stdoutPath)
    }
    else {
        ""
    }
    $stderr = if (Test-Path -LiteralPath $stderrPath) {
        [System.IO.File]::ReadAllText($stderrPath)
    }
    else {
        ""
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Stdout = $stdout
        Stderr = $stderr
    }
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
        )
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
        )
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

function Stop-ClientWorker {
    param([string]$ClientRoot)

    return Invoke-Client -ClientRoot $ClientRoot -Arguments @("share", "worker", "stop")
}

if (-not (Test-Path -LiteralPath $script:SeBinary -PathType Leaf)) {
    throw "Windows se test binary is missing: $($script:SeBinary)"
}
if (-not (Test-Path -LiteralPath $script:ShareServerBinary -PathType Leaf)) {
    throw "Windows Share server test binary is missing: $($script:ShareServerBinary)"
}

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("se-share-lifecycle-windows-" + [Guid]::NewGuid().ToString("N"))
$clientA = Join-Path $root "a"
$clientB = Join-Path $root "b"
$serverStdout = Join-Path $root "share-server.stdout"
$serverStderr = Join-Path $root "share-server.stderr"
$server = $null
$workersStopped = $false
$succeeded = $false

New-Item -ItemType Directory -Path $root -Force | Out-Null
New-IsolatedClient $clientA
New-IsolatedClient $clientB

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

    $add = Invoke-Client -ClientRoot $clientA -Arguments @(
        "connections", "add-peer", "--code", $directCodeB, "--name", "WindowsTarget"
    )
    Assert-Success $add "A add-peer"
    $contactMatch = [regex]::Match($add.Stdout, "peer contact ([^;]+);")
    $requestMatch = [regex]::Match($add.Stdout, "request_id=([^;]+);")
    Assert-True $contactMatch.Success "add-peer did not emit a peer contact selector"
    Assert-True $requestMatch.Success "add-peer did not emit a request ID"
    $contactId = $contactMatch.Groups[1].Value
    $requestId = $requestMatch.Groups[1].Value
    Assert-True (-not [string]::IsNullOrWhiteSpace($contactId)) "empty contact selector"
    Assert-True (-not [string]::IsNullOrWhiteSpace($requestId)) "empty request ID"

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

    Wait-RequestState $clientB $requestId {
        param($request)
        $request.direction -eq "incoming" -and
            $request.delivery.state -eq "received" -and
            $request.decision.state -eq "pending"
    } "incoming received/pending" | Out-Null

    # The inbox is the sole source for the target-side request selector and the
    # exact next command. Acceptance itself intentionally has no arguments.
    $inbox = Invoke-Client -ClientRoot $clientB -Arguments @("share", "request")
    Assert-Success $inbox "B request inbox"
    $inboxLines = [regex]::Split($inbox.Stdout.Trim(), "\r?\n")
    $pendingLine = @($inboxLines | Where-Object { $_ -like "pending_request`t*" })
    Assert-True ($pendingLine.Count -eq 1) "B inbox did not expose exactly one pending request"
    $inboxRequestId = ($pendingLine[0] -split "`t")[1]
    Assert-True ($inboxRequestId -eq $requestId) "B inbox request does not match A add-peer output"
    Assert-True ($inboxLines -contains "pending_requests`t1") "B inbox did not report one pending request"
    Assert-True ($inboxLines -contains "next`tse share request accept") "B inbox did not emit the bare accept command"

    $accepted = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "accept", "--json"
    )) "B bare request accept"
    Assert-True ($accepted.request.decision.state -eq "accepted") "B did not persist accepted state"
    Assert-True ([bool]$accepted.request.authorization.active) "B authorization did not become active"

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.decision.state -eq "accepted" -and
            [bool]$request.authorization.active -and
            $request.connectivity.state -eq "available"
    } "accepted/active/available on requester" | Out-Null
    Wait-RequestState $clientB $requestId {
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
    Assert-True ($stillAccepted.request_id -eq $requestId) "refused delete lost the accepted request"
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
    Assert-True ($revoked.request.request_id -eq $requestId) "base revoke selected an undiscovered authorization"
    Assert-True ($revoked.request.decision.state -eq "revoked") "B did not persist revoked state"
    Assert-True (-not [bool]$revoked.request.authorization.active) "B authorization remained active after revoke"

    Wait-RequestState $clientA $requestId {
        param($request)
        $request.decision.state -eq "revoked" -and -not [bool]$request.authorization.active
    } "revoked/inactive on requester" | Out-Null
    Wait-RequestState $clientB $requestId {
        param($request)
        $request.decision.state -eq "revoked" -and
            -not [bool]$request.authorization.active -and
            $request.peer_receipt.decision.state -eq "received"
    } "revoked/inactive with receipt on target" | Out-Null

    $deleted = Convert-CommandJson (Invoke-Client -ClientRoot $clientB -Arguments @(
        "share", "request", "delete", "--json"
    )) "B context-free post-revoke delete"
    Assert-True ($deleted.action -eq "deleted") "post-revoke request history was not deleted"
    Assert-True ($deleted.request_id -eq $requestId) "post-revoke delete selected an undiscovered request"
    Assert-True ([bool]$deleted.persisted) "post-revoke delete was not persisted"

    $stopA = Stop-ClientWorker $clientA
    Assert-Success $stopA "A worker stop"
    $stopB = Stop-ClientWorker $clientB
    Assert-Success $stopB "B worker stop"
    $workersStopped = $true
    $succeeded = $true
    Write-Host "Windows Share/Exec CLI lifecycle E2E passed: $requestId"
}
finally {
    if (-not $workersStopped) {
        try { Stop-ClientWorker $clientA | Out-Null } catch { }
        try { Stop-ClientWorker $clientB | Out-Null } catch { }
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
