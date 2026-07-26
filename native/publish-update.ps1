# Build an explicitly isolated Windows-only Smart Explorer bundle and feed.
# A complete release must use publish-release-local.ps1, which supplies staged
# paths here and adds the Linux payloads before publishing version.txt.
# Direct calls must provide -AllowPartialFeed plus separate -Feed and
# -ReleaseOutput paths; they do not create a complete cross-platform release.

param(
    [string]$Feed = "",
    [switch]$AllowPartialFeed,
    [switch]$DeferFeedVersion,
    [string]$ReleaseOutput = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# Keep every direct Windows release leaf within the canonical memory budget;
# WSL does not reliably inherit custom Windows environment variables.
$env:CARGO_BUILD_JOBS = [Environment]::ProcessorCount.ToString()
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_RELEASE_LTO = "off"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16"
$env:CARGO_PROFILE_RELEASE_DEBUG = "0"

# Version aus Cargo.toml lesen
$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Baue Version $version ..."

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$git = Get-Command git -ErrorAction SilentlyContinue
if (-not $git) {
    throw "git wird fuer die Build-Provenienz im Windows-Manifest benoetigt."
}
$sourceCommit = (& $git.Source -C $repoRoot rev-parse "HEAD^{commit}" | Out-String).Trim().ToLowerInvariant()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40,64}$') {
    throw "Quell-Commit fuer das Windows-Buildmanifest konnte nicht bestimmt werden."
}
$defaultFeed = Join-Path $repoRoot "release-native\update-feed"
$defaultReleaseOutput = Join-Path $repoRoot "release-native"
if ([string]::IsNullOrWhiteSpace($Feed)) {
    $Feed = $defaultFeed
}
$resolvedFeed = if (Test-Path $Feed) { (Resolve-Path $Feed).Path } else { [System.IO.Path]::GetFullPath($Feed) }
$resolvedDefaultFeed = if (Test-Path $defaultFeed) { (Resolve-Path $defaultFeed).Path } else { [System.IO.Path]::GetFullPath($defaultFeed) }
if (-not $AllowPartialFeed) {
    throw "publish-update.ps1 baut immer nur Windows-Payloads. Bestaetige den Teil-Build explizit mit -AllowPartialFeed; fuer einen vollstaendigen Release nutze publish-release-local.ps1."
}
if ($resolvedFeed -eq $resolvedDefaultFeed) {
    throw "Ein Windows-only Lauf darf den gemeinsamen Standard-Feed nicht veraendern. Nutze einen separaten -Feed Pfad oder publish-release-local.ps1 fuer den vollstaendigen Feed."
}
if ([string]::IsNullOrWhiteSpace($ReleaseOutput)) {
    throw "Ein Windows-only Lauf braucht einen expliziten separaten -ReleaseOutput Pfad."
}
$resolvedReleaseOutput = if (Test-Path $ReleaseOutput) { (Resolve-Path $ReleaseOutput).Path } else { [System.IO.Path]::GetFullPath($ReleaseOutput) }
$resolvedDefaultReleaseOutput = if (Test-Path $defaultReleaseOutput) { (Resolve-Path $defaultReleaseOutput).Path } else { [System.IO.Path]::GetFullPath($defaultReleaseOutput) }
if ($resolvedReleaseOutput -eq $resolvedDefaultReleaseOutput) {
    throw "Ein Windows-only Lauf darf die gemeinsamen release-native Artefakte nicht veraendern. Nutze einen separaten -ReleaseOutput Pfad."
}
$feedPrefix = $resolvedFeed.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if ($resolvedReleaseOutput -eq $resolvedFeed -or
    $resolvedReleaseOutput.StartsWith($feedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "-ReleaseOutput darf nicht dem -Feed entsprechen oder innerhalb des Feed-Pfads liegen."
}
$ReleaseOutput = $resolvedReleaseOutput

function Assert-NonEmptyFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Erforderliche Release-Datei fehlt: $Path"
    }
    if ((Get-Item -LiteralPath $Path).Length -le 0) {
        throw "Erforderliche Release-Datei ist leer: $Path"
    }
}

function Write-AsciiLf([string]$Path, [string[]]$Lines) {
    $content = ($Lines -join "`n") + "`n"
    [System.IO.File]::WriteAllText($Path, $content, [System.Text.Encoding]::ASCII)
}

function Write-Sha256File([string]$Path) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    $name = [System.IO.Path]::GetFileName($Path)
    Write-AsciiLf "$Path.sha256" @("$hash  $name")
}

function Publish-FileAtomic([string]$Source, [string]$Destination) {
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force $parent | Out-Null
    $temporary = Join-Path $parent (".{0}.{1}.tmp" -f ([System.IO.Path]::GetFileName($Destination)), $PID)
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
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

function Publish-FeedDirectoryTransaction(
    [string]$Candidate,
    [string]$Destination,
    [string]$VersionSource,
    [string]$ExpectedVersion
) {
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force $parent | Out-Null
    $backup = Join-Path $parent (".{0}.release-backup.{1}.{2}" -f ([System.IO.Path]::GetFileName($Destination)), $PID, [guid]::NewGuid().ToString("N"))
    $hadDestination = Test-Path -LiteralPath $Destination
    $installed = $false
    try {
        if ($hadDestination) {
            Move-Item -LiteralPath $Destination -Destination $backup
        }
        $installed = $true
        Move-Item -LiteralPath $Candidate -Destination $Destination
        if (-not [string]::IsNullOrWhiteSpace($VersionSource)) {
            Publish-FileAtomic $VersionSource (Join-Path $Destination "version.txt")
        }
        foreach ($name in @("smart_explorer.exe", "smart_explorer_updater.exe", "se.exe")) {
            Assert-NonEmptyFile (Join-Path $Destination $name)
            Assert-NonEmptyFile (Join-Path $Destination "$name.sha256")
        }
        Assert-NonEmptyFile (Join-Path $Destination "windows-build.manifest")
        if (-not [string]::IsNullOrWhiteSpace($VersionSource)) {
            $publishedVersion = (Get-Content -LiteralPath (Join-Path $Destination "version.txt") -TotalCount 1).Trim()
            if ($publishedVersion -ne $ExpectedVersion) {
                throw "Windows-only Feed-Version '$publishedVersion' stimmt nicht mit '$ExpectedVersion' ueberein."
            }
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
            throw "Feed-Publikation fehlgeschlagen ($primary); Rollback fehlgeschlagen: $_"
        }
        throw $primary
    }
}

# Resolve and verify the one centrally pinned Dokany MSI before starting the
# expensive native builds. The ignored target cache is shared by preflight and
# release so the full build never redownloads an already verified dependency.
$dokanyFetch = Join-Path $PSScriptRoot "fetch-dokany-runtime.ps1"
if (-not (Test-Path -LiteralPath $dokanyFetch -PathType Leaf)) {
    throw "Dokany dependency fetcher missing: $dokanyFetch"
}
$dokanyMsi = (& $dokanyFetch | Select-Object -Last 1)
if ([string]::IsNullOrWhiteSpace($dokanyMsi)) {
    throw "Dokany dependency fetcher returned no MSI path."
}
Assert-NonEmptyFile $dokanyMsi
$archiveExtractor = @("7z.exe", "7za.exe", "7z", "7za") |
    ForEach-Object { Get-Command $_ -CommandType Application -ErrorAction SilentlyContinue } |
    Select-Object -First 1 |
    ForEach-Object { $_.Source }
if (-not $archiveExtractor) {
    $archiveCandidates = @(
        "$env:ProgramFiles\7-Zip\7z.exe",
        "${env:ProgramFiles(x86)}\7-Zip\7z.exe"
    )
    $archiveExtractor = $archiveCandidates |
        Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
        Select-Object -First 1
}
if (-not $archiveExtractor) {
    throw "7z wird zum Prüfen der eingebetteten Dokany-Installerdateien benötigt."
}

# Build
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Strawberry\c\bin;$env:Path"
$rustcVersion = (& rustc -vV | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "Rust-Host-Target konnte nicht bestimmt werden."
}
$hostMatch = [regex]::Match($rustcVersion, '(?m)^host:\s*([0-9A-Za-z_.-]+)\s*$')
if (-not $hostMatch.Success) {
    throw "Rust-Host-Target fehlt in rustc -vV."
}
$hostTriple = $hostMatch.Groups[1].Value
$nativeTargetDir = Join-Path $PSScriptRoot "target"
$nativeReleaseDir = Join-Path (Join-Path $nativeTargetDir $hostTriple) "release"
$nativeApp = Join-Path $nativeReleaseDir "smart_explorer.exe"
$nativeUpdater = Join-Path $nativeReleaseDir "smart_explorer_updater.exe"
$nativeCli = Join-Path $nativeReleaseDir "se.exe"
cargo build --locked --release --target-dir $nativeTargetDir --target $hostTriple --bin smart_explorer --bin smart_explorer_updater --bin se
if ($LASTEXITCODE -ne 0) { throw "Build fehlgeschlagen" }

$shareSrc = Join-Path $repoRoot "share-server"
$shareOut = Join-Path $ReleaseOutput "share-server"
if (Test-Path $shareSrc) {
    $shareTargetDir = Join-Path $shareSrc "target"
    $shareReleaseDir = Join-Path (Join-Path $shareTargetDir $hostTriple) "release"
    Push-Location $shareSrc
    try {
        cargo build --locked --release --target-dir $shareTargetDir --target $hostTriple --bin se-share-server
        if ($LASTEXITCODE -ne 0) { throw "Share-Server-Build fehlgeschlagen" }
    } finally {
        Pop-Location
    }
    New-Item -ItemType Directory -Force $shareOut | Out-Null
    Copy-Item (Join-Path $shareReleaseDir "se-share-server.exe") (Join-Path $shareOut "se-share-server.exe") -Force
} else {
    throw "Share-Server-Quellverzeichnis fehlt: $shareSrc"
}

$commandProject = Join-Path $PSScriptRoot "explorer-command"
$commandTargetDir = Join-Path $commandProject "target"
$commandReleaseDir = Join-Path (Join-Path $commandTargetDir $hostTriple) "release"
Push-Location $commandProject
try {
    cargo build --locked --release --target-dir $commandTargetDir --target $hostTriple
    if ($LASTEXITCODE -ne 0) { throw "Context-Menü-DLL-Build fehlgeschlagen" }
} finally {
    Pop-Location
}
$commandDll = Join-Path $commandReleaseDir "smart_explorer_command.dll"
Assert-NonEmptyFile $commandDll

# Installer neu bauen (fuer Neuinstallationen). EXE_SRC zeigt auf den nativen
# Windows-Build (installer.nsi defaultet auf den gnu-Cross-Pfad).
$makensis = $null
$makensisCmd = Get-Command makensis.exe -ErrorAction SilentlyContinue
if ($makensisCmd) {
    $makensis = $makensisCmd.Source
} else {
    $candidates = @(
        "$env:LOCALAPPDATA\electron-builder\Cache\nsis\nsis-3.0.4.1\Bin\makensis.exe",
        "$env:ProgramFiles\NSIS\makensis.exe",
        "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
    )
    $makensis = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if ($makensis) {
    New-Item -ItemType Directory -Force $ReleaseOutput | Out-Null
    $installer = Join-Path $ReleaseOutput "Smart Explorer Setup $version.exe"
    Remove-Item $installer -Force -ErrorAction SilentlyContinue
    & $makensis "/DVERSION=$version" "/DEXE_SRC=$nativeApp" "/DUPDATER_SRC=$nativeUpdater" "/DCLI_SRC=$nativeCli" "/DDOKANY_MSI_SRC=$dokanyMsi" "/DINSTALLER_OUT=$installer" "installer.nsi" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Installer-Build fehlgeschlagen: $installer"
    }
    Assert-NonEmptyFile $installer
    $verifyDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ("smart-explorer-installer-verify-{0}-{1}" -f $PID, [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $verifyDirectory | Out-Null
        $dokanyEntry = '$PLUGINSDIR/' + [System.IO.Path]::GetFileName($dokanyMsi)
        & $archiveExtractor e -y "-o$verifyDirectory" $installer $dokanyEntry | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Eingebettete Dokany-Dateien konnten nicht aus dem Installer gelesen werden."
        }
        $embeddedMsi = Join-Path $verifyDirectory ([System.IO.Path]::GetFileName($dokanyMsi))
        Assert-NonEmptyFile $embeddedMsi
        if ((Get-Item -LiteralPath $embeddedMsi).Length -ne (Get-Item -LiteralPath $dokanyMsi).Length -or
            (Get-FileHash -LiteralPath $embeddedMsi -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $dokanyMsi -Algorithm SHA256).Hash) {
            throw "Der Installer enthält nicht die verifizierte Dokany-MSI-Datei."
        }
    } finally {
        Remove-Item -LiteralPath $verifyDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Installer: $installer"
} else {
    throw "makensis nicht gefunden - ein Release ohne Installer ist unvollstaendig"
}

# Portable Kopie
New-Item -ItemType Directory -Force $ReleaseOutput | Out-Null
Copy-Item $nativeApp (Join-Path $ReleaseOutput "Smart Explorer.exe") -Force
Copy-Item $nativeUpdater (Join-Path $ReleaseOutput "Smart Explorer Updater.exe") -Force
Copy-Item $nativeCli (Join-Path $ReleaseOutput "se.exe") -Force
Copy-Item -LiteralPath $commandDll -Destination (Join-Path $ReleaseOutput "smart_explorer_command.dll") -Force

# Publish the Windows-only candidate only after every Windows artifact exists.
# The whole directory replaces the prior custom feed, so stale Linux payloads
# cannot survive under the new partial version.
$feedParent = Split-Path -Parent $resolvedFeed
New-Item -ItemType Directory -Force $feedParent | Out-Null
$feedStage = Join-Path $feedParent (".windows-feed-candidate.{0}.{1}" -f $PID, [guid]::NewGuid().ToString("N"))
$versionStage = Join-Path $feedParent (".windows-version.{0}.{1}.tmp" -f $PID, [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force $feedStage | Out-Null
try {
    $payloads = @(
        [pscustomobject]@{ Name = "smart_explorer.exe"; Source = $nativeApp },
        [pscustomobject]@{ Name = "smart_explorer_updater.exe"; Source = $nativeUpdater },
        [pscustomobject]@{ Name = "se.exe"; Source = $nativeCli }
    )
    $manifest = @("version=$version", "source_commit=$sourceCommit")
    foreach ($payload in $payloads) {
        $name = $payload.Name
        $source = $payload.Source
        Assert-NonEmptyFile $source
        $staged = Join-Path $feedStage $name
        Copy-Item -LiteralPath $source -Destination $staged -Force
        Write-Sha256File $staged
        $expected = ((Get-Content -LiteralPath "$staged.sha256" -TotalCount 1) -split '\s+')[0]
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $staged).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "SHA256-Prüfung fehlgeschlagen: $staged"
        }
        $manifest += "$name=$actual"
    }
    Write-AsciiLf (Join-Path $feedStage "windows-build.manifest") $manifest
    Assert-NonEmptyFile (Join-Path $feedStage "windows-build.manifest")
    if (-not $DeferFeedVersion) {
        Write-AsciiLf $versionStage @($version)
        Publish-FeedDirectoryTransaction $feedStage $resolvedFeed $versionStage $version
        Write-Host "Expliziter Windows-only Feed atomar aktualisiert: $resolvedFeed (v$version)"
    } else {
        Publish-FeedDirectoryTransaction $feedStage $resolvedFeed "" $version
        Write-Host "Windows-Payload-Stage erstellt; version.txt bleibt fuer den Gesamt-Commit unverändert."
    }
} finally {
    Remove-Item -LiteralPath $feedStage -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $versionStage -Force -ErrorAction SilentlyContinue
}
Write-Host "Fertig."
