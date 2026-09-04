#requires -Version 5.1
<#
Checks observed Windows mounted-volume behavior using bounded, read-only I/O.
No compilation, installation, remote writes, or automatic mount changes occur.
Exit codes: 0 PASS, 2 INCONCLUSIVE, 3 ERROR, 4 TIMEOUT.
The CLI version does not prove the identity/version of an existing mount host.
#>
[CmdletBinding()]
param(
    [string[]]$Drive = @(),
    [string]$SeBinary,
    [string]$ReportPath,
    [ValidateRange(20, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$clock = [Diagnostics.Stopwatch]::StartNew()
$deadlineSeconds = [double]$TimeoutSeconds
$children = New-Object 'System.Collections.Generic.List[object]'
$survivors = New-Object 'System.Collections.Generic.List[int]'
$results = New-Object 'System.Collections.Generic.List[object]'
$targets = New-Object 'System.Collections.Generic.List[object]'
$overall = 'INCONCLUSIVE'
$reason = 'no_active_managed_mount'
$cliVersion = $null
$reportSaved = $false
$shellExe = [IO.Path]::Combine([Environment]::SystemDirectory, 'WindowsPowerShell\v1.0\powershell.exe')

function Start-Captured {
    param([string]$Executable, [string]$Arguments, [int]$Seconds)
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $Executable
    $start.Arguments = $Arguments
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = [Environment]::SystemDirectory
    [void]$start.EnvironmentVariables.Remove('COMPLETE')
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'process_start_failed' }
    $child = [pscustomobject]@{
        Process = $process; Pid = $process.Id
        Out = $process.StandardOutput.ReadToEndAsync()
        Err = $process.StandardError.ReadToEndAsync()
        Deadline = [Math]::Min($deadlineSeconds, $clock.Elapsed.TotalSeconds + $Seconds)
        Finished = $false; TimedOut = $false; ExitCode = -1; Text = ''
    }
    [void]$children.Add($child)
    return $child
}

function Start-Worker {
    param([string]$Body, [object]$Payload, [int]$Seconds)
    $payloadText = ConvertTo-Json -InputObject $Payload -Depth 12 -Compress
    $payload64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($payloadText))
    $prefix = '$ErrorActionPreference = ''Stop''; $ProgressPreference = ''SilentlyContinue''; ' +
        '$inputData = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String(''' +
        $payload64 + ''')) | ConvertFrom-Json; ' + [Environment]::NewLine
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($prefix + $Body))
    return Start-Captured $shellExe "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded" $Seconds
}

function Stop-Captured {
    param([object]$Child)
    # Kill only a process this invocation started; never wait indefinitely for
    # a filesystem request blocked in the kernel to permit process termination.
    try {
        if (-not $Child.Process.WaitForExit(0)) { $Child.Process.Kill() }
        if (-not $Child.Process.WaitForExit(100)) {
            if (-not $survivors.Contains($Child.Pid)) { [void]$survivors.Add($Child.Pid) }
        }
    } catch {
        if (-not $survivors.Contains($Child.Pid)) { [void]$survivors.Add($Child.Pid) }
    }
}

function Wait-Captured {
    param([object[]]$Group)
    while (@($Group | Where-Object { -not $_.Finished }).Count -gt 0) {
        foreach ($child in $Group) {
            if ($child.Finished) { continue }
            $exited = $child.Process.WaitForExit(0)
            if ($exited -and $child.Out.IsCompleted -and $child.Err.IsCompleted) {
                $child.ExitCode = $child.Process.ExitCode
                if ($child.Out.Status -eq [Threading.Tasks.TaskStatus]::RanToCompletion) {
                    $child.Text = $child.Out.Result
                }
                $child.Finished = $true
            } elseif ($clock.Elapsed.TotalSeconds -ge $child.Deadline) {
                $child.TimedOut = $true
                $child.Finished = $true
                Stop-Captured $child
            }
        }
        if (@($Group | Where-Object { -not $_.Finished }).Count -gt 0) {
            [Threading.Thread]::Sleep(40)
        }
    }
}

$discover = @'
try {
    $candidate = [string]$inputData.SeBinary
    if (-not [string]::IsNullOrWhiteSpace($candidate) -and -not [IO.Path]::IsPathRooted($candidate)) {
        $candidate = [IO.Path]::Combine([string]$inputData.BaseDirectory, $candidate)
    }
    if ([string]::IsNullOrWhiteSpace($candidate)) {
        $command = Get-Command se.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -ne $command) { $candidate = $command.Source }
    }
    if ([string]::IsNullOrWhiteSpace($candidate)) {
        $candidate = [IO.Path]::Combine([Environment]::GetFolderPath('LocalApplicationData'), 'Programs\Smart Explorer\se.exe')
    }
    if (-not [IO.File]::Exists($candidate)) { throw 'cli_missing' }
    $candidate = [IO.Path]::GetFullPath($candidate)
    $systemDrive = [IO.Path]::GetPathRoot([Environment]::SystemDirectory)
    if (-not $candidate.StartsWith($systemDrive, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'cli_requires_plain_system_drive_path'
    }
    # CreateProcess opens its executable synchronously before a child deadline
    # can apply. Never hand the parent a CLI on a Dokany/UNC/reparse path.
    # All potentially blocking path inspection stays in this bounded worker.
    $cursor = $candidate
    while (-not [string]::IsNullOrEmpty($cursor)) {
        if (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'cli_requires_plain_system_drive_path'
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    [pscustomobject]@{ executable = $candidate } | ConvertTo-Json -Compress
    exit 0
} catch { '{"error":"cli_discovery_failed"}'; exit 3 }
'@

$probe = @'
$stats = [ordered]@{
    worker = [int]$inputData.Worker; outcome = 'INCONCLUSIVE'; error_code = $null
    rounds = 0; directories = 0; unique_directories = 0; metadata = 0
    files_read = 0; bytes_read = 0; links_skipped = 0; capped = $false
}
$seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
$root = ([string]$inputData.Drive) + ':\'
try {
    for ($round = 0; $round -lt 3; $round++) {
        $queue = New-Object 'System.Collections.Generic.Queue[object]'
        $queue.Enqueue([pscustomobject]@{ Path = $root; Depth = 0 })
        $roundDirs = 0
        $roundEntries = 0
        $roundReads = 0
        while ($queue.Count -gt 0 -and $roundDirs -lt 18 -and $roundEntries -lt 1024) {
            $node = $queue.Dequeue()
            # Inspect every ancestor again before opening a descendant. No
            # recursive enumeration API is used, and links are never queued.
            $relative = $node.Path.Substring($root.Length)
            $cursor = $root
            $safe = $true
            foreach ($part in @('') + @($relative.Split('\') | Where-Object { $_.Length -gt 0 })) {
                if ($part.Length -gt 0) { $cursor = [IO.Path]::Combine($cursor, $part) }
                $attributes = [IO.File]::GetAttributes($cursor)
                if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { $safe = $false; break }
            }
            if (-not $safe) { $stats.links_skipped++; continue }
            $directory = New-Object IO.DirectoryInfo $node.Path
            $directory.Refresh()
            [void]$directory.LastWriteTimeUtc
            $stats.metadata++
            $iterator = $directory.EnumerateFileSystemInfos().GetEnumerator()
            $dirEntries = 0
            try {
                while ($dirEntries -lt 128 -and $roundEntries -lt 1024 -and $iterator.MoveNext()) {
                    $item = $iterator.Current
                    $dirEntries++
                    $roundEntries++
                    $attributes = $item.Attributes
                    if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                        $stats.links_skipped++
                        continue
                    }
                    [void]$item.LastWriteTimeUtc
                    $stats.metadata++
                    if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                        if ($node.Depth -lt 6 -and $queue.Count -lt 64) {
                            $queue.Enqueue([pscustomobject]@{ Path = $item.FullName; Depth = $node.Depth + 1 })
                        } else { $stats.capped = $true }
                    } elseif (($attributes -band [IO.FileAttributes]::Device) -eq 0) {
                        $length = $item.Length
                        # A mount may download the whole file on first open.
                        # Only already-small files are sampled; contents never
                        # leave this worker. Parent timeout also covers open.
                        if ($length -gt 0 -and $length -le 65536 -and $roundReads -lt 2) {
                            $fresh = [IO.File]::GetAttributes($item.FullName)
                            if (($fresh -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                                $stats.links_skipped++
                                continue
                            }
                            $stream = [IO.FileStream]::new($item.FullName, [IO.FileMode]::Open,
                                [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
                            $roundReads++
                            try {
                                $buffer = New-Object byte[] 4096
                                $read = $stream.Read($buffer, 0, $buffer.Length)
                                if ($read -gt 0) { $stats.files_read++; $stats.bytes_read += $read }
                            } finally { $stream.Dispose() }
                        }
                    }
                }
                if ($dirEntries -ge 128 -or $roundEntries -ge 1024) { $stats.capped = $true }
            } finally { $iterator.Dispose() }
            $stats.directories++
            [void]$seen.Add($node.Path)
            $roundDirs++
        }
        if ($queue.Count -gt 0) { $stats.capped = $true }
        $stats.rounds++
    }
    $stats.unique_directories = $seen.Count
    if ($stats.rounds -eq 3 -and $seen.Count -ge 5 -and $stats.files_read -ge 1 -and $stats.bytes_read -gt 0) {
        $stats.outcome = 'PASS'
    }
} catch {
    $stats.outcome = 'ERROR'
    # Exception messages and paths may contain account or filename details.
    $stats.error_code = '0x{0:X8}' -f ($_.Exception.HResult -band 0xffffffffL)
    $stats.unique_directories = $seen.Count
}
[pscustomobject]$stats | ConvertTo-Json -Compress
if ($stats.outcome -eq 'PASS') { exit 0 }
if ($stats.outcome -eq 'ERROR') { exit 3 }
exit 2
'@

try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'windows_required' }
    if (@($Drive).Count -gt 0) {
        foreach ($value in $Drive) {
            if ($value -notmatch '^[A-Za-z]:?[\\/]?$') { throw 'invalid_drive_letter' }
            $letter = $value.Substring(0, 1).ToUpperInvariant()
            if (@($targets | Where-Object { $_.drive -eq $letter }).Count -eq 0) {
                [void]$targets.Add([pscustomobject]@{ drive = $letter; mode = 'Unspecified'; state = 'Explicit' })
            }
        }
        if ($targets.Count -gt 3) { throw 'too_many_drives' }
    } else {
        $discovery = Start-Worker $discover @{ SeBinary = $SeBinary; BaseDirectory = [string]$PWD.ProviderPath } 10
        Wait-Captured @($discovery)
        if ($discovery.TimedOut) { $overall = 'TIMEOUT'; throw 'discovery_timeout' }
        if ($discovery.ExitCode -ne 0) { throw 'cli_discovery_failed' }
        $executable = [string](ConvertFrom-Json $discovery.Text).executable
        if ([string]::IsNullOrWhiteSpace($executable)) { throw 'cli_discovery_failed' }
        $version = Start-Captured $executable '--version' 10
        $listing = Start-Captured $executable 'drive list --json' 15
        Wait-Captured @($version, $listing)
        if ($listing.TimedOut) { $overall = 'TIMEOUT'; throw 'mount_discovery_timeout' }
        if ($listing.ExitCode -ne 0 -or $listing.Text.Length -gt 4194304) { throw 'mount_discovery_failed' }
        if ($version.ExitCode -eq 0 -and $version.Text.Trim() -match '^se ([0-9]+\.[0-9]+\.[0-9]+)(?:\r?\n)?$') {
            $cliVersion = $Matches[1]
        }
        $mounts = @(ConvertFrom-Json $listing.Text)
        foreach ($mount in $mounts) {
            if ($null -eq $mount -or $mount.status -is [string]) { continue }
            $mounted = $mount.status.PSObject.Properties['Mounted']
            if ($null -eq $mounted) { continue }
            $letter = [string]$mounted.Value.drive
            $mode = [string]$mount.config.mode
            if ($letter -notmatch '^[A-Z]$' -or $mode -notin @('ReadOnly', 'ReadWrite')) { throw 'invalid_mount_snapshot' }
            if (@($targets | Where-Object { $_.drive -eq $letter }).Count -eq 0 -and $targets.Count -lt 3) {
                [void]$targets.Add([pscustomobject]@{ drive = $letter; mode = $mode; state = 'Mounted' })
            }
        }
    }
    foreach ($target in $targets) {
        if ($clock.Elapsed.TotalSeconds -ge ($TimeoutSeconds - 5)) {
            $overall = 'TIMEOUT'; $reason = 'overall_deadline'; break
        }
        Write-Host ("Checking {0}: with four read-only workers..." -f $target.drive)
        $group = @()
        for ($number = 1; $number -le 4; $number++) {
            $group += Start-Worker $probe @{ Drive = $target.drive; Worker = $number } 45
        }
        Wait-Captured $group
        $workerReports = @()
        foreach ($worker in $group) {
            if ($worker.TimedOut) {
                $workerReports += [pscustomobject]@{ worker = $workerReports.Count + 1; outcome = 'TIMEOUT'; pid = $worker.Pid }
            } elseif ($worker.ExitCode -in @(0, 2, 3) -and $worker.Text.Length -le 16384) {
                $parsed = ConvertFrom-Json $worker.Text
                # Only the fixed worker schema is copied into the public report.
                $workerReports += [pscustomobject]@{
                    worker = [int]$parsed.worker; outcome = [string]$parsed.outcome
                    error_code = $parsed.error_code; rounds = [int]$parsed.rounds
                    directories = [int]$parsed.directories; unique_directories = [int]$parsed.unique_directories
                    metadata = [int]$parsed.metadata; files_read = [int]$parsed.files_read
                    bytes_read = [int]$parsed.bytes_read; links_skipped = [int]$parsed.links_skipped
                    capped = [bool]$parsed.capped
                }
            } else {
                $workerReports += [pscustomobject]@{ worker = $workerReports.Count + 1; outcome = 'ERROR'; error_code = 'worker_failed' }
            }
        }
        $driveOutcome = 'PASS'
        if (@($workerReports | Where-Object { $_.outcome -ne 'PASS' }).Count -gt 0) { $driveOutcome = 'INCONCLUSIVE' }
        if (@($workerReports | Where-Object { $_.outcome -eq 'ERROR' }).Count -gt 0) { $driveOutcome = 'ERROR' }
        if (@($workerReports | Where-Object { $_.outcome -eq 'TIMEOUT' }).Count -gt 0) { $driveOutcome = 'TIMEOUT' }
        [void]$results.Add([pscustomobject]@{ drive = $target.drive; mode = $target.mode; outcome = $driveOutcome; workers = $workerReports })
        if ($driveOutcome -eq 'TIMEOUT') { $overall = 'TIMEOUT'; $reason = 'filesystem_deadline'; break }
    }
    if ($overall -ne 'TIMEOUT' -and $results.Count -gt 0) {
        $overall = 'PASS'; $reason = 'bounded_navigation_and_reads_completed'
        if (@($results | Where-Object { $_.outcome -eq 'INCONCLUSIVE' }).Count -gt 0) {
            $overall = 'INCONCLUSIVE'; $reason = 'need_five_directories_and_small_nonempty_file'
        }
        if (@($results | Where-Object { $_.outcome -eq 'ERROR' }).Count -gt 0) {
            $overall = 'ERROR'; $reason = 'filesystem_operation_failed'
        }
    }
} catch {
    if ($overall -ne 'TIMEOUT') { $overall = 'ERROR' }
    $reason = 'checker_or_discovery_failed'
} finally {
    foreach ($child in $children) {
        if (-not $child.Finished) { Stop-Captured $child }
    }
}

$report = [ordered]@{
    schema = 1; outcome = $overall; reason = $reason
    elapsed_seconds = [Math]::Round($clock.Elapsed.TotalSeconds, 2)
    cli_version = $cliVersion; verifies_existing_host_identity = $false
    read_only_probe = $true; drives = @($results.ToArray())
    surviving_worker_pids = @($survivors.ToArray())
    coverage = 'Four workers; three rounds each; at least five directories and one small nonempty file per worker for PASS.'
}

if (-not [string]::IsNullOrWhiteSpace($ReportPath)) {
    # Isolate report-path inspection/writing too. A supplied path might cross a
    # network/reparse boundary; it must not hang the supervising console.
    $saveReport = @'
try {
    $path = [IO.Path]::GetFullPath([IO.Path]::Combine([string]$inputData.BaseDirectory, [string]$inputData.Path))
    if ($path -notmatch '^[A-Za-z]:\\' -or $path.IndexOf(':', 2) -ge 0) { throw 'local_path_required' }
    $root = [IO.Path]::GetPathRoot($path)
    if (@($inputData.BlockedDrives) -contains $root.Substring(0, 1).ToUpperInvariant()) { throw 'mounted_report_path' }
    $driveInfo = New-Object IO.DriveInfo $root
    if ($driveInfo.DriveType -ne [IO.DriveType]::Fixed -and $driveInfo.DriveType -ne [IO.DriveType]::Removable) { throw 'local_storage_required' }
    $directory = [IO.Path]::GetDirectoryName($path)
    $cursor = $root
    foreach ($part in @('') + @($directory.Substring($root.Length).Split('\') | Where-Object { $_.Length -gt 0 })) {
        if ($part.Length -gt 0) { $cursor = [IO.Path]::Combine($cursor, $part) }
        if (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'report_link_ancestor' }
    }
    # CreateNew prevents overwriting any existing file or following its link.
    $stream = [IO.FileStream]::new($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes([string]$inputData.Report)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush()
    } finally { $stream.Dispose() }
    '{"saved":true}'
    exit 0
} catch { '{"saved":false}'; exit 3 }
'@
    try {
        # Reserve at most five additional seconds solely for local report I/O.
        $deadlineSeconds = $clock.Elapsed.TotalSeconds + 5
        $saving = Start-Worker $saveReport @{
            Path = $ReportPath; BaseDirectory = [string]$PWD.ProviderPath
            BlockedDrives = @($targets | ForEach-Object { $_.drive })
            Report = (ConvertTo-Json -InputObject $report -Depth 12)
        } 5
        Wait-Captured @($saving)
        $reportSaved = (-not $saving.TimedOut -and $saving.ExitCode -eq 0)
    } catch { $reportSaved = $false }
    if (-not $reportSaved) { Write-Host 'Local JSON report could not be saved (use a new file on local storage).' }
}

foreach ($child in $children) {
    if (-not $child.Finished) { Stop-Captured $child }
}
$report.surviving_worker_pids = @($survivors.ToArray())
Write-Output (ConvertTo-Json -InputObject $report -Depth 12)
Write-Host 'This checks observed mounted-drive behavior, not the identity/version of the existing mount host.'
if ($survivors.Count -gt 0) {
    Write-Host ('Worker termination is still pending; PIDs: ' + ($survivors.ToArray() -join ', '))
}
if ($overall -eq 'TIMEOUT') { exit 4 }
if ($overall -eq 'ERROR') { exit 3 }
if (-not [string]::IsNullOrWhiteSpace($ReportPath) -and -not $reportSaved) { exit 3 }
if ($overall -eq 'INCONCLUSIVE') { exit 2 }
exit 0
