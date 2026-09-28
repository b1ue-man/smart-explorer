# Diagnostic evidence for this isolated Windows task process only.
function Get-WindowsRemoteTaskDumpTool {
    param([string]$LogDirectory)
    $folder = Join-Path $LogDirectory 'dump-tool'
    [void][IO.Directory]::CreateDirectory($folder)
    $archive = Join-Path $folder 'Procdump.zip'
    Invoke-WebRequest -Uri 'https://download.sysinternals.com/files/Procdump.zip' -OutFile $archive -TimeoutSec 120
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ine
        '68e057587b0fd654efa095f76d80d633c0e5c60ea26fd3e7c0011c076bb2d00c') {
        throw 'ProcDump archive differs from the diagnostic version reviewed on 2026-09-28.'
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $folder
    $executable = Join-Path $folder 'procdump64.exe'
    if ((Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash -ine
        'd1fc99ae304bd1d2bf28abeb62531da959e2431916194981b88c958fd713a8e6') {
        throw 'ProcDump executable hash differs.'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $executable
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'ProcDump requires a valid Microsoft Authenticode signature.'
    }
    return $executable
}

function Start-WindowsRemoteTaskDumpMonitor {
    param([string]$Executable, [int]$ProcessId, [string]$LogDirectory)
    $folder = Join-Path $LogDirectory 'crash-dumps'
    [void][IO.Directory]::CreateDirectory($folder)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [Text.Encoding]::Unicode
    $start.StandardErrorEncoding = [Text.Encoding]::Unicode
    foreach ($argument in @('-accepteula', '-mm', '-e', '-n', '1', '-at', '15', [string]$ProcessId, $folder)) {
        $start.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Native exception monitor did not start.' }
        return [pscustomobject]@{
            Process = $process
            Output = $process.StandardOutput.ReadToEndAsync()
            Error = $process.StandardError.ReadToEndAsync()
        }
    } catch {
        $process.Dispose()
        throw
    }
}

function Stop-WindowsRemoteTaskDumpMonitor {
    param($Monitor, [string]$LogDirectory)
    if ($null -eq $Monitor) { return }
    try {
        # Called only after the owned fixture has terminated. Never kill or
        # attach to any other process; the fixture's real exit remains decisive.
        if (-not $Monitor.Process.WaitForExit(30000)) {
            $Monitor.Process.Kill()
            [void]$Monitor.Process.WaitForExit(5000)
        }
        foreach ($entry in @(@('stdout', $Monitor.Output), @('stderr', $Monitor.Error))) {
            if ($entry[1].Wait(5000)) {
                [IO.File]::WriteAllText((Join-Path $LogDirectory "dump-monitor.$($entry[0]).log"), [string]$entry[1].Result)
            }
        }
    } finally { $Monitor.Process.Dispose() }
}

function Enable-WindowsRemoteTaskDump {
    param([string]$Executable, [string]$LogDirectory)
    $folder = Join-Path $LogDirectory 'crash-dumps'
    [void][IO.Directory]::CreateDirectory($folder)
    $key = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\' + [IO.Path]::GetFileName($Executable)
    if (Test-Path -LiteralPath $key) { throw "Unexpected existing task-specific dump policy: $key" }
    [void](New-Item -Path $key -Force)
    [void](New-ItemProperty -LiteralPath $key -Name DumpFolder -Value $folder -PropertyType ExpandString)
    [void](New-ItemProperty -LiteralPath $key -Name DumpCount -Value 2 -PropertyType DWord)
    [void](New-ItemProperty -LiteralPath $key -Name DumpType -Value 1 -PropertyType DWord)
    return $key
}

function Save-WindowsRemoteTaskExit {
    param([int]$Code, [int]$ProcessId, [datetime]$Started, [string]$Label, [string]$LogDirectory, [bool]$Fixture)
    $unsigned = [BitConverter]::ToUInt32([BitConverter]::GetBytes($Code), 0)
    $hex = '0x{0:X8}' -f $unsigned
    [ordered]@{ code = $Code; hex = $hex; pid = $ProcessId; started = $Started.ToUniversalTime().ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogDirectory "$Label.exit.json")
    if ($Fixture -and $Code -ne 0) {
        Write-Host "$Label exited with $Code ($hex), PID $ProcessId."
        $filter = @{
            LogName = 'Application'; Id = @(1000, 1001); StartTime = $Started
        }
        $events = @(Get-WinEvent -FilterHashtable $filter -MaxEvents 20 -ErrorAction SilentlyContinue |
            Select-Object -Property @('TimeCreated', 'Id', 'ProviderName', 'Message'))
        ConvertTo-Json -InputObject $events -Depth 4 |
            Set-Content -LiteralPath (Join-Path $LogDirectory "$Label.windows-events.json")
    }
}
