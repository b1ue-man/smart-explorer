# Cross-host complete-release lock shared by Windows, WSL, and Linux.
# The matching Bash implementation is native/release-lock.sh.

function Enter-CompleteReleaseLock {
    param(
        [Parameter(Mandatory = $true)][string]$ReleaseRoot,
        [Parameter(Mandatory = $true)][string]$Owner
    )

    New-Item -ItemType Directory -Force $ReleaseRoot | Out-Null
    $path = Join-Path $ReleaseRoot ".complete-release.lock"
    $token = "{0}-{1}" -f ([guid]::NewGuid().ToString("N")), $PID
    $stream = $null
    try {
        $stream = [System.IO.File]::Open(
            $path,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::Read
        )
    } catch [System.IO.IOException] {
        $metadata = try {
            (Get-Content -LiteralPath $path -ErrorAction Stop | Select-Object -First 12) -join [Environment]::NewLine
        } catch {
            "(owner metadata is unreadable)"
        }
        throw "Another complete release already owns ${path}:`n$metadata`nIf the owner crashed, verify that no Windows, WSL, or Linux release process remains, then remove only this stale lock file."
    }

    try {
        $metadata = @(
            "token=$token"
            "owner=$Owner"
            "pid=$PID"
            "host=$([Environment]::MachineName)"
            "started_utc=$([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))"
        ) -join "`n"
        $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes("$metadata`n")
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        return [pscustomobject]@{
            Path = $path
            Token = $token
            Stream = $stream
        }
    } catch {
        $stream.Dispose()
        Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        throw
    }
}

function Exit-CompleteReleaseLock {
    param([Parameter(Mandatory = $true)]$Lock)

    $Lock.Stream.Dispose()
    $firstLine = try {
        Get-Content -LiteralPath $Lock.Path -TotalCount 1 -ErrorAction Stop
    } catch {
        ""
    }
    if ($firstLine -ne "token=$($Lock.Token)") {
        throw "Complete-release lock ownership changed; refusing to remove $($Lock.Path)."
    }
    Remove-Item -LiteralPath $Lock.Path -Force
}
