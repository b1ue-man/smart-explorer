# Publishes one release built by .github/workflows/release.yml (plan
# docs/plaene/2026-11-01-idealzustand, D8). The 26 assets live only in the
# GitHub Release; Git gets one release commit with the version and the
# prepared sources (embedded SSH-agent bundles, license notices) and its tag.
#
# Order: bind to the exact main source, assemble the artifacts of the build
# jobs in the layout of Get-PublicationReleaseAssetMap, verify every sidecar,
# the installer's embedded payloads and the 26 expected assets, then commit,
# tag, upload into a draft, compare every uploaded digest with the expected
# hash, and only then publish it as the latest release. A dry run uploads
# into a draft under its own tag name, compares, and deletes it again.
#
# Inputs (environment): PLAN_VERSION, SOURCE_SHA, DRY_RUN ("true"/"false"),
# GH_TOKEN, RUNNER_TEMP, GITHUB_REPOSITORY, GITHUB_RUN_ID.
$ErrorActionPreference = "Stop"

$scriptRoot = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $scriptRoot "..")).Path
. (Join-Path $scriptRoot "release-publication.ps1")

$version = $env:PLAN_VERSION
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "PLAN_VERSION '$version' is no release version." }
$source = "$env:SOURCE_SHA".ToLowerInvariant()
$dryRun = $env:DRY_RUN -eq "true"
$repository = $env:GITHUB_REPOSITORY
$artifacts = Join-Path $env:RUNNER_TEMP "artifacts"
$tag = "v$version"
$releaseTag = if ($dryRun) { "dryrun-$tag-$env:GITHUB_RUN_ID" } else { $tag }

function Invoke-Checked([string]$File, [string[]]$Arguments) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE." }
}

function Copy-Required([string]$From, [string]$To) {
    if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "Build artifact missing: $From" }
    New-Item -ItemType Directory -Force (Split-Path -Parent $To) | Out-Null
    Copy-Item -LiteralPath $From -Destination $To -Force
}

# 1. The release commit follows exactly the source the builds used.
Push-Location $repoRoot
try {
    Invoke-Checked git @("fetch", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main")
    $main = (git rev-parse 'refs/remotes/origin/main^{commit}').Trim()
    $head = (git rev-parse 'HEAD^{commit}').Trim()
    if ($main -ne $source -or $head -ne $source) {
        throw "Release source drift: source=$source HEAD=$head origin/main=$main."
    }
    if (-not $dryRun -and (git ls-remote --tags origin "refs/tags/$tag")) {
        throw "$tag already exists; tags are immutable."
    }
} finally {
    Pop-Location
}

# 2. The layout Get-PublicationReleaseAssetMap reads, outside the checkout.
$stage = Join-Path $env:RUNNER_TEMP "release-stage"
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
$release = Join-Path $stage "release-native"
$feed = Join-Path $release "update-feed"
$share = Join-Path $release "share-server"
New-Item -ItemType Directory -Force $feed, $share | Out-Null

$windowsFeed = Join-Path $artifacts "windows/update-feed"
$windowsRelease = Join-Path $artifacts "windows/release"
$linuxFeed = Join-Path $artifacts "linux/update-feed"
foreach ($name in @(
        "smart_explorer.exe", "smart_explorer_updater.exe", "se.exe", "se-mcp-launcher.exe")) {
    Copy-Required (Join-Path $windowsFeed $name) (Join-Path $feed $name)
    Copy-Required (Join-Path $windowsFeed "$name.sha256") (Join-Path $feed "$name.sha256")
}
foreach ($name in @("smart_explorer", "smart_explorer_updater", "se", "se-mcp-launcher")) {
    Copy-Required (Join-Path $linuxFeed $name) (Join-Path $feed $name)
    Copy-Required (Join-Path $linuxFeed "$name.sha256") (Join-Path $feed "$name.sha256")
}
$installerName = "Smart Explorer Setup $version.exe"
Copy-Required (Join-Path $windowsRelease $installerName) (Join-Path $release $installerName)
Copy-Required (Join-Path $windowsRelease "smart_explorer_command.dll") (Join-Path $release "smart_explorer_command.dll")
Copy-Required (Join-Path $windowsRelease "share-server/se-share-server.exe") (Join-Path $share "se-share-server.exe")
Copy-Required (Join-Path $artifacts "linux/share-server/se-share-server-linux") (Join-Path $share "se-share-server-linux")
$apk = Join-Path $feed "smart-explorer-android.apk"
Copy-Required (Join-Path $artifacts "android/smart-explorer-android.apk") $apk
$apkHash = Get-PublicationSha256 $apk
[System.IO.File]::WriteAllText("$apk.sha256", "$apkHash  smart-explorer-android.apk`n", [System.Text.UTF8Encoding]::new($false))
[System.IO.File]::WriteAllText((Join-Path $feed "version.txt"), "$version`n", [System.Text.UTF8Encoding]::new($false))
Copy-Required (Join-Path $repoRoot "install-linux.sh") (Join-Path $stage "install-linux.sh")
Copy-Required (Join-Path $repoRoot "native/install-login-service.ps1") (Join-Path $stage "native/install-login-service.ps1")
Copy-Required (Join-Path $repoRoot "native/install-login-service.sh") (Join-Path $stage "native/install-login-service.sh")

# 3. Every sidecar binds its bytes, the installer embeds exactly the feed's
# Windows payloads, the Windows CLI reports the planned version.
foreach ($name in @(
        "smart_explorer.exe", "smart_explorer_updater.exe", "se.exe", "se-mcp-launcher.exe",
        "smart_explorer", "smart_explorer_updater", "se", "se-mcp-launcher",
        "smart-explorer-android.apk")) {
    $null = Assert-PublicationHashSidecar -Feed $feed -PayloadName $name
}
Assert-PublicationInstallerPayloads -RepoRoot $stage -Installer (Join-Path $release $installerName) -Feed $feed
$reported = (& (Join-Path $feed "se.exe") --version | Out-String).Trim()
if ($reported -ne "se $version") { throw "se.exe reports '$reported', expected 'se $version'." }
$expected = @(Get-PublicationReleaseAssetMap -RepoRoot $stage -Version $version |
    ForEach-Object { Get-PublicationExpectedReleaseAsset -Asset $_ -Version $version })
if ($expected.Count -ne 26) { throw "Expected 26 release assets, assembled $($expected.Count)." }
$upload = Join-Path $env:RUNNER_TEMP "release-upload"
Remove-Item -LiteralPath $upload -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $upload | Out-Null
foreach ($asset in $expected) {
    Copy-Item -LiteralPath $asset.LocalPath -Destination (Join-Path $upload $asset.Name)
}
Write-Host "26 release assets assembled and verified for $tag."

# 4. The release commit and tag (not in a dry run): the version and the
# prepared sources, never a build.
Push-Location $repoRoot
try {
    if (-not $dryRun) {
        $prepared = Join-Path $artifacts "prepared"
        $sources = @(
            "native/Cargo.toml", "native/Cargo.lock",
            "native/agent-bin/se-agent-x86_64-linux-musl", "native/agent-bin/se-agent-aarch64-linux-musl",
            "third-party/rust-crates/THIRD-PARTY-LICENSES.txt")
        foreach ($path in $sources) {
            Copy-Required (Join-Path $prepared $path) (Join-Path $repoRoot $path)
        }
        Invoke-Checked git @("config", "user.name", "github-actions[bot]")
        Invoke-Checked git @("config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com")
        Invoke-Checked git (@("add", "--") + $sources)
        Invoke-Checked git @("commit", "-m", "Release Smart Explorer $tag", "-m", "Assets: GitHub Release $tag (26 files, built by .github/workflows/release.yml).")
        Invoke-Checked git @("tag", $tag)
        # Fast-forward only: a main that moved since the builds refuses the push.
        Invoke-Checked git @("push", "origin", "HEAD:refs/heads/main")
        Invoke-Checked git @("push", "origin", "refs/tags/$tag")
    }
} finally {
    Pop-Location
}

# 5. Draft, upload, compare every digest, then publish (or delete a dry run).
$files = @(Get-ChildItem -LiteralPath $upload -File | ForEach-Object { $_.FullName })
$createArgs = @("release", "create", $releaseTag, "--repo", $repository, "--draft",
    "--title", "Smart Explorer $tag", "--notes", "Smart Explorer $tag")
if ($dryRun) { $createArgs += @("--target", $source) } else { $createArgs += "--verify-tag" }
Invoke-Checked gh ($createArgs + $files)
try {
    # A draft is reachable only through the list of releases.
    $releases = gh api "repos/$repository/releases?per_page=50" | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Could not list the releases of $repository." }
    $draft = @($releases | Where-Object { $_.tag_name -eq $releaseTag }) | Select-Object -First 1
    if (-not $draft) { throw "The draft $releaseTag is not listed." }
    $seen = @{}
    foreach ($remote in @($draft.assets)) {
        $seen[$remote.name] = @{ Size = [long]$remote.size; Digest = "$($remote.digest)" }
    }
    foreach ($asset in $expected) {
        $remote = $seen[$asset.Name]
        if (-not $remote) { throw "Uploaded release lacks $($asset.Name)." }
        if ($remote.Size -ne $asset.Size -or $remote.Digest -ne "sha256:$($asset.Sha256)") {
            throw "Uploaded $($asset.Name) differs: $($remote.Size) $($remote.Digest), expected $($asset.Size) sha256:$($asset.Sha256)."
        }
    }
    if ($seen.Count -ne 26) { throw "The draft holds $($seen.Count) assets, expected 26." }
    Write-Host "All 26 uploaded assets match their expected size and SHA-256."
    if ($dryRun) { return }
    Invoke-Checked gh @("release", "edit", $tag, "--repo", $repository, "--draft=false", "--latest")
    $feedUrl = "https://github.com/$repository/releases/latest/download/version.txt"
    $deadline = (Get-Date).AddMinutes(5)
    do {
        # The asset arrives as application/octet-stream, which Invoke-WebRequest
        # returns as bytes; curl gives the text (release 41369467666).
        $latest = "$(& curl.exe -sSL $feedUrl)".Trim()
        if ($latest -eq $version) { break }
        Start-Sleep -Seconds 10
    } while ((Get-Date) -lt $deadline)
    if ($latest -ne $version) { throw "The release feed answers '$latest', expected $version." }
    Write-Host "Smart Explorer $tag published; the release feed answers $version."
} finally {
    if ($dryRun) {
        # A draft never created its tag, so there is no tag to clean up.
        & gh release delete $releaseTag --repo $repository --yes
        if ($LASTEXITCODE -ne 0) { throw "Could not delete the dry-run draft $releaseTag." }
        Write-Host "Dry run: draft $releaseTag deleted."
    }
}
exit 0
