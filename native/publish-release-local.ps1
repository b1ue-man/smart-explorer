# Build a complete local Smart Explorer release on Windows with WSL available.
#
# This wraps the existing Windows release script and the Linux/WSL feed builder
# so release-native/update-feed is complete before it is committed or tagged.

param(
    [switch]$SkipLinuxFeed,
    [switch]$NoBootstrapZig,
    [switch]$CheckEnvOnly
)

$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptRoot "..")
$feed = Join-Path $repoRoot "release-native\update-feed"

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
    Write-Warning "makensis.exe not found; publish-update.ps1 can continue but installer output may be skipped."
}

if ($CheckEnvOnly -and $SkipLinuxFeed) {
    Write-Host "Windows release environment OK for v$version."
    exit 0
}

if ($CheckEnvOnly) {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) {
        throw "wsl.exe not found. Install WSL or rerun with -SkipLinuxFeed for an explicit partial feed."
    }

    $repoRootForWsl = ($repoRoot.Path -replace '\\', '/')
    $repoRootWsl = (& wsl.exe wslpath -a $repoRootForWsl).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $repoRootWsl) {
        throw "Could not translate repo path for WSL."
    }

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

Push-Location $scriptRoot
try {
    & .\publish-update.ps1 -AllowPartialFeed
    if ($LASTEXITCODE -ne 0) {
        throw "Windows release build failed."
    }
} finally {
    Pop-Location
}

if (-not $SkipLinuxFeed) {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) {
        throw "wsl.exe not found. Install WSL or rerun with -SkipLinuxFeed for an explicit partial feed."
    }

    $repoRootForWsl = ($repoRoot.Path -replace '\\', '/')
    $repoRootWsl = (& wsl.exe wslpath -a $repoRootForWsl).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $repoRootWsl) {
        throw "Could not translate repo path for WSL."
    }

    $linuxArgs = "--write-version"
    if ($NoBootstrapZig) {
        $linuxArgs += " --no-bootstrap-zig"
    }

    Invoke-Checked -ErrorMessage "Linux feed build failed." -Command {
        & wsl.exe bash -lc "cd '$repoRootWsl' && native/publish-linux-feed-wsl.sh $linuxArgs"
    }
}

$feedVersionPath = Join-Path $feed "version.txt"
if (-not (Test-Path $feedVersionPath)) {
    throw "Feed version file missing: $feedVersionPath"
}
$feedVersion = (Get-Content $feedVersionPath -TotalCount 1).Trim()
if ($feedVersion -ne $version) {
    throw "Feed version '$feedVersion' does not match Cargo.toml version '$version'."
}

$requiredFeedFiles = @(
    "smart_explorer.exe",
    "smart_explorer.exe.sha256",
    "smart_explorer_updater.exe",
    "smart_explorer_updater.exe.sha256",
    "smart_explorer",
    "smart_explorer.sha256",
    "smart_explorer_updater",
    "smart_explorer_updater.sha256"
)
foreach ($name in $requiredFeedFiles) {
    $path = Join-Path $feed $name
    if (-not (Test-Path $path)) {
        throw "Required feed file missing: $path"
    }
}

if (-not $SkipLinuxFeed) {
    $repoRootForWsl = ($repoRoot.Path -replace '\\', '/')
    $repoRootWsl = (& wsl.exe wslpath -a $repoRootForWsl).Trim()
    Invoke-Checked -ErrorMessage "Feed SHA256 verification failed." -Command {
        & wsl.exe bash -lc "cd '$repoRootWsl/release-native/update-feed' && sha256sum -c smart_explorer.exe.sha256 && sha256sum -c smart_explorer_updater.exe.sha256 && sha256sum -c smart_explorer.sha256 && sha256sum -c smart_explorer_updater.sha256"
    }
}

Write-Host "Complete local release artifacts staged for v$version."
