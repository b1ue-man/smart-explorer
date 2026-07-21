# Build and publish one complete Smart Explorer release from Windows/WSL or Linux.
#
# Windows and Linux outputs are built into one isolated release tree. The live
# release-native artifacts are changed only after every staged artifact passes
# validation; version.txt is the final commit marker.

param(
    [switch]$SkipLinuxFeed,
    [switch]$NoBootstrapZig,
    [switch]$CheckEnvOnly,
    [switch]$SkipLocalCliUpdate,
    [ValidateRange(30, 360)][int]$PublicationTimeoutMinutes = 180
)

$ErrorActionPreference = "Stop"

# The canonical release must remain safe on the 8-GiB builders used for local
# publication. Pin these values instead of accepting ambient Cargo overrides.
$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_RELEASE_LTO = "thin"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
$env:CARGO_PROFILE_RELEASE_DEBUG = "0"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptRoot "..")).Path
$releaseRoot = Join-Path $repoRoot "release-native"
$feed = Join-Path $releaseRoot "update-feed"
$workflowFile = "build.yml"
$releaseCommitSuffix = "[release candidate]"
. (Join-Path $scriptRoot "release-lock.ps1")
$publicationHelper = Join-Path $scriptRoot "release-publication.ps1"
if (-not (Test-Path -LiteralPath $publicationHelper -PathType Leaf)) {
    throw "Release publication helper missing: $publicationHelper"
}
. $publicationHelper

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

function Assert-WindowsManifest(
    [string]$FeedDirectory,
    [string]$ExpectedVersion,
    [string]$ExpectedSourceCommit
) {
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
    if ($ExpectedSourceCommit -notmatch '^[0-9a-fA-F]{40,64}$') {
        throw "Expected Windows build source commit is invalid."
    }
    $ExpectedSourceCommit = $ExpectedSourceCommit.ToLowerInvariant()
    $manifestSourceCommit = [string]$entries["source_commit"]
    if ($manifestSourceCommit -notmatch '^[0-9a-fA-F]{40,64}$' -or
        $manifestSourceCommit.ToLowerInvariant() -ne $expectedSourceCommit) {
        throw "Windows build manifest source commit '$($entries["source_commit"])' does not match '$expectedSourceCommit'."
    }
    if ($entries.Count -ne 5) {
        throw "Windows build manifest must contain exactly version, source_commit, and three payload hashes."
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
    [string]$ExpectedVersion,
    [string]$ExpectedSourceCommit
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
        Assert-WindowsManifest $feedCandidate $ExpectedVersion $ExpectedSourceCommit
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
        Assert-WindowsManifest $feed $ExpectedVersion $ExpectedSourceCommit
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

function Invoke-WindowsReleaseBuild {
$version = Get-NativeVersion
$buildSourceCommit = (Invoke-ReleasePublicationGit `
    -RepoRoot $repoRoot `
    -Arguments @("rev-parse", "HEAD^{commit}")).StdOut.Trim().ToLowerInvariant()
if ($buildSourceCommit -notmatch '^[0-9a-fA-F]{40,64}$') {
    throw "Could not bind the complete Windows/WSL build to one source commit."
}
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
$ownsReleaseLock = $false
$stageRoot = $null
$releaseCompleted = $false
if (-not $SkipLinuxFeed) {
    if ($script:completeReleaseLock) {
        $releaseLock = $script:completeReleaseLock
    } else {
        $releaseLock = Enter-CompleteReleaseLock $releaseRoot "native/publish-release-local.ps1"
        $ownsReleaseLock = $true
    }
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
    Assert-WindowsManifest $stageFeed $version $buildSourceCommit
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
        & wsl.exe bash -lc "cd '$repoRootWsl' && SMART_EXPLORER_RELEASE_LOCK_TOKEN='$($releaseLock.Token)' SMART_EXPLORER_FEED_DIR='$stageReleaseWsl/update-feed' SMART_EXPLORER_SHARE_DIR='$stageReleaseWsl/share-server' native/run-release-memory-bounded.sh native/publish-linux-feed-wsl.sh$linuxArgs"
    }

    Assert-NonEmptyFile $linuxInstaller
    Assert-NonEmptyFile $linuxShare
    Assert-WindowsManifest $stageFeed $version $buildSourceCommit
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
    Publish-CompleteRelease `
        $stageRelease `
        $stageFeed `
        $versionStage `
        $version `
        $buildSourceCommit

    $feedVersion = (Get-Content -LiteralPath (Join-Path $feed "version.txt") -TotalCount 1).Trim()
    if ($feedVersion -ne $version) {
        throw "Feed version '$feedVersion' does not match Cargo.toml version '$version'."
    }
    Assert-WindowsManifest $feed $version $buildSourceCommit
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
    if ($ownsReleaseLock -and $releaseLock) {
        Exit-CompleteReleaseLock $releaseLock
    }
}
}

function Test-RunningOnLinux {
    $isLinuxVariable = Get-Variable -Name IsLinux -ErrorAction SilentlyContinue
    if ($isLinuxVariable) {
        return [bool]$isLinuxVariable.Value
    }
    return [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Unix
}

function Invoke-GitCaptured {
    param([string[]]$ArgumentList, [switch]$AllowFailure)
    return Invoke-ReleasePublicationGit `
        -RepoRoot $repoRoot `
        -Arguments $ArgumentList `
        -AllowFailure:$AllowFailure
}

function Get-GitText([string[]]$ArgumentList) {
    return (Invoke-GitCaptured -ArgumentList $ArgumentList).StdOut.Trim()
}

function Get-GitHubRepositorySlug {
    return Get-PublicationRepositorySlug -RepoRoot $repoRoot
}

function Invoke-GitHubGet {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$AllowNotFound
    )
    $apiPath = if ($Path.StartsWith('/')) { $Path } else { "/$Path" }
    return Invoke-ReleasePublicationGitHubGet `
        -RepositorySlug $script:githubRepository `
        -ApiPath $apiPath `
        -AllowNotFound:$AllowNotFound
}

function Get-VersionFromCargoText([string]$Text, [string]$Source) {
    $match = [regex]::Match($Text, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        throw "Could not read native version from $Source."
    }
    return $match.Groups[1].Value
}

function Get-RemoteMainVersion {
    return Get-VersionFromCargoText (Get-GitText @("show", "origin/main:native/Cargo.toml")) "origin/main:native/Cargo.toml"
}

function Set-NativeVersion([string]$Version) {
    $cargoToml = Join-Path $scriptRoot "Cargo.toml"
    $cargoLock = Join-Path $scriptRoot "Cargo.lock"
    $cargoText = [System.IO.File]::ReadAllText($cargoToml)
    $cargoPattern = [regex]::new('(?m)^version\s*=\s*"([^"]+)"')
    $cargoMatch = $cargoPattern.Match($cargoText)
    if (-not $cargoMatch.Success) {
        throw "Could not update version in $cargoToml"
    }
    $cargoVersion = $cargoMatch.Groups[1].Value
    if ($cargoVersion -ne $Version -and (Get-NextPatchVersion $cargoVersion) -ne $Version) {
        throw "Cargo.toml version '$cargoVersion' cannot advance or resume '$Version'."
    }

    $lockText = [System.IO.File]::ReadAllText($cargoLock)
    $lockPattern = [regex]::new(
        '(?ms)(^\[\[package\]\]\r?\nname = "smart_explorer"\r?\nversion = ")([^"]+)(")'
    )
    $lockMatches = $lockPattern.Matches($lockText)
    if ($lockMatches.Count -ne 1) {
        throw "Cargo.lock must contain exactly one smart_explorer root package entry."
    }
    $lockVersion = $lockMatches[0].Groups[2].Value
    if ($lockVersion -ne $Version -and (Get-NextPatchVersion $lockVersion) -ne $Version) {
        throw "Cargo.lock root version '$lockVersion' cannot advance or resume '$Version'."
    }

    $updatedCargo = $cargoPattern.Replace($cargoText, "version = `"$Version`"", 1)
    $updatedLock = $lockPattern.Replace(
        $lockText,
        { param($match) "$($match.Groups[1].Value)$Version$($match.Groups[3].Value)" },
        1
    )
    $encoding = [System.Text.UTF8Encoding]::new($false)
    $stageId = "{0}-{1}" -f $PID, [guid]::NewGuid().ToString('N')
    $lockStage = Join-Path $scriptRoot ".Cargo.lock.complete-release-version.$stageId"
    $cargoStage = Join-Path $scriptRoot ".Cargo.toml.complete-release-version.$stageId"
    try {
        [System.IO.File]::WriteAllText($lockStage, $updatedLock, $encoding)
        [System.IO.File]::WriteAllText($cargoStage, $updatedCargo, $encoding)
        # Lock first: a crash between the two atomic same-directory renames is
        # recovered by the next Bump/Resume call, while Cargo.toml still keeps
        # the remote version decision unambiguous.
        [System.IO.File]::Move($lockStage, $cargoLock, $true)
        [System.IO.File]::Move($cargoStage, $cargoToml, $true)
    } finally {
        Remove-Item -LiteralPath $lockStage -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $cargoStage -Force -ErrorAction SilentlyContinue
    }
}

function Get-NextPatchVersion([string]$Version) {
    $match = [regex]::Match($Version, '^(\d+)\.(\d+)\.(\d+)$')
    if (-not $match.Success) {
        throw "Automatic release bump requires a stable major.minor.patch version, got '$Version'."
    }
    $patch = [uint64]$match.Groups[3].Value
    if ($patch -eq [uint64]::MaxValue) {
        throw "Patch version overflow for $Version."
    }
    return "$($match.Groups[1].Value).$($match.Groups[2].Value).$($patch + 1)"
}

function Get-HeadSha {
    return Get-GitText @("rev-parse", "HEAD")
}

function Get-OriginMainSha {
    return Get-GitText @("rev-parse", "origin/main")
}

function Get-RemoteMainSha {
    $line = Get-GitText @("ls-remote", "origin", "refs/heads/main")
    if (-not $line) {
        throw "origin/main is missing."
    }
    return ($line -split '\s+')[0]
}

function Get-RemoteTagCommit([string]$Tag) {
    return Get-PublicationRemoteTagCommit -RepoRoot $repoRoot -Tag $Tag
}

function Test-GitAncestor([string]$Ancestor, [string]$Descendant) {
    $result = Invoke-GitCaptured -ArgumentList @("merge-base", "--is-ancestor", $Ancestor, $Descendant) -AllowFailure
    if ($result.ExitCode -eq 0) { return $true }
    if ($result.ExitCode -eq 1) { return $false }
    throw "Could not compare Git ancestry: $($result.Output)"
}

function Test-ReleaseMutablePath([string]$Path) {
    $normalized = $Path.Replace('\', '/')
    return $normalized -eq "native/Cargo.toml" -or
        $normalized -eq "native/Cargo.lock" -or
        $normalized.StartsWith("native/agent-bin/") -or
        $normalized.StartsWith("release-native/update-feed/") -or
        $normalized.StartsWith("release-native/share-server/") -or
        $normalized -eq "release-native/Smart Explorer.exe" -or
        $normalized -eq "release-native/Smart Explorer Updater.exe" -or
        $normalized -eq "release-native/se.exe" -or
        $normalized -eq "release-native/smart_explorer_command.dll" -or
        $normalized.StartsWith("release-native/Smart Explorer Setup ")
}

function Get-TrackedChanges {
    $status = Get-GitText @("status", "--porcelain=v1", "--untracked-files=no")
    $paths = [System.Collections.Generic.List[string]]::new()
    foreach ($line in ($status -split "`n")) {
        if ($line.Length -lt 4) { continue }
        $path = $line.Substring(3)
        if ($path.Contains(" -> ")) {
            $path = ($path -split ' -> ')[-1]
        }
        $paths.Add($path.Trim('"'))
    }
    return $paths.ToArray()
}

function Assert-GitReleasePreflight {
    $branch = Get-GitText @("branch", "--show-current")
    if ($branch -ne "main") {
        throw "Complete releases require the local main branch; current branch is '$branch'."
    }
    Invoke-GitCaptured -ArgumentList @("fetch", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main") | Out-Null
    $head = Get-HeadSha
    $remote = Get-OriginMainSha
    if ($head -ne $remote) {
        $subject = Get-GitText @("show", "-s", "--format=%s", "HEAD")
        $ahead = Get-GitText @("rev-list", "--count", "origin/main..HEAD")
        if (-not (Test-GitAncestor $remote $head) -or $ahead -ne "1" -or
            $subject -notmatch '^Release Smart Explorer v[^ ]+ \[release candidate\]$') {
            throw "main must equal origin/main before a release. Only one exact unpushed release-candidate commit may resume."
        }
    }
    foreach ($path in (Get-TrackedChanges)) {
        if (-not (Test-ReleaseMutablePath $path)) {
            throw "Tracked worktree change is outside release recovery state: $path"
        }
    }
}

function Assert-NonInteractiveGitWriteAccess {
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    $tagRef = "refs/tags/v$Version"
    $fallbackRef = "refs/heads/release/v$Version"
    $oldPrompt = $env:GIT_TERMINAL_PROMPT
    $oldGcmInteractive = $env:GCM_INTERACTIVE
    try {
        $env:GIT_TERMINAL_PROMPT = "0"
        $env:GCM_INTERACTIVE = "Never"
        $mainProbe = Invoke-GitCaptured `
            -ArgumentList @("push", "--dry-run", "origin", "HEAD:refs/heads/main") `
            -AllowFailure
        if ($mainProbe.ExitCode -ne 0) {
            throw "Non-interactive Git main write preflight failed: $($mainProbe.Output)"
        }
        $tagProbe = Invoke-GitCaptured `
            -ArgumentList @("push", "--dry-run", "origin", "HEAD:$tagRef") `
            -AllowFailure
        $fallbackProbe = Invoke-GitCaptured `
            -ArgumentList @("push", "--dry-run", "origin", "HEAD:$fallbackRef") `
            -AllowFailure
        if ($tagProbe.ExitCode -ne 0 -and $fallbackProbe.ExitCode -ne 0) {
            throw "Neither the immutable tag nor release-branch publication trigger is writable non-interactively. Tag: $($tagProbe.Output); fallback: $($fallbackProbe.Output)"
        }
    } finally {
        $env:GIT_TERMINAL_PROMPT = $oldPrompt
        $env:GCM_INTERACTIVE = $oldGcmInteractive
    }
}

function Resolve-ReleasePlan {
    $localVersion = Get-NativeVersion
    $remoteVersion = Get-RemoteMainVersion
    try {
        $localSemver = [version]$localVersion
        $remoteSemver = [version]$remoteVersion
    } catch {
        throw "Release recovery requires numeric stable versions (local '$localVersion', origin/main '$remoteVersion')."
    }
    $head = Get-HeadSha
    $tag = "v$localVersion"
    $tagCommit = Get-RemoteTagCommit $tag
    if ($tagCommit) {
        if ($tagCommit -eq $head) {
            return [pscustomobject]@{ Action = "Tagged"; Version = $localVersion; Tag = $tag; Candidate = $head }
        }
        if (-not (Test-GitAncestor $tagCommit $head)) {
            throw "$tag points to unrelated commit $tagCommit; tags are immutable."
        }
        $next = Get-NextPatchVersion $localVersion
        if (Get-RemoteTagCommit "v$next") {
            throw "The next patch tag v$next already exists; refusing to skip or rewrite versions."
        }
        return [pscustomobject]@{ Action = "Bump"; Version = $next; Tag = "v$next"; Candidate = $null }
    }
    if ($localSemver -lt $remoteSemver) {
        throw "Local version $localVersion is older than origin/main $remoteVersion."
    }
    if ($localSemver -gt $remoteSemver) {
        $expected = Get-NextPatchVersion $remoteVersion
        if ($localVersion -ne $expected) {
            throw "Recovery version $localVersion must be the single next patch after origin/main $remoteVersion."
        }
    }
    return [pscustomobject]@{ Action = "Resume"; Version = $localVersion; Tag = $tag; Candidate = $null }
}

function Assert-LinuxReleaseEnvironment {
    foreach ($tool in @("bash", "git")) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "$tool is required for the Linux release path."
        }
    }
    $script:peInspector = Get-Command x86_64-w64-mingw32-objdump -ErrorAction SilentlyContinue
    $args = @((Join-Path $scriptRoot "publish-feed.sh"), "--check-env")
    if ($NoBootstrapZig) { $args += "--no-bootstrap-zig" }
    & bash @args
    if ($LASTEXITCODE -ne 0) {
        throw "Linux complete-release environment check failed."
    }
}

function Assert-WindowsReleaseEnvironment {
    $env:Path = "$env:USERPROFILE\.cargo\bin;C:\Strawberry\c\bin;C:\Program Files (x86)\NSIS;$env:Path"
    $cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if (-not $cargo) { throw "cargo.exe not found. Install Rust for Windows or fix PATH." }
    & cargo fmt --version | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt is not available for the Windows Rust toolchain." }
    & cargo clippy --version | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy is not available for the Windows Rust toolchain." }
    $makensis = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if (-not $makensis) {
        $nsisCandidates = @(
            "$env:ProgramFiles\NSIS\makensis.exe",
            "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
        )
        $makensis = $nsisCandidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    }
    if (-not $makensis) { throw "makensis.exe not found; a release without an installer is incomplete." }
    $archiveExtractor = @("7z.exe", "7za.exe", "7z", "7za") |
        ForEach-Object { Get-Command $_ -CommandType Application -ErrorAction SilentlyContinue } |
        Select-Object -First 1
    if (-not $archiveExtractor) {
        $archiveCandidates = @(
            "$env:ProgramFiles\7-Zip\7z.exe",
            "${env:ProgramFiles(x86)}\7-Zip\7z.exe"
        )
        $archivePath = $archiveCandidates |
            Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
            Select-Object -First 1
        if ($archivePath) {
            $env:Path = "$(Split-Path -Parent $archivePath);$env:Path"
            $archiveExtractor = Get-Command 7z.exe -CommandType Application -ErrorAction SilentlyContinue
        }
    }
    if (-not $archiveExtractor) {
        throw "7z is required to verify the exact installer payloads before tagging."
    }
    $script:peInspector = @("llvm-objdump.exe", "objdump.exe", "dumpbin.exe") |
        ForEach-Object { Get-Command $_ -ErrorAction SilentlyContinue } |
        Select-Object -First 1
    if (-not $script:peInspector) { throw "No PE export inspector found." }
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) { throw "wsl.exe not found; a complete Windows-host release requires WSL." }
    $repoRootForWsl = ($repoRoot -replace '\\', '/')
    $repoRootWsl = (& wsl.exe wslpath -a $repoRootForWsl).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $repoRootWsl) { throw "Could not translate repo path for WSL." }
    $linuxArgs = "--check-env"
    if ($NoBootstrapZig) { $linuxArgs += " --no-bootstrap-zig" }
    & wsl.exe bash -lc "cd '$repoRootWsl' && native/publish-linux-feed-wsl.sh $linuxArgs"
    if ($LASTEXITCODE -ne 0) { throw "WSL/Linux release environment check failed." }
}

function Assert-CommonReleasePreflight {
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        throw "git is required for a complete release."
    }
    Assert-GitReleasePreflight
    Assert-PublicationNoUntrackedBuildInputs -RepoRoot $repoRoot
    $script:githubRepository = Get-GitHubRepositorySlug
    $null = Get-ReleasePublicationGitHubToken -Require
    $workflow = Invoke-GitHubGet "actions/workflows/$workflowFile"
    if (-not $workflow -or $workflow.state -ne "active") {
        throw "GitHub workflow $workflowFile is not active."
    }
    $plan = Resolve-ReleasePlan
    Assert-NonInteractiveGitWriteAccess -Version $plan.Version
    if (Test-RunningOnLinux) {
        Assert-LinuxReleaseEnvironment
    } else {
        Assert-WindowsReleaseEnvironment
    }
    Write-Host "Release preflight OK: action=$($plan.Action), candidate=$($plan.Tag)."
    return $plan
}

function Invoke-LinuxCompleteReleaseBuild($ReleaseLock) {
    $oldToken = $env:SMART_EXPLORER_RELEASE_LOCK_TOKEN
    try {
        $env:SMART_EXPLORER_RELEASE_LOCK_TOKEN = $ReleaseLock.Token
        $args = @(
            (Join-Path $scriptRoot "run-release-memory-bounded.sh"),
            (Join-Path $scriptRoot "publish-feed.sh")
        )
        if ($NoBootstrapZig) { $args += "--no-bootstrap-zig" }
        & bash @args
        if ($LASTEXITCODE -ne 0) {
            throw "Complete Linux-host release build failed."
        }
    } finally {
        $env:SMART_EXPLORER_RELEASE_LOCK_TOKEN = $oldToken
    }
}

function Test-CompleteReleaseCandidateAvailable([string]$Version) {
    try {
        $null = Assert-ReleasePublicationCandidate -RepoRoot $repoRoot -Version $Version
        return $true
    } catch {
        Write-Host "A complete v$Version candidate is not yet available: $($_.Exception.Message)"
        return $false
    }
}

if ($SkipLinuxFeed) {
    if (Test-RunningOnLinux) {
        throw "-SkipLinuxFeed is a Windows-only non-publishable diagnostic."
    }
    Invoke-WindowsReleaseBuild
    exit 0
}

$preflightPlan = Assert-CommonReleasePreflight
if ($CheckEnvOnly) {
    Write-Host "Complete release environment and publication access are ready for $($preflightPlan.Tag)."
    exit 0
}

$completeReleaseLock = Enter-CompleteReleaseLock $releaseRoot "native/publish-release-local.ps1"
$script:completeReleaseLock = $completeReleaseLock
$releaseSucceeded = $false
try {
    # Re-resolve under the cross-host lock so a concurrent remote tag cannot
    # turn the preflight decision into a second version or a rewritten tag.
    $plan = Resolve-ReleasePlan
    if ($plan.Action -in @("Bump", "Resume")) {
        Set-NativeVersion $plan.Version
    }
    if ($plan.Action -eq "Bump") {
        Write-Host "Release version advanced once to $($plan.Version)."
    }
    $version = $plan.Version

    if ($plan.Action -ne "Tagged") {
        if (-not (Test-CompleteReleaseCandidateAvailable $version)) {
            if (Test-RunningOnLinux) {
                Invoke-LinuxCompleteReleaseBuild $completeReleaseLock
            } else {
                Invoke-WindowsReleaseBuild
            }
        } else {
            Write-Host "Reusing the already verified v$version release build after a pre-tag interruption."
        }
        if (-not (Test-CompleteReleaseCandidateAvailable $version)) {
            throw "Complete v$version candidate validation failed after the release build."
        }
        $commit = Invoke-ReleasePublicationCommit -RepoRoot $repoRoot -Version $version
        $candidateSha = $commit.CandidateSha
        # Revalidate the committed shape and parent-bound manifest before any
        # remote ref can expose this candidate.
        $null = Assert-ReleasePublicationCandidate -RepoRoot $repoRoot -Version $version
        Invoke-ReleasePublicationMainPush -RepoRoot $repoRoot -CandidateSha $candidateSha
        $publicationTrigger = Invoke-ReleasePublicationTagPush `
            -RepoRoot $repoRoot `
            -Version $version `
            -CandidateSha $candidateSha
    } else {
        $candidateSha = $plan.Candidate
        if (-not (Test-CompleteReleaseCandidateAvailable $version)) {
            throw "The immutable $($plan.Tag) candidate is incomplete in the local checkout."
        }
        Invoke-ReleasePublicationMainPush -RepoRoot $repoRoot -CandidateSha $candidateSha
        Write-Host "Immutable $($plan.Tag) already targets $candidateSha; no tag is created or moved."
        $fallbackBranch = "release/v$version"
        $fallbackCandidate = Get-PublicationRemoteBranchCommit `
            -RepoRoot $repoRoot `
            -Branch $fallbackBranch
        $publicationTrigger = [pscustomobject]@{
            TriggerBranch = if ($fallbackCandidate -eq $candidateSha) { $fallbackBranch } else { $plan.Tag }
            TriggerKind = if ($fallbackCandidate -eq $candidateSha) { "release-branch" } else { "tag" }
            ExistingRun = $true
        }
    }

    $deadline = [DateTime]::UtcNow.AddMinutes($PublicationTimeoutMinutes)
    Wait-ReleasePublicationWorkflow `
        -RepositorySlug $script:githubRepository `
        -Version $version `
        -CandidateSha $candidateSha `
        -TriggerBranch $publicationTrigger.TriggerBranch `
        -RetryFailedOnce:([bool]$publicationTrigger.ExistingRun) `
        -Deadline $deadline
    Wait-ReleasePublicationAssets `
        -RepoRoot $repoRoot `
        -RepositorySlug $script:githubRepository `
        -Version $version `
        -Deadline $deadline
    $publishedTagCommit = Get-RemoteTagCommit "v$version"
    if ($publishedTagCommit -ne $candidateSha) {
        throw "Published tag v$version moved to '$publishedTagCommit'; expected immutable candidate '$candidateSha'."
    }

    if ((Test-RunningOnLinux) -and -not $SkipLocalCliUpdate) {
        Invoke-ReleasePublicationLinuxCliUpdate `
            -RepoRoot $repoRoot `
            -RepositorySlug $script:githubRepository `
            -Version $version `
            -CandidateSha $candidateSha
    }
    Write-Host "Complete Smart Explorer v$version release published and verified from $candidateSha."
    $releaseSucceeded = $true
} finally {
    $lockToRelease = $completeReleaseLock
    $script:completeReleaseLock = $null
    if ($lockToRelease) {
        Exit-CompleteReleaseLock $lockToRelease
    }
}

if (-not $releaseSucceeded) {
    throw "Complete release did not reach its final publication and local-update checks."
}
