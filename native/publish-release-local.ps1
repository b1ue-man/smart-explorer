# Build a complete local Smart Explorer release on Windows with WSL available.
#
# Windows and Linux outputs are built into one isolated release tree. The live
# release-native artifacts are changed only after every staged artifact passes
# validation; version.txt is the final commit marker.

param(
    [switch]$SkipLinuxFeed,
    [switch]$NoBootstrapZig,
    [switch]$CheckEnvOnly
)

$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptRoot "..")).Path
$releaseRoot = Join-Path $repoRoot "release-native"
$feed = Join-Path $releaseRoot "update-feed"
. (Join-Path $scriptRoot "release-lock.ps1")

function Get-NativeVersion {
    $cargoToml = Join-Path $scriptRoot "Cargo.toml"
    $match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $match) {
        throw "Could not read version from $cargoToml"
    }
    return $match.Matches[0].Groups[1].Value
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Command,
        [Parameter(Mandatory = $true)][string]$ErrorMessage
    )
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw $ErrorMessage
    }
}

function Assert-NonEmptyFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required release artifact missing: $Path"
    }
    if ((Get-Item -LiteralPath $Path).Length -le 0) {
        throw "Required release artifact is empty: $Path"
    }
}

function Assert-FeedHash([string]$FeedDirectory, [string]$PayloadName) {
    $payload = Join-Path $FeedDirectory $PayloadName
    $sidecar = "$payload.sha256"
    Assert-NonEmptyFile $payload
    Assert-NonEmptyFile $sidecar
    $expected = ((Get-Content -LiteralPath $sidecar -TotalCount 1) -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $payload).Hash.ToLowerInvariant()
    if ($expected -notmatch '^[0-9a-f]{64}$' -or $expected -ne $actual) {
        throw "Feed SHA256 mismatch for $PayloadName (expected $expected, got $actual)."
    }
}

function Assert-SameSha256([string]$Left, [string]$Right) {
    Assert-NonEmptyFile $Left
    Assert-NonEmptyFile $Right
    $leftHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Left).Hash
    $rightHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Right).Hash
    if ($leftHash -ne $rightHash) {
        throw "Release artifacts differ: $Left and $Right"
    }
}

function Assert-WindowsManifest([string]$FeedDirectory, [string]$ExpectedVersion) {
    $manifestPath = Join-Path $FeedDirectory "windows-build.manifest"
    Assert-NonEmptyFile $manifestPath
    $entries = @{}
    foreach ($line in Get-Content -LiteralPath $manifestPath) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        $parts = $line.Split('=', 2)
        if ($parts.Count -ne 2 -or [string]::IsNullOrWhiteSpace($parts[0]) -or $entries.ContainsKey($parts[0])) {
            throw "Invalid or duplicate entry in Windows build manifest: $line"
        }
        $entries[$parts[0]] = $parts[1].Trim()
    }
    if ($entries["version"] -ne $ExpectedVersion) {
        throw "Windows build manifest version '$($entries["version"])' does not match '$ExpectedVersion'."
    }
    foreach ($name in @("smart_explorer.exe", "smart_explorer_updater.exe", "se.exe")) {
        Assert-FeedHash $FeedDirectory $name
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $FeedDirectory $name)).Hash.ToLowerInvariant()
        $manifestHash = $entries[$name].ToLowerInvariant()
        if ($manifestHash -notmatch '^[0-9a-f]{64}$' -or $manifestHash -ne $actual) {
            throw "Windows build manifest SHA256 mismatch for $name."
        }
    }
}

function Assert-ContextDll([string]$Path, $Inspector) {
    Assert-NonEmptyFile $Path
    $exportOutput = if ($Inspector.Name -eq "dumpbin.exe") {
        (& $Inspector.Source /exports $Path 2>&1 | Out-String)
    } else {
        (& $Inspector.Source -p $Path 2>&1 | Out-String)
    }
    if ($LASTEXITCODE -ne 0 -or
        $exportOutput -notmatch 'DllGetClassObject' -or
        $exportOutput -notmatch 'DllCanUnloadNow') {
        throw "Context-menu DLL export verification failed: $Path"
    }
}

function Publish-FileAtomic([string]$Source, [string]$Destination) {
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force $parent | Out-Null
    $temporary = Join-Path $parent (".{0}.{1}.{2}.tmp" -f ([System.IO.Path]::GetFileName($Destination)), $PID, [guid]::NewGuid().ToString("N"))
    try {
        Copy-Item -LiteralPath $Source -Destination $temporary -Force
        Assert-NonEmptyFile $temporary
        if (Test-Path -LiteralPath $Destination -PathType Leaf) {
            [System.IO.File]::Replace($temporary, $Destination, $null)
        } else {
            [System.IO.File]::Move($temporary, $Destination)
        }
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Publish-PartialBundle([string]$StageRelease, [string]$Destination) {
    $parent = Split-Path -Parent $Destination
    $backup = Join-Path $parent (".{0}.backup.{1}.{2}" -f ([System.IO.Path]::GetFileName($Destination)), $PID, [guid]::NewGuid().ToString("N"))
    $hadDestination = Test-Path -LiteralPath $Destination
    $installed = $false
    try {
        if ($hadDestination) {
            Move-Item -LiteralPath $Destination -Destination $backup
        }
        $installed = $true
        Move-Item -LiteralPath $StageRelease -Destination $Destination
        if (Test-Path -LiteralPath (Join-Path $Destination "update-feed\version.txt")) {
            throw "A non-publishable Windows bundle must not contain update-feed/version.txt."
        }
        if ($hadDestination) {
            Remove-Item -LiteralPath $backup -Recurse -Force
        }
    } catch {
        $primary = $_
        try {
            if ($installed -and (Test-Path -LiteralPath $Destination)) {
                Remove-Item -LiteralPath $Destination -Recurse -Force
            }
            if ($hadDestination -and (Test-Path -LiteralPath $backup)) {
                Move-Item -LiteralPath $backup -Destination $Destination
            }
        } catch {
            throw "Partial-bundle publication failed ($primary); rollback failed: $_"
        }
        throw $primary
    }
}

function Publish-CompleteRelease(
    [string]$StageRelease,
    [string]$StageFeed,
    [string]$VersionSource,
    [string]$ExpectedVersion
) {
    $artifactSpecs = @(
        [pscustomobject]@{ Source = Join-Path $StageRelease "Smart Explorer.exe"; Destination = Join-Path $releaseRoot "Smart Explorer.exe" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "Smart Explorer Updater.exe"; Destination = Join-Path $releaseRoot "Smart Explorer Updater.exe" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "se.exe"; Destination = Join-Path $releaseRoot "se.exe" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "Smart Explorer Setup $ExpectedVersion.exe"; Destination = Join-Path $releaseRoot "Smart Explorer Setup $ExpectedVersion.exe" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "smart_explorer_command.dll"; Destination = Join-Path $releaseRoot "smart_explorer_command.dll" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "share-server\se-share-server.exe"; Destination = Join-Path $releaseRoot "share-server\se-share-server.exe" },
        [pscustomobject]@{ Source = Join-Path $StageRelease "share-server\se-share-server-linux"; Destination = Join-Path $releaseRoot "share-server\se-share-server-linux" }
    )
    $records = [System.Collections.Generic.List[object]]::new()
    $feedBackup = Join-Path $releaseRoot (".update-feed.release-backup.{0}.{1}" -f $PID, [guid]::NewGuid().ToString("N"))
    $feedCandidate = Join-Path $releaseRoot (".update-feed.release-new.{0}.{1}" -f $PID, [guid]::NewGuid().ToString("N"))
    $feedHadDestination = Test-Path -LiteralPath $feed
    $feedInstalled = $false
    $committed = $false
    try {
        foreach ($spec in $artifactSpecs) {
            Assert-NonEmptyFile $spec.Source
            $destinationParent = Split-Path -Parent $spec.Destination
            New-Item -ItemType Directory -Force $destinationParent | Out-Null
            $record = [pscustomobject]@{
                Destination = $spec.Destination
                Backup = Join-Path $destinationParent (".{0}.release-backup.{1}.{2}" -f ([System.IO.Path]::GetFileName($spec.Destination)), $PID, [guid]::NewGuid().ToString("N"))
                Candidate = Join-Path $destinationParent (".{0}.release-new.{1}.{2}" -f ([System.IO.Path]::GetFileName($spec.Destination)), $PID, [guid]::NewGuid().ToString("N"))
                HadDestination = Test-Path -LiteralPath $spec.Destination
                Installed = $false
            }
            $records.Add($record)
            Copy-Item -LiteralPath $spec.Source -Destination $record.Candidate
            Assert-SameSha256 $spec.Source $record.Candidate
            if ($record.HadDestination) {
                Move-Item -LiteralPath $record.Destination -Destination $record.Backup
            }
            $record.Installed = $true
            Move-Item -LiteralPath $record.Candidate -Destination $record.Destination
        }

        Copy-Item -LiteralPath $StageFeed -Destination $feedCandidate -Recurse
        Assert-WindowsManifest $feedCandidate $ExpectedVersion
        foreach ($name in @("smart_explorer", "smart_explorer_updater", "se")) {
            Assert-FeedHash $feedCandidate $name
        }
        if ($feedHadDestination) {
            Move-Item -LiteralPath $feed -Destination $feedBackup
        }
        $feedInstalled = $true
        Move-Item -LiteralPath $feedCandidate -Destination $feed
        Publish-FileAtomic $VersionSource (Join-Path $feed "version.txt")

        $publishedVersion = (Get-Content -LiteralPath (Join-Path $feed "version.txt") -TotalCount 1).Trim()
        if ($publishedVersion -ne $ExpectedVersion) {
            throw "Published feed version '$publishedVersion' does not match '$ExpectedVersion'."
        }
        Assert-WindowsManifest $feed $ExpectedVersion
        foreach ($name in @("smart_explorer", "smart_explorer_updater", "se")) {
            Assert-FeedHash $feed $name
        }
        foreach ($record in $records) {
            Assert-NonEmptyFile $record.Destination
        }
        $committed = $true
    } catch {
        $primary = $_
        $rollbackErrors = [System.Collections.Generic.List[System.Exception]]::new()
        $attemptRollback = {
            param(
                [Parameter(Mandatory = $true)][string]$Description,
                [Parameter(Mandatory = $true)][scriptblock]$Action
            )
            try {
                & $Action
            } catch {
                $rollbackErrors.Add(
                    [System.InvalidOperationException]::new(
                        "Rollback step failed: $Description",
                        $_.Exception
                    )
                )
            }
        }

        & $attemptRollback "remove newly installed feed '$feed'" {
            if ($feedInstalled -and (Test-Path -LiteralPath $feed)) {
                Remove-Item -LiteralPath $feed -Recurse -Force
            }
        }
        & $attemptRollback "restore prior feed from '$feedBackup'" {
            if ($feedHadDestination -and (Test-Path -LiteralPath $feedBackup)) {
                if (Test-Path -LiteralPath $feed) {
                    throw "Cannot restore the prior feed while the newly installed feed still exists: $feed"
                }
                Move-Item -LiteralPath $feedBackup -Destination $feed
            }
        }
        for ($index = $records.Count - 1; $index -ge 0; $index--) {
            $record = $records[$index]
            & $attemptRollback "remove newly installed artifact '$($record.Destination)'" {
                if ($record.Installed -and (Test-Path -LiteralPath $record.Destination)) {
                    Remove-Item -LiteralPath $record.Destination -Force
                }
            }
            & $attemptRollback "restore prior artifact '$($record.Destination)' from '$($record.Backup)'" {
                if ($record.HadDestination -and (Test-Path -LiteralPath $record.Backup)) {
                    if (Test-Path -LiteralPath $record.Destination) {
                        throw "Cannot restore the prior artifact while the newly installed artifact still exists: $($record.Destination)"
                    }
                    Move-Item -LiteralPath $record.Backup -Destination $record.Destination
                }
            }
        }

        if ($rollbackErrors.Count -gt 0) {
            $allErrors = [System.Collections.Generic.List[System.Exception]]::new()
            $allErrors.Add($primary.Exception)
            foreach ($rollbackError in $rollbackErrors) {
                $allErrors.Add($rollbackError)
            }
            throw [System.AggregateException]::new(
                "Complete-release publication failed and $($rollbackErrors.Count) rollback step(s) also failed. The first inner exception is the primary publication failure; every remaining inner exception is a rollback failure.",
                $allErrors
            )
        }
        throw $primary
    } finally {
        if ($committed) {
            if ($feedHadDestination) {
                Remove-Item -LiteralPath $feedBackup -Recurse -Force -ErrorAction SilentlyContinue
            }
            foreach ($record in $records) {
                if ($record.HadDestination) {
                    Remove-Item -LiteralPath $record.Backup -Force -ErrorAction SilentlyContinue
                }
            }
        }
        if (Test-Path -LiteralPath $feedCandidate) {
            Remove-Item -LiteralPath $feedCandidate -Recurse -Force -ErrorAction SilentlyContinue
        }
        foreach ($record in $records) {
            if (Test-Path -LiteralPath $record.Candidate) {
                Remove-Item -LiteralPath $record.Candidate -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

$version = Get-NativeVersion
$action = if ($CheckEnvOnly) { "Checking" } else { "Building" }
Write-Host "$action complete local release v$version ..."

$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Strawberry\c\bin;C:\Program Files (x86)\NSIS;$env:Path"

$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if (-not $cargo) {
    throw "cargo.exe not found. Install Rust for Windows or fix PATH."
}
if ($CheckEnvOnly) {
    & cargo fmt --version | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "cargo fmt is not available. Install rustfmt for the Windows Rust toolchain."
    }
    & cargo clippy --version | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "cargo clippy is not available. Install clippy for the Windows Rust toolchain."
    }
}
$makensis = Get-Command makensis.exe -ErrorAction SilentlyContinue
if (-not $makensis) {
    $nsisCandidates = @(
        "$env:ProgramFiles\NSIS\makensis.exe",
        "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
    )
    $makensis = $nsisCandidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $makensis) {
    throw "makensis.exe not found; a release without an installer is incomplete."
}
$peInspector = @("llvm-objdump.exe", "objdump.exe", "dumpbin.exe") |
    ForEach-Object { Get-Command $_ -ErrorAction SilentlyContinue } |
    Select-Object -First 1
if (-not $peInspector) {
    throw "No PE export inspector found (llvm-objdump.exe, objdump.exe, or dumpbin.exe)."
}

if ($CheckEnvOnly -and $SkipLinuxFeed) {
    Write-Host "Windows release environment OK for v$version."
    exit 0
}

$repoRootWsl = ""
if ($CheckEnvOnly -or -not $SkipLinuxFeed) {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) {
        throw "wsl.exe not found. Install WSL or rerun with -SkipLinuxFeed for an explicit non-publishable Windows bundle."
    }
    $repoRootForWsl = ($repoRoot -replace '\\', '/')
    $repoRootWsl = (& wsl.exe wslpath -a $repoRootForWsl).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $repoRootWsl) {
        throw "Could not translate repo path for WSL."
    }
}

if ($CheckEnvOnly) {
    $linuxArgs = "--check-env"
    if ($NoBootstrapZig) {
        $linuxArgs += " --no-bootstrap-zig"
    }
    Invoke-Checked -ErrorMessage "Linux release environment check failed." -Command {
        & wsl.exe bash -lc "cd '$repoRootWsl' && native/publish-linux-feed-wsl.sh $linuxArgs"
    }
    Write-Host "Complete local release environment OK for v$version."
    exit 0
}

$releaseLock = $null
$stageRoot = $null
$releaseCompleted = $false
if (-not $SkipLinuxFeed) {
    $releaseLock = Enter-CompleteReleaseLock $releaseRoot "native/publish-release-local.ps1"
}
try {
    # The Windows desktop binary embeds the Linux SSH-agent payloads. Refresh
    # them before the Windows build so source/protocol changes cannot ship a
    # stale helper.
    if (-not $SkipLinuxFeed) {
        Invoke-Checked -ErrorMessage "SSH-agent bundle build failed." -Command {
            & wsl.exe bash -lc "cd '$repoRootWsl' && native/build-agent-bundles.sh"
        }
    }

    New-Item -ItemType Directory -Force $releaseRoot | Out-Null
    $stageName = ".complete-release-stage.{0}.{1}" -f $PID, [guid]::NewGuid().ToString("N")
    $stageRoot = Join-Path $releaseRoot $stageName
    $stageRelease = Join-Path $stageRoot "release"
    $stageFeed = Join-Path $stageRelease "update-feed"
    $versionStage = Join-Path $stageRoot "version.txt"
    New-Item -ItemType Directory -Force $stageRelease | Out-Null

    Push-Location $scriptRoot
    try {
        & .\publish-update.ps1 -Feed $stageFeed -ReleaseOutput $stageRelease -AllowPartialFeed -DeferFeedVersion
        if ($LASTEXITCODE -ne 0) {
            throw "Windows release build failed."
        }
    } finally {
        Pop-Location
    }

    $installer = Join-Path $stageRelease "Smart Explorer Setup $version.exe"
    $portableApp = Join-Path $stageRelease "Smart Explorer.exe"
    $portableUpdater = Join-Path $stageRelease "Smart Explorer Updater.exe"
    $portableCli = Join-Path $stageRelease "se.exe"
    $windowsShare = Join-Path $stageRelease "share-server\se-share-server.exe"
    $linuxShare = Join-Path $stageRelease "share-server\se-share-server-linux"
    $commandDll = Join-Path $stageRelease "smart_explorer_command.dll"
    $linuxInstaller = Join-Path $repoRoot "install-linux.sh"

    foreach ($path in @($installer, $portableApp, $portableUpdater, $portableCli, $windowsShare, $commandDll)) {
        Assert-NonEmptyFile $path
    }
    Assert-ContextDll $commandDll $peInspector
    Assert-WindowsManifest $stageFeed $version
    Assert-SameSha256 $portableApp (Join-Path $stageFeed "smart_explorer.exe")
    Assert-SameSha256 $portableUpdater (Join-Path $stageFeed "smart_explorer_updater.exe")
    Assert-SameSha256 $portableCli (Join-Path $stageFeed "se.exe")
    if (Test-Path -LiteralPath (Join-Path $stageFeed "version.txt")) {
        throw "The isolated feed was versioned before the complete release commit."
    }

    if ($SkipLinuxFeed) {
        foreach ($linuxName in @("smart_explorer", "smart_explorer_updater", "se")) {
            if (Test-Path -LiteralPath (Join-Path $stageFeed $linuxName)) {
                throw "Windows-only stage unexpectedly contains Linux payload: $linuxName"
            }
        }
        $partialOutput = Join-Path $releaseRoot "windows-partial-v$version"
        Publish-PartialBundle $stageRelease $partialOutput
        Write-Warning "NON-PUBLISHABLE partial Windows bundle verified at $partialOutput. The shared update feed and version.txt were not changed."
        $releaseCompleted = $true
        return
    }

    $stageReleaseWsl = "$repoRootWsl/release-native/$stageName/release"
    $linuxArgs = ""
    if ($NoBootstrapZig) {
        $linuxArgs = " --no-bootstrap-zig"
    }
    Invoke-Checked -ErrorMessage "Linux feed build failed." -Command {
        & wsl.exe bash -lc "cd '$repoRootWsl' && SMART_EXPLORER_RELEASE_LOCK_TOKEN='$($releaseLock.Token)' SMART_EXPLORER_FEED_DIR='$stageReleaseWsl/update-feed' SMART_EXPLORER_SHARE_DIR='$stageReleaseWsl/share-server' native/publish-linux-feed-wsl.sh$linuxArgs"
    }

    Assert-NonEmptyFile $linuxInstaller
    Assert-NonEmptyFile $linuxShare
    Assert-WindowsManifest $stageFeed $version
    foreach ($name in @("smart_explorer", "smart_explorer_updater", "se")) {
        Assert-FeedHash $stageFeed $name
    }
    foreach ($path in @($installer, $portableApp, $portableUpdater, $portableCli, $windowsShare, $linuxShare, $commandDll)) {
        Assert-NonEmptyFile $path
    }
    if (Test-Path -LiteralPath (Join-Path $stageFeed "version.txt")) {
        throw "The isolated feed was versioned before the complete release commit."
    }

    Invoke-Checked -ErrorMessage "Staged Linux/static release verification failed." -Command {
        & wsl.exe bash -lc "cd '$stageReleaseWsl/update-feed' && sha256sum -c smart_explorer.exe.sha256 && sha256sum -c smart_explorer_updater.exe.sha256 && sha256sum -c se.exe.sha256 && sha256sum -c smart_explorer.sha256 && sha256sum -c smart_explorer_updater.sha256 && sha256sum -c se.sha256 && test -x '$repoRootWsl/install-linux.sh' && file smart_explorer | grep -Fq 'dynamically linked' && file '$stageReleaseWsl/share-server/se-share-server-linux' | grep -Eq 'statically linked|static-pie linked'"
    }

    Set-Content -LiteralPath $versionStage -Value $version -Encoding ascii
    Publish-CompleteRelease $stageRelease $stageFeed $versionStage $version

    $feedVersion = (Get-Content -LiteralPath (Join-Path $feed "version.txt") -TotalCount 1).Trim()
    if ($feedVersion -ne $version) {
        throw "Feed version '$feedVersion' does not match Cargo.toml version '$version'."
    }
    Assert-WindowsManifest $feed $version
    foreach ($name in @("smart_explorer", "smart_explorer_updater", "se")) {
        Assert-FeedHash $feed $name
    }
    Write-Host "Complete local release artifacts atomically staged and verified: v$version"
    Write-Host "Installer: $(Join-Path $releaseRoot "Smart Explorer Setup $version.exe")"
    Write-Host "Feed: $feed"
    $releaseCompleted = $true
} finally {
    if ($stageRoot -and (Test-Path -LiteralPath $stageRoot)) {
        if ($releaseCompleted -or $SkipLinuxFeed) {
            Remove-Item -LiteralPath $stageRoot -Recurse -Force -ErrorAction SilentlyContinue
        } else {
            Write-Warning "Complete release failed; preserved stage: $stageRoot"
            Write-Warning "No automatic resume is assumed. Inspect the stage and rerun only through a verified release script."
        }
    }
    if ($releaseLock) {
        Exit-CompleteReleaseLock $releaseLock
    }
}
