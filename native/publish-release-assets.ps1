# Publishes one release built by .github/workflows/release.yml (plan
# docs/plaene/2026-11-01-idealzustand, D8). The 28 assets (26 build assets
# plus the signed release manifest, plan E20) live only in the GitHub Release;
# Git gets one release commit with the version and the prepared sources
# (embedded SSH-agent bundles, license notices) and its tag.
#
# Order: require the signing key, bind to the exact main source, assemble the
# artifacts of the build jobs in the layout of Get-PublicationReleaseAssetMap,
# verify every sidecar, the installer's embedded payloads and the 26 expected
# build assets, write release-manifest over them, sign it with
# SE_UPDATE_SIGNING_KEY into release-manifest.sig and verify that signature
# against the updater's public key, then commit, tag, upload the 28 assets
# into a draft, compare every uploaded digest with the expected hash, and only
# then publish it as the latest release. A dry run uploads into a draft under
# its own tag name, compares, and deletes it again.
#
# Manifest format (read by the updater, engine updater/os/shared/
# release_signature.rs): UTF-8 without BOM, every line ends with LF; line 1
# "smart-explorer-release-manifest 1", line 2 "version X.Y.Z", then one line
# "<sha256 lowercase hex> <size in bytes> <asset name>" per asset except the
# manifest and its signature, ordered by the ordinal bytes of the name. The
# signature is Ed25519 (RFC 8032, pure) over the exact manifest bytes, stored
# as 128 lowercase hex characters and one LF.
#
# Inputs (environment): PLAN_VERSION, SOURCE_SHA, DRY_RUN ("true"/"false"),
# SE_UPDATE_SIGNING_KEY (Ed25519 private key, PKCS#8 PEM), GH_TOKEN,
# RUNNER_TEMP, GITHUB_REPOSITORY, GITHUB_RUN_ID.
$ErrorActionPreference = "Stop"

# The signing key leaves the environment at once, so no child process (git,
# gh, curl, se.exe) inherits it; it is never printed.
$signingKeyPem = "$env:SE_UPDATE_SIGNING_KEY"
Remove-Item -LiteralPath Env:SE_UPDATE_SIGNING_KEY -ErrorAction SilentlyContinue
if ([string]::IsNullOrWhiteSpace($signingKeyPem)) {
    throw "The SE_UPDATE_SIGNING_KEY repository secret is missing: the release manifest cannot be signed, nothing is committed, tagged or published."
}
# The raw Ed25519 public key (RFC 8032, 32 bytes) the updater embeds as
# RELEASE_PUBLIC_KEY; a signature that does not verify against it stops the release.
$releasePublicKeyHex = "1a3cc5ff345526a71e5624165b47cfc27f766af707093bfb6aed28cefba0cd6b"
$manifestAsset = "release-manifest"
$signatureAsset = "release-manifest.sig"
$buildAssetCount = 26
$releaseAssetCount = $buildAssetCount + 2

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

# OpenSSL 3 signs and verifies pure Ed25519 with pkeyutl -rawin; 1.1.1 lacks
# -rawin. windows-2025 ships OpenSSL 3 on PATH, Git for Windows its own copy.
function Find-OpenSsl3 {
    $candidates = @()
    $onPath = Get-Command openssl -CommandType Application -ErrorAction SilentlyContinue
    if ($onPath) { $candidates += @($onPath | ForEach-Object { $_.Source }) }
    if ($env:ProgramFiles) {
        $candidates += @(
            (Join-Path $env:ProgramFiles "OpenSSL/bin/openssl.exe"),
            (Join-Path $env:ProgramFiles "Git/usr/bin/openssl.exe"),
            (Join-Path $env:ProgramFiles "Git/mingw64/bin/openssl.exe"))
    }
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        $reported = "$(& $candidate version 2>$null)".Trim()
        if ($LASTEXITCODE -eq 0 -and $reported -match '^OpenSSL ([3-9]|[1-9]\d+)\.') {
            Write-Host "Release signing tool: $candidate ($reported)."
            return $candidate
        }
    }
    throw "No OpenSSL 3 (pkeyutl -rawin for Ed25519) found; tried: $($candidates -join ', ')."
}

# A directory only the current account can read, for the private key file.
function New-PrivateDirectory([string]$Path) {
    Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $Path | Out-Null
    if ($IsWindows) {
        $account = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
        $null = & icacls $Path /inheritance:r /grant:r "${account}:(OI)(CI)F"
        if ($LASTEXITCODE -ne 0) { throw "Could not restrict access to $Path (icacls exit $LASTEXITCODE)." }
    } else {
        Invoke-Checked chmod @("700", $Path)
    }
}

# Writes release-manifest over the verified build assets (format in the
# header), signs it with the private key, verifies the signature against
# $releasePublicKeyHex and writes release-manifest.sig. Returns the two new
# assets in the shape of Get-PublicationExpectedReleaseAsset.
function New-SignedReleaseManifest([object[]]$BuildAssets, [string]$UploadDir, [string]$OpenSsl) {
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    $byName = @{}
    foreach ($asset in $BuildAssets) {
        $name = "$($asset.Name)"
        if ($name -eq $manifestAsset -or $name -eq $signatureAsset) { throw "Build asset '$name' collides with the release manifest." }
        # Printable ASCII keeps the ordinal order equal to the UTF-8 byte order.
        if ($name.Length -eq 0 -or $name -ne $name.Trim() -or $name -cmatch '[^\x20-\x7e]') {
            throw "Asset name '$name' cannot appear in the release manifest (printable ASCII, no leading or trailing space)."
        }
        if ($byName.ContainsKey($name)) { throw "Asset name '$name' appears twice." }
        $byName[$name] = $asset
    }
    # Ordinal order of the names (ASCII here, so equal to their UTF-8 bytes).
    $names = [System.Collections.Generic.List[string]]::new([string[]]@($byName.Keys))
    $names.Sort([System.StringComparer]::Ordinal)
    $text = [System.Text.StringBuilder]::new()
    $null = $text.Append("smart-explorer-release-manifest 1`n").Append("version $version`n")
    foreach ($name in $names) {
        $asset = $byName[$name]
        $hash = "$($asset.Sha256)".ToLowerInvariant()
        if ($hash -notmatch '^[0-9a-f]{64}$') { throw "Asset $name has no SHA-256 for the manifest." }
        $size = ([long]$asset.Size).ToString([System.Globalization.CultureInfo]::InvariantCulture)
        $null = $text.Append("$hash $size $name`n")
    }
    $manifestPath = Join-Path $UploadDir $manifestAsset
    [System.IO.File]::WriteAllText($manifestPath, $text.ToString(), $utf8)

    [byte[]]$publicDer = [byte[]](0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00) +
        [System.Convert]::FromHexString($releasePublicKeyHex)
    $keyDir = Join-Path $env:RUNNER_TEMP "release-signing"
    New-PrivateDirectory $keyDir
    try {
        $keyFile = Join-Path $keyDir "signing-key.pem"
        $publicFile = Join-Path $keyDir "release-public.pem"
        $signatureBin = Join-Path $keyDir "release-manifest.sig.bin"
        [System.IO.File]::WriteAllText($keyFile, $signingKeyPem, $utf8)
        [System.IO.File]::WriteAllText($publicFile,
            "-----BEGIN PUBLIC KEY-----`n$([System.Convert]::ToBase64String($publicDer))`n-----END PUBLIC KEY-----`n", $utf8)
        # Everything OpenSSL prints goes to the host, never into this
        # function's output (the returned assets).
        Invoke-Checked $OpenSsl @("pkeyutl", "-sign", "-rawin", "-inkey", $keyFile, "-in", $manifestPath, "-out", $signatureBin) |
            Out-Host
        $signature = [System.IO.File]::ReadAllBytes($signatureBin)
        if ($signature.Length -ne 64) { throw "The release manifest signature has $($signature.Length) bytes, expected 64 (Ed25519)." }
        & $OpenSsl pkeyutl -verify -pubin -inkey $publicFile -rawin -in $manifestPath -sigfile $signatureBin 2>&1 | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "The release manifest signature does not verify against the updater's public key $($releasePublicKeyHex): SE_UPDATE_SIGNING_KEY holds another key."
        }
    } finally {
        Remove-Item -LiteralPath $keyDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    $signaturePath = Join-Path $UploadDir $signatureAsset
    [System.IO.File]::WriteAllText($signaturePath, "$([System.Convert]::ToHexString($signature).ToLowerInvariant())`n", $utf8)
    Write-Host "Release manifest signed and verified against public key $releasePublicKeyHex ($($names.Count) assets, version $version)."
    foreach ($path in @($manifestPath, $signaturePath)) {
        [pscustomobject]@{
            Name = Split-Path -Leaf $path
            LocalPath = $path
            Size = [long](Get-Item -LiteralPath $path).Length
            Sha256 = (Get-PublicationSha256 $path).ToLowerInvariant()
        }
    }
}

$openssl = Find-OpenSsl3

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
if ($expected.Count -ne $buildAssetCount) { throw "Expected $buildAssetCount build assets, assembled $($expected.Count)." }
$upload = Join-Path $env:RUNNER_TEMP "release-upload"
Remove-Item -LiteralPath $upload -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $upload | Out-Null
foreach ($asset in $expected) {
    $copy = Join-Path $upload $asset.Name
    Copy-Item -LiteralPath $asset.LocalPath -Destination $copy
    # The manifest signs the bytes that are uploaded, so the copy must be them.
    $copyHash = (Get-PublicationSha256 $copy).ToLowerInvariant()
    $copySize = [long](Get-Item -LiteralPath $copy).Length
    if ($copyHash -ne "$($asset.Sha256)".ToLowerInvariant() -or $copySize -ne [long]$asset.Size) {
        throw "Upload copy of $($asset.Name) differs: $copySize $copyHash, expected $($asset.Size) $($asset.Sha256)."
    }
}
Write-Host "$buildAssetCount build assets assembled and verified for $tag."

# 3b. The signed release manifest over exactly these 26 build assets (plan E20).
$expected += @(New-SignedReleaseManifest -BuildAssets $expected -UploadDir $upload -OpenSsl $openssl)
if ($expected.Count -ne $releaseAssetCount) { throw "Expected $releaseAssetCount release assets, assembled $($expected.Count)." }
Write-Host "$releaseAssetCount release assets ($manifestAsset, $signatureAsset included) ready for $tag."

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
        Invoke-Checked git @("commit", "-m", "Release Smart Explorer $tag", "-m", "Assets: GitHub Release $tag ($releaseAssetCount files, built and signed by .github/workflows/release.yml).")
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
    if ($seen.Count -ne $releaseAssetCount) { throw "The draft holds $($seen.Count) assets, expected $releaseAssetCount." }
    Write-Host "All $releaseAssetCount uploaded assets match their expected size and SHA-256."
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
