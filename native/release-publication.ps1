# Publication helpers for the one complete Smart Explorer release wrapper.
#
# This file is dot-sourced by publish-release-local.ps1.  Every public helper
# accepts its repository context explicitly so importing it has no side effects.

function ConvertTo-ReleasePublicationSafeText {
    param([AllowNull()][string]$Text)

    if ($null -eq $Text) {
        return ""
    }
    $safe = $Text
    foreach ($name in @("GH_TOKEN", "GITHUB_TOKEN")) {
        $secret = [Environment]::GetEnvironmentVariable($name, "Process")
        if (-not [string]::IsNullOrEmpty($secret)) {
            $safe = $safe.Replace($secret, "<redacted>")
        }
    }
    $safe = [regex]::Replace(
        $safe,
        '(?i)(https?://)[^/@\s:]+:[^/@\s]+@',
        '$1<redacted>@'
    )
    $safe = [regex]::Replace(
        $safe,
        '(?i)\b(?:github_pat_[0-9A-Za-z_]+|gh[pousr]_[0-9A-Za-z]+)\b',
        '<redacted>'
    )
    return $safe
}

function Invoke-ReleasePublicationProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [ValidateRange(1, 7200)][int]$TimeoutSeconds = 300,
        [switch]$AllowFailure
    )

    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    foreach ($argument in $Arguments) {
        [void]$start.ArgumentList.Add($argument)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $start.Environment[$entry.Key] = [string]$entry.Value
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        try {
            if (-not $process.Start()) {
                throw "process did not start"
            }
        } catch {
            $detail = ConvertTo-ReleasePublicationSafeText $_.Exception.Message
            throw "Could not start release publication process '$FilePath': $detail"
        }
        $process.StandardInput.Close()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try {
                $process.Kill($true)
            } catch {
                # The process may have exited between WaitForExit and Kill.
            }
            throw "Release publication process '$FilePath' exceeded $TimeoutSeconds seconds."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $result = [pscustomobject]@{
            ExitCode = $process.ExitCode
            StdOut = ConvertTo-ReleasePublicationSafeText $stdout
            StdErr = ConvertTo-ReleasePublicationSafeText $stderr
            Output = ConvertTo-ReleasePublicationSafeText (($stdout + "`n" + $stderr).Trim())
        }
        if (-not $AllowFailure -and $result.ExitCode -ne 0) {
            $detail = $result.Output
            if ([string]::IsNullOrWhiteSpace($detail)) {
                $detail = "exit code $($result.ExitCode)"
            }
            throw "Release publication process '$FilePath' failed: $detail"
        }
        return $result
    } finally {
        $process.Dispose()
    }
}

function Invoke-ReleasePublicationGit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure
    )

    $gitArguments = @("-c", "credential.interactive=never") + $Arguments
    return Invoke-ReleasePublicationProcess `
        -FilePath "git" `
        -Arguments $gitArguments `
        -WorkingDirectory $RepoRoot `
        -Environment @{
            GIT_TERMINAL_PROMPT = "0"
            GCM_INTERACTIVE = "Never"
        } `
        -AllowFailure:$AllowFailure
}

function Get-PublicationReleaseAssetMap {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    $releaseRoot = Join-Path $RepoRoot "release-native"
    $feed = Join-Path $releaseRoot "update-feed"
    $items = @(
        [pscustomobject]@{
            LocalPath = Join-Path $releaseRoot "Smart Explorer Setup $Version.exe"
            PublishedName = "Smart.Explorer.Setup.$Version.exe"
        },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer.exe"; PublishedName = "smart_explorer.exe" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer_updater.exe"; PublishedName = "smart_explorer_updater.exe" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "se.exe"; PublishedName = "se.exe" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer.exe.sha256"; PublishedName = "smart_explorer.exe.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer_updater.exe.sha256"; PublishedName = "smart_explorer_updater.exe.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "se.exe.sha256"; PublishedName = "se.exe.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer"; PublishedName = "smart_explorer" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer_updater"; PublishedName = "smart_explorer_updater" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "se"; PublishedName = "se" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer.sha256"; PublishedName = "smart_explorer.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "smart_explorer_updater.sha256"; PublishedName = "smart_explorer_updater.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "se.sha256"; PublishedName = "se.sha256" },
        [pscustomobject]@{ LocalPath = Join-Path $RepoRoot "install-linux.sh"; PublishedName = "install-linux.sh" },
        [pscustomobject]@{ LocalPath = Join-Path $releaseRoot "smart_explorer_command.dll"; PublishedName = "smart_explorer_command.dll" },
        [pscustomobject]@{ LocalPath = Join-Path $releaseRoot "share-server/se-share-server.exe"; PublishedName = "se-share-server.exe" },
        [pscustomobject]@{ LocalPath = Join-Path $releaseRoot "share-server/se-share-server-linux"; PublishedName = "se-share-server-linux" },
        [pscustomobject]@{ LocalPath = Join-Path $feed "version.txt"; PublishedName = "version.txt" }
    )
    if ($items.Count -ne 18) {
        throw "Internal release publication map must contain exactly 18 assets."
    }
    return $items
}

function Get-PublicationCargoVersion {
    param([Parameter(Mandatory = $true)][string]$CargoToml)

    if (-not (Test-Path -LiteralPath $CargoToml -PathType Leaf)) {
        throw "Cargo manifest is missing: $CargoToml"
    }
    $match = Select-String -LiteralPath $CargoToml -Pattern '^version\s*=\s*"([^"]+)"' |
        Select-Object -First 1
    if (-not $match) {
        throw "Could not read the package version from $CargoToml."
    }
    return $match.Matches[0].Groups[1].Value
}

function Assert-PublicationNonEmptyFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required release publication file is missing: $Path"
    }
    if ((Get-Item -LiteralPath $Path).Length -le 0) {
        throw "Required release publication file is empty: $Path"
    }
}

function Get-PublicationSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    Assert-PublicationNonEmptyFile $Path
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-PublicationHashSidecar {
    param(
        [Parameter(Mandatory = $true)][string]$Feed,
        [Parameter(Mandatory = $true)][string]$PayloadName
    )

    $payload = Join-Path $Feed $PayloadName
    $sidecar = "$payload.sha256"
    Assert-PublicationNonEmptyFile $payload
    Assert-PublicationNonEmptyFile $sidecar
    $line = (Get-Content -LiteralPath $sidecar -TotalCount 1).Trim()
    $match = [regex]::Match($line, '^([0-9A-Fa-f]{64})\s+\*?(.+)$')
    if (-not $match.Success) {
        throw "Invalid SHA-256 sidecar format: $sidecar"
    }
    if ($match.Groups[2].Value -ne $PayloadName) {
        throw "SHA-256 sidecar '$sidecar' names '$($match.Groups[2].Value)', expected '$PayloadName'."
    }
    $actual = Get-PublicationSha256 $payload
    if ($match.Groups[1].Value.ToLowerInvariant() -ne $actual) {
        throw "SHA-256 sidecar does not bind the exact '$PayloadName' bytes."
    }
    return $actual
}

function Assert-ReleasePublicationCandidate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    $cargoToml = Join-Path $RepoRoot "native/Cargo.toml"
    $cargoVersion = Get-PublicationCargoVersion $cargoToml
    if ($cargoVersion -ne $Version) {
        throw "Cargo.toml version '$cargoVersion' does not match release '$Version'."
    }

    $cargoLock = Join-Path $RepoRoot "native/Cargo.lock"
    Assert-PublicationNonEmptyFile $cargoLock
    $lockText = Get-Content -LiteralPath $cargoLock -Raw
    $lockMatch = [regex]::Match(
        $lockText,
        '(?ms)^\[\[package\]\]\s*\r?\nname = "smart_explorer"\s*\r?\nversion = "([^"]+)"'
    )
    if (-not $lockMatch.Success -or $lockMatch.Groups[1].Value -ne $Version) {
        throw "native/Cargo.lock does not bind smart_explorer version '$Version'."
    }

    $releaseRoot = Join-Path $RepoRoot "release-native"
    $feed = Join-Path $releaseRoot "update-feed"
    $feedVersionPath = Join-Path $feed "version.txt"
    Assert-PublicationNonEmptyFile $feedVersionPath
    $feedVersion = (Get-Content -LiteralPath $feedVersionPath -Raw).Trim()
    if ($feedVersion -ne $Version) {
        throw "Feed version '$feedVersion' does not match release '$Version'."
    }

    $hashes = @{}
    foreach ($payload in @(
        "smart_explorer.exe",
        "smart_explorer_updater.exe",
        "se.exe",
        "smart_explorer",
        "smart_explorer_updater",
        "se"
    )) {
        $hashes[$payload] = Assert-PublicationHashSidecar $feed $payload
    }

    $manifestPath = Join-Path $feed "windows-build.manifest"
    Assert-PublicationNonEmptyFile $manifestPath
    $manifest = @{}
    foreach ($line in Get-Content -LiteralPath $manifestPath) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        $parts = $line.Split('=', 2)
        if ($parts.Count -ne 2 -or [string]::IsNullOrWhiteSpace($parts[0])) {
            throw "Malformed Windows build manifest line: $line"
        }
        $key = $parts[0].Trim()
        if ($manifest.ContainsKey($key)) {
            throw "Duplicate Windows build manifest entry: $key"
        }
        $manifest[$key] = $parts[1].Trim()
    }
    $expectedManifestKeys = @("version", "smart_explorer.exe", "smart_explorer_updater.exe", "se.exe")
    if ($manifest.Count -ne $expectedManifestKeys.Count) {
        throw "Windows build manifest must contain exactly version and three Windows payload hashes."
    }
    foreach ($key in $expectedManifestKeys) {
        if (-not $manifest.ContainsKey($key)) {
            throw "Windows build manifest is missing '$key'."
        }
    }
    if ($manifest["version"] -ne $Version) {
        throw "Windows build manifest version '$($manifest["version"])' does not match '$Version'."
    }
    foreach ($payload in @("smart_explorer.exe", "smart_explorer_updater.exe", "se.exe")) {
        if ($manifest[$payload].ToLowerInvariant() -ne $hashes[$payload]) {
            throw "Windows build manifest does not bind the exact '$payload' bytes."
        }
    }

    $portablePairs = @(
        @("Smart Explorer.exe", "smart_explorer.exe"),
        @("Smart Explorer Updater.exe", "smart_explorer_updater.exe"),
        @("se.exe", "se.exe")
    )
    foreach ($pair in $portablePairs) {
        $portable = Join-Path $releaseRoot $pair[0]
        $payload = Join-Path $feed $pair[1]
        if ((Get-PublicationSha256 $portable) -ne (Get-PublicationSha256 $payload)) {
            throw "Portable release file '$($pair[0])' differs from feed payload '$($pair[1])'."
        }
    }
    foreach ($agent in @(
        "native/agent-bin/se-agent-x86_64-linux-musl",
        "native/agent-bin/se-agent-aarch64-linux-musl"
    )) {
        Assert-PublicationNonEmptyFile (Join-Path $RepoRoot $agent)
    }

    $map = @(Get-PublicationReleaseAssetMap -RepoRoot $RepoRoot -Version $Version)
    $publishedNames = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::Ordinal
    )
    $localPaths = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    foreach ($asset in $map) {
        Assert-PublicationNonEmptyFile $asset.LocalPath
        $resolved = (Resolve-Path -LiteralPath $asset.LocalPath).Path
        if (-not $publishedNames.Add($asset.PublishedName) -or -not $localPaths.Add($resolved)) {
            throw "Release publication map contains a duplicate: $($asset.PublishedName)"
        }
    }

    return [pscustomobject]@{
        Version = $Version
        Assets = $map
        FeedHashes = $hashes
        InstallerPublishedName = "Smart.Explorer.Setup.$Version.exe"
    }
}

function Test-ReleasePublicationCandidate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    try {
        return Assert-ReleasePublicationCandidate -RepoRoot $RepoRoot -Version $Version
    } catch {
        Write-Verbose (ConvertTo-ReleasePublicationSafeText $_.Exception.Message)
        return $false
    }
}

function Get-PublicationRepositorySlug {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $origin = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("remote", "get-url", "origin")).StdOut.Trim()
    if ([string]::IsNullOrWhiteSpace($origin)) {
        throw "Git origin has no URL."
    }
    $match = [regex]::Match(
        $origin,
        '^(?:(?:https?|git|ssh)://)?(?:[^/@]+@)?github\.com(?::|/)(?<owner>[0-9A-Za-z_.-]+)/(?<repo>[0-9A-Za-z_.-]+?)(?:\.git)?/?$'
    )
    if (-not $match.Success) {
        throw "Git origin must identify a github.com owner/repository pair."
    }
    $slug = "$($match.Groups['owner'].Value)/$($match.Groups['repo'].Value)"
    if ($slug -notmatch '^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$') {
        throw "Git origin produced an invalid GitHub repository slug."
    }
    return $slug
}

function Get-PublicationReleaseCommitPaths {
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    return @(
        "native/Cargo.toml",
        "native/Cargo.lock",
        "native/agent-bin/se-agent-x86_64-linux-musl",
        "native/agent-bin/se-agent-aarch64-linux-musl",
        "release-native/Smart Explorer Setup $Version.exe",
        "release-native/Smart Explorer.exe",
        "release-native/Smart Explorer Updater.exe",
        "release-native/se.exe",
        "release-native/smart_explorer_command.dll",
        "release-native/share-server/se-share-server.exe",
        "release-native/share-server/se-share-server-linux",
        "release-native/update-feed/smart_explorer.exe",
        "release-native/update-feed/smart_explorer_updater.exe",
        "release-native/update-feed/se.exe",
        "release-native/update-feed/smart_explorer.exe.sha256",
        "release-native/update-feed/smart_explorer_updater.exe.sha256",
        "release-native/update-feed/se.exe.sha256",
        "release-native/update-feed/smart_explorer",
        "release-native/update-feed/smart_explorer_updater",
        "release-native/update-feed/se",
        "release-native/update-feed/smart_explorer.sha256",
        "release-native/update-feed/smart_explorer_updater.sha256",
        "release-native/update-feed/se.sha256",
        "release-native/update-feed/windows-build.manifest",
        "release-native/update-feed/version.txt"
    )
}

function Test-PublicationLocalReleaseStatePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $normalized = $Path.Replace('\', '/').Trim('"')
    return $normalized -eq "release-native/.complete-release.lock" -or
        $normalized.StartsWith("release-native/.release-stage.") -or
        $normalized.StartsWith("release-native/.complete-release-stage.") -or
        $normalized.StartsWith("release-native/.update-feed.linux-candidate.")
}

function Invoke-ReleasePublicationCommit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version
    )

    $candidate = Test-ReleasePublicationCandidate -RepoRoot $RepoRoot -Version $Version
    if (-not $candidate) {
        throw "Complete v$Version release candidate validation failed before commit."
    }
    $branch = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("symbolic-ref", "--quiet", "--short", "HEAD")).StdOut.Trim()
    if ($branch -ne "main") {
        throw "A complete release candidate must be committed on local main, not '$branch'."
    }

    $allowed = @(Get-PublicationReleaseCommitPaths -Version $Version)
    $index = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("diff", "--cached", "--quiet") `
        -AllowFailure
    if ($index.ExitCode -eq 1) {
        # A terminated wrapper may have staged this exact bounded set just
        # before committing. Unstage only those paths, then prove that no
        # unrelated index entry remains before rebuilding the exact stage.
        $null = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments (@("reset", "--quiet", "HEAD", "--") + $allowed)
        $index = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("diff", "--cached", "--quiet") `
            -AllowFailure
        if ($index.ExitCode -eq 1) {
            throw "The Git index contains changes outside the explicit release publication set."
        }
    }
    if ($index.ExitCode -ne 0) {
        throw "Could not verify that the Git index is clean: $($index.Output)"
    }

    $stageArguments = @("add", "--") + $allowed
    $null = Invoke-ReleasePublicationGit -RepoRoot $RepoRoot -Arguments $stageArguments

    $releaseStatus = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @(
            "status", "--porcelain=v1", "--untracked-files=all", "--",
            "native/Cargo.toml", "native/Cargo.lock", "native/agent-bin", "release-native"
        )).StdOut
    $unexpected = @()
    foreach ($line in @($releaseStatus -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line) -or $line.Length -lt 2) {
            continue
        }
        $statusPath = if ($line.Length -gt 3) { $line.Substring(3) } else { "" }
        if ($line.Substring(0, 2) -eq "??" -and
            (Test-PublicationLocalReleaseStatePath -Path $statusPath)) {
            continue
        }
        if ($line.Substring(0, 2) -eq "??" -or $line[1] -ne ' ') {
            $unexpected += $line
        }
    }
    if ($unexpected.Count -gt 0) {
        $null = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments (@("reset", "--quiet", "HEAD", "--") + $allowed) `
            -AllowFailure
        throw "Unexpected tracked or untracked release changes remain outside the explicit publication set: $($unexpected -join '; ')"
    }

    $staged = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("diff", "--cached", "--quiet") `
        -AllowFailure
    $subject = "Release Smart Explorer v$Version [release candidate]"
    if ($staged.ExitCode -eq 0) {
        $headSubject = (Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("show", "-s", "--format=%s", "HEAD")).StdOut.Trim()
        if ($headSubject -ne $subject) {
            throw "No release changes are staged, and HEAD is not the expected release candidate commit."
        }
        $existing = (Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("rev-parse", "HEAD")).StdOut.Trim()
        return [pscustomobject]@{
            Sha = $existing
            CandidateSha = $existing
            Created = $false
            Subject = $subject
        }
    }
    if ($staged.ExitCode -ne 1) {
        throw "Could not inspect staged release changes: $($staged.Output)"
    }

    $null = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("commit", "-m", $subject, "--")
    $candidateSha = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("rev-parse", "HEAD")).StdOut.Trim()
    return [pscustomobject]@{
        Sha = $candidateSha
        CandidateSha = $candidateSha
        Created = $true
        Subject = $subject
    }
}

function Invoke-ReleasePublicationMainPush {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-fA-F]{40,64}$')]
        [string]$CandidateSha
    )

    $candidateSha = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("rev-parse", "$CandidateSha^{commit}")).StdOut.Trim()
    $head = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("rev-parse", "HEAD")).StdOut.Trim()
    if ($head -ne $candidateSha) {
        throw "Release candidate '$candidateSha' is not local HEAD '$head'."
    }
    $branch = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("symbolic-ref", "--quiet", "--short", "HEAD")).StdOut.Trim()
    if ($branch -ne "main") {
        throw "Release candidate main push requires local main, not '$branch'."
    }

    $null = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("fetch", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main")
    $ancestor = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("merge-base", "--is-ancestor", "refs/remotes/origin/main", $candidateSha) `
        -AllowFailure
    if ($ancestor.ExitCode -eq 1) {
        throw "origin/main is not an ancestor of the exact release candidate; refusing a non-fast-forward push."
    }
    if ($ancestor.ExitCode -ne 0) {
        throw "Could not verify the main fast-forward boundary: $($ancestor.Output)"
    }

    $originMain = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("rev-parse", "refs/remotes/origin/main")).StdOut.Trim()
    if ($originMain -eq $candidateSha) {
        return [pscustomobject]@{ CandidateSha = $candidateSha; Branch = "main"; Pushed = $false }
    }

    $null = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("push", "--porcelain", "origin", "HEAD:refs/heads/main")
    $remote = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("ls-remote", "origin", "refs/heads/main")).StdOut.Trim()
    $remoteSha = ($remote -split '\s+')[0]
    if ($remoteSha -ne $candidateSha) {
        throw "origin/main is '$remoteSha' after push, expected exact candidate '$candidateSha'."
    }
    return [pscustomobject]@{ CandidateSha = $candidateSha; Branch = "main"; Pushed = $true }
}

function Get-PublicationRemoteTagCommit {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Tag
    )

    $tagRef = "refs/tags/$Tag"
    $lines = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("ls-remote", "origin", $tagRef, "$tagRef^{}" )).StdOut -split "`r?`n"
    $direct = $null
    $peeled = $null
    foreach ($line in $lines) {
        if ($line -match '^([0-9a-fA-F]{40,64})\s+(.+)$') {
            if ($Matches[2] -eq "$tagRef^{}") {
                $peeled = $Matches[1].ToLowerInvariant()
            } elseif ($Matches[2] -eq $tagRef) {
                $direct = $Matches[1].ToLowerInvariant()
            }
        }
    }
    if ($peeled) {
        return $peeled
    }
    return $direct
}

function Invoke-ReleasePublicationTagPush {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-fA-F]{40,64}$')]
        [string]$CandidateSha
    )

    $candidateSha = (Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("rev-parse", "$CandidateSha^{commit}")).StdOut.Trim().ToLowerInvariant()
    $tag = "v$Version"
    $tagRef = "refs/tags/$tag"
    $remoteBefore = Get-PublicationRemoteTagCommit -RepoRoot $RepoRoot -Tag $tag
    if ($remoteBefore -and $remoteBefore -ne $candidateSha) {
        throw "Immutable remote tag '$tag' already points to '$remoteBefore', not '$candidateSha'."
    }

    $localExists = Invoke-ReleasePublicationGit `
        -RepoRoot $RepoRoot `
        -Arguments @("show-ref", "--verify", "--quiet", $tagRef) `
        -AllowFailure
    if ($localExists.ExitCode -notin @(0, 1)) {
        throw "Could not inspect local tag '$tag': $($localExists.Output)"
    }
    if ($localExists.ExitCode -eq 0) {
        $localCommit = (Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("rev-list", "-n", "1", $tagRef)).StdOut.Trim().ToLowerInvariant()
        if ($localCommit -ne $candidateSha) {
            throw "Immutable local tag '$tag' already points to '$localCommit', not '$candidateSha'."
        }
    } elseif ($remoteBefore) {
        $null = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("fetch", "--no-tags", "origin", "$tagRef`:$tagRef")
    } else {
        $null = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("update-ref", $tagRef, $candidateSha, ("0" * 40))
    }

    $pushed = $false
    if (-not $remoteBefore) {
        $push = Invoke-ReleasePublicationGit `
            -RepoRoot $RepoRoot `
            -Arguments @("push", "--porcelain", "origin", "$tagRef`:$tagRef") `
            -AllowFailure
        if ($push.ExitCode -ne 0) {
            $raced = Get-PublicationRemoteTagCommit -RepoRoot $RepoRoot -Tag $tag
            if ($raced -ne $candidateSha) {
                throw "Could not create immutable remote tag '$tag': $($push.Output)"
            }
        } else {
            $pushed = $true
        }
    }
    $remoteAfter = Get-PublicationRemoteTagCommit -RepoRoot $RepoRoot -Tag $tag
    if ($remoteAfter -ne $candidateSha) {
        throw "Remote tag '$tag' is '$remoteAfter' after publication trigger, expected '$candidateSha'."
    }
    return [pscustomobject]@{
        Tag = $tag
        CandidateSha = $candidateSha
        Created = -not [bool]$remoteBefore
        Pushed = $pushed
    }
}

function Invoke-ReleasePublicationGitHubGet {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$')]
        [string]$RepositorySlug,
        [Parameter(Mandatory = $true)][string]$ApiPath,
        [switch]$AllowNotFound
    )

    if (-not $ApiPath.StartsWith('/') -or $ApiPath.StartsWith('//')) {
        throw "GitHub API path must be a repository-relative absolute path."
    }
    $headers = @{
        Accept = "application/vnd.github+json"
        "X-GitHub-Api-Version" = "2026-03-10"
        "User-Agent" = "smart-explorer-release-publication"
    }
    $token = [Environment]::GetEnvironmentVariable("GH_TOKEN", "Process")
    if ([string]::IsNullOrWhiteSpace($token)) {
        $token = [Environment]::GetEnvironmentVariable("GITHUB_TOKEN", "Process")
    }
    if (-not [string]::IsNullOrWhiteSpace($token)) {
        $headers["Authorization"] = "Bearer $token"
    }
    $uri = "https://api.github.com/repos/$RepositorySlug$ApiPath"
    try {
        return Invoke-RestMethod -Method Get -Uri $uri -Headers $headers -ErrorAction Stop
    } catch {
        $status = $null
        if ($_.Exception.Response -and $_.Exception.Response.StatusCode) {
            $status = [int]$_.Exception.Response.StatusCode
        }
        if ($AllowNotFound -and $status -eq 404) {
            return $null
        }
        $detail = ConvertTo-ReleasePublicationSafeText $_.Exception.Message
        if ($status) {
            throw "GitHub public API GET failed with HTTP $status for '$ApiPath': $detail"
        }
        throw "GitHub public API GET failed for '$ApiPath': $detail"
    }
}

function Wait-ReleasePublicationDelay {
    param([Parameter(Mandatory = $true)][datetimeoffset]$Deadline)

    $remaining = $Deadline - [datetimeoffset]::UtcNow
    if ($remaining.TotalSeconds -le 0) {
        return $false
    }
    $delay = [Math]::Min(30, [Math]::Max(1, [Math]::Floor($remaining.TotalSeconds)))
    Start-Sleep -Seconds $delay
    return $true
}

function Wait-ReleasePublicationWorkflow {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$')]
        [string]$RepositorySlug,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-fA-F]{40,64}$')]
        [string]$CandidateSha,
        [Parameter(Mandatory = $true)][datetimeoffset]$Deadline
    )

    $tag = "v$Version"
    $candidateSha = $CandidateSha.ToLowerInvariant()
    $encodedTag = [uri]::EscapeDataString($tag)
    $encodedSha = [uri]::EscapeDataString($candidateSha)
    $path = "/actions/workflows/build.yml/runs?event=push&branch=$encodedTag&head_sha=$encodedSha&per_page=100"
    $lastState = "not visible"
    while ([datetimeoffset]::UtcNow -lt $Deadline) {
        $response = Invoke-ReleasePublicationGitHubGet `
            -RepositorySlug $RepositorySlug `
            -ApiPath $path
        $matches = @($response.workflow_runs | Where-Object {
            $_.event -eq "push" -and
            $_.head_sha.ToLowerInvariant() -eq $candidateSha -and
            $_.head_branch -eq $tag -and
            $_.path -eq ".github/workflows/build.yml"
        })
        if ($matches.Count -gt 1) {
            throw "More than one exact build.yml tag-push run exists for '$tag' at '$candidateSha'."
        }
        if ($matches.Count -eq 1) {
            $run = $matches[0]
            $lastState = "$($run.status)/$($run.conclusion)"
            if ($run.status -eq "completed") {
                if ($run.conclusion -ne "success") {
                    throw "Exact build.yml tag run $($run.id) completed '$($run.conclusion)': $($run.html_url)"
                }
                return [pscustomobject]@{
                    RunId = [long]$run.id
                    RunAttempt = [int]$run.run_attempt
                    Tag = $tag
                    CandidateSha = $candidateSha
                    Url = [string]$run.html_url
                    Conclusion = "success"
                }
            }
        }
        if (-not (Wait-ReleasePublicationDelay -Deadline $Deadline)) {
            break
        }
    }
    throw "Timed out waiting for exact build.yml push run for '$tag' at '$candidateSha' (last state: $lastState)."
}

function Wait-ReleasePublicationAssets {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$')]
        [string]$RepositorySlug,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version,
        [Parameter(Mandatory = $true)][datetimeoffset]$Deadline
    )

    $candidate = Test-ReleasePublicationCandidate -RepoRoot $RepoRoot -Version $Version
    if (-not $candidate) {
        throw "Complete v$Version release candidate validation failed before GitHub Release verification."
    }
    $expected = @{}
    foreach ($asset in $candidate.Assets) {
        $item = Get-Item -LiteralPath $asset.LocalPath
        $expected[$asset.PublishedName] = [pscustomobject]@{
            Name = $asset.PublishedName
            LocalPath = $asset.LocalPath
            Size = [long]$item.Length
            Sha256 = Get-PublicationSha256 $asset.LocalPath
        }
    }
    if ($expected.Count -ne 18) {
        throw "Expected release publication set must contain exactly 18 assets."
    }

    $tag = "v$Version"
    $apiTag = [uri]::EscapeDataString($tag)
    $lastState = "release not visible"
    while ([datetimeoffset]::UtcNow -lt $Deadline) {
        $release = Invoke-ReleasePublicationGitHubGet `
            -RepositorySlug $RepositorySlug `
            -ApiPath "/releases/tags/$apiTag" `
            -AllowNotFound
        if ($release) {
            if ($release.tag_name -ne $tag -or $release.draft) {
                throw "Visible GitHub Release does not represent published tag '$tag'."
            }
            $remoteAssets = @($release.assets)
            $remote = @{}
            foreach ($asset in $remoteAssets) {
                if ($remote.ContainsKey([string]$asset.name)) {
                    throw "GitHub Release '$tag' contains duplicate asset '$($asset.name)'."
                }
                $remote[[string]$asset.name] = $asset
            }
            $unknown = @($remote.Keys | Where-Object { -not $expected.ContainsKey($_) })
            if ($unknown.Count -gt 0) {
                throw "GitHub Release '$tag' contains unexpected asset(s): $($unknown -join ', ')"
            }
            if ($remote.Count -gt 18) {
                throw "GitHub Release '$tag' contains $($remote.Count) assets, expected exactly 18."
            }
            $missing = @($expected.Keys | Where-Object { -not $remote.ContainsKey($_) })
            if ($missing.Count -eq 0 -and $remote.Count -eq 18) {
                $digestPending = @()
                foreach ($name in $expected.Keys) {
                    $want = $expected[$name]
                    $got = $remote[$name]
                    if ([long]$got.size -ne $want.Size) {
                        throw "GitHub Release asset '$name' has size $($got.size), expected $($want.Size)."
                    }
                    if ([string]::IsNullOrWhiteSpace([string]$got.digest)) {
                        $digestPending += $name
                        continue
                    }
                    $wantDigest = "sha256:$($want.Sha256)"
                    if ([string]$got.digest -ne $wantDigest) {
                        throw "GitHub Release asset '$name' digest '$($got.digest)' does not match '$wantDigest'."
                    }
                }
                if ($digestPending.Count -eq 0) {
                    return [pscustomobject]@{
                        Tag = $tag
                        ReleaseId = [long]$release.id
                        Url = [string]$release.html_url
                        AssetCount = 18
                        Verified = $true
                    }
                }
                $lastState = "digest pending for $($digestPending -join ', ')"
            } else {
                $lastState = "$($remote.Count)/18 assets; missing $($missing -join ', ')"
            }
        }
        if (-not (Wait-ReleasePublicationDelay -Deadline $Deadline)) {
            break
        }
    }
    throw "Timed out waiting for exact 18-asset GitHub Release '$tag' (last state: $lastState)."
}

function Invoke-ReleasePublicationLinuxCliUpdate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$')]
        [string]$RepositorySlug,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^\d+\.\d+\.\d+$')]
        [string]$Version,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9a-fA-F]{40,64}$')]
        [string]$CandidateSha
    )

    $isLinuxVariable = Get-Variable -Name IsLinux -ErrorAction SilentlyContinue
    $runningOnLinux = if ($isLinuxVariable) {
        [bool]$isLinuxVariable.Value
    } else {
        [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Unix
    }
    if (-not $runningOnLinux) {
        throw "The exact local se update is supported only on Linux."
    }

    $candidate = Test-ReleasePublicationCandidate -RepoRoot $RepoRoot -Version $Version
    if (-not $candidate) {
        throw "Complete v$Version release candidate validation failed before local se installation."
    }
    $installer = Join-Path $RepoRoot "install-linux.sh"
    Assert-PublicationNonEmptyFile $installer
    $userProfile = [System.Environment]::GetFolderPath(
        [System.Environment+SpecialFolder]::UserProfile
    )
    if ([string]::IsNullOrWhiteSpace($userProfile)) {
        throw "Could not determine the current Linux user profile directory."
    }
    $installDir = [Environment]::GetEnvironmentVariable(
        "SMART_EXPLORER_INSTALL_DIR",
        "Process"
    )
    if ([string]::IsNullOrWhiteSpace($installDir)) {
        $installDir = Join-Path $userProfile ".local/opt/smart-explorer"
    }
    $cli = Join-Path $installDir "se"
    $environment = @{
        SMART_EXPLORER_REPO = $RepositorySlug
        SMART_EXPLORER_REF = $CandidateSha.ToLowerInvariant()
        SMART_EXPLORER_RELEASE_TAG = "v$Version"
        SMART_EXPLORER_REQUIRE_RELEASE_ASSETS = "1"
    }
    $installResult = Invoke-ReleasePublicationProcess `
        -FilePath "/bin/sh" `
        -Arguments @($installer, "--cli-only") `
        -WorkingDirectory $RepoRoot `
        -Environment $environment `
        -TimeoutSeconds 600 `
        -AllowFailure
    if ($installResult.ExitCode -ne 0) {
        throw "Exact Linux se installation failed: $($installResult.Output)"
    }

    Assert-PublicationNonEmptyFile $cli
    $expectedCli = Join-Path $RepoRoot "release-native/update-feed/se"
    $expectedHash = Get-PublicationSha256 $expectedCli
    $installedHash = Get-PublicationSha256 $cli
    if ($installedHash -ne $expectedHash) {
        throw "Installed se SHA-256 '$installedHash' does not match release '$expectedHash'."
    }
    $reported = Invoke-ReleasePublicationProcess `
        -FilePath $cli `
        -Arguments @("--version") `
        -WorkingDirectory $RepoRoot `
        -TimeoutSeconds 30
    if ($reported.StdOut.Trim() -ne "se $Version") {
        throw "Installed se reports '$($reported.StdOut.Trim())', expected 'se $Version'."
    }

    # Status probes the version-bound daemon IPC.  A stale daemon is handed off
    # to this exact installed executable before the status response succeeds.
    $statusResult = Invoke-ReleasePublicationProcess `
        -FilePath $cli `
        -Arguments @("share", "status", "--json") `
        -WorkingDirectory $RepoRoot `
        -TimeoutSeconds 90
    try {
        $status = $statusResult.StdOut | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw "Installed se returned invalid JSON while verifying daemon handoff."
    }
    if ($status.worker.reachable -ne $true) {
        throw "Installed se could not complete the version-bound Share daemon handoff."
    }
    return [pscustomobject]@{
        Version = $Version
        Path = $cli
        Sha256 = $installedHash
        DaemonHandoff = $true
        WorkerRunning = $status.worker.running
        WorkerConnected = $status.worker.connected
    }
}
