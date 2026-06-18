# Publish a new Smart Explorer version to the local update feed + rebuild the installer.
#
# Workflow:
#   1. Version in Cargo.toml erhoehen (z.B. 0.2.1)
#   2. .\publish-update.ps1 ausfuehren
#   3. Fertig — alle installierten Instanzen updaten sich beim naechsten Start.
#
# Optional: -Feed <Pfad> fuer einen anderen Feed-Ordner (z.B. Netzlaufwerk).

param(
    [string]$Feed = "C:\Users\Silas\Desktop\fun-projects\smartExplorer\release-native\update-feed"
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# Version aus Cargo.toml lesen
$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Baue Version $version ..."

# Build
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Strawberry\c\bin;$env:Path"
cargo build --release --bin smart_explorer --bin smart_explorer_updater
if ($LASTEXITCODE -ne 0) { throw "Build fehlgeschlagen" }

# Feed aktualisieren (EXE zuerst, version.txt zuletzt — Clients sehen die neue
# Version erst, wenn die EXE schon vollstaendig da ist). Dateiname
# smart_explorer.exe (ohne Leerzeichen) ist identisch zum Git/HTTPS-Feed unter
# release-native\update-feed, der ins Repo committet und ueber
# raw.githubusercontent.com ausgeliefert wird ("Git als Update-Quelle").
New-Item -ItemType Directory -Force $Feed | Out-Null
Copy-Item "target\release\smart_explorer.exe" "$Feed\smart_explorer.exe" -Force
Copy-Item "target\release\smart_explorer_updater.exe" "$Feed\smart_explorer_updater.exe" -Force
function Write-Sha256File([string]$Path) {
    $hash = (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLowerInvariant()
    $name = [System.IO.Path]::GetFileName($Path)
    Set-Content "$Path.sha256" "$hash  $name" -Encoding ascii
}
Write-Sha256File "$Feed\smart_explorer.exe"
Write-Sha256File "$Feed\smart_explorer_updater.exe"
Set-Content "$Feed\version.txt" $version -Encoding ascii
Write-Host "Feed aktualisiert: $Feed (v$version)"

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
    & $makensis "/DVERSION=$version" "/DEXE_SRC=target\release\smart_explorer.exe" "/DUPDATER_SRC=target\release\smart_explorer_updater.exe" "installer.nsi" | Out-Null
    Write-Host "Installer: ..\release-native\Smart Explorer Setup $version.exe"
} else {
    Write-Warning "makensis nicht gefunden - Installer uebersprungen"
}

# Portable Kopie
Copy-Item "target\release\smart_explorer.exe" "..\release-native\Smart Explorer.exe" -Force
Copy-Item "target\release\smart_explorer_updater.exe" "..\release-native\Smart Explorer Updater.exe" -Force
Write-Host "Fertig."
