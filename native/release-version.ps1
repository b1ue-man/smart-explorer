# Release version planning shared by the complete release wrapper and the
# build.yml Android APK job.
#
# Dot-source native/release-publication.ps1 first, then this file. The caller
# defines $scriptRoot (the native directory) and $repoRoot (the repository
# root) before invoking these helpers; importing the file has no side effects.
# The functions are the wrapper's unchanged Tagged/Bump/Resume plan and exact
# next-patch version writer, so every consumer resolves the same version.

function Get-NativeVersion {
    $cargoToml = Join-Path $scriptRoot "Cargo.toml"
    $match = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $match) {
        throw "Could not read version from $cargoToml"
    }
    return $match.Matches[0].Groups[1].Value
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

function Get-RemoteTagCommit([string]$Tag) {
    return Get-PublicationRemoteTagCommit -RepoRoot $repoRoot -Tag $Tag
}

function Test-GitAncestor([string]$Ancestor, [string]$Descendant) {
    $result = Invoke-GitCaptured -ArgumentList @("merge-base", "--is-ancestor", $Ancestor, $Descendant) -AllowFailure
    if ($result.ExitCode -eq 0) { return $true }
    if ($result.ExitCode -eq 1) { return $false }
    throw "Could not compare Git ancestry: $($result.Output)"
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
