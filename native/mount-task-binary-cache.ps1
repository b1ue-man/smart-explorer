#requires -Version 7.2
# Dot-sourced by test-mount-batching-task.ps1. Git execution uses that suite's
# bounded Invoke-TaskProcess. The workflow owns restoring/saving this directory.

function Get-MountTaskBuildFingerprint {
    param([string]$RepositoryRoot)
    $tree = Invoke-TaskProcess (Get-Command git.exe -CommandType Application).Source @(
        '-C', $RepositoryRoot, '-c', 'core.quotePath=true', 'ls-tree', '-r', '--full-tree', 'HEAD'
    ) 60 'binary-cache-inputs'
    if ($tree.Code -ne 0) { throw 'Could not fingerprint the committed mount task build inputs.' }
    $excluded = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($path in @('native/verify-mount-windows.ps1', 'docs/MOUNT_BATCHING.md', 'docs/RELEASING.md')) {
        [void]$excluded.Add($path)
    }
    $canonical = [Text.StringBuilder]::new()
    $included = 0
    foreach ($raw in ($tree.Output -split "`n")) {
        $entry = $raw.TrimEnd([char]13)
        if ($entry.Length -eq 0) { continue }
        $separator = $entry.IndexOf([char]9)
        if ($separator -lt 1 -or $entry.Substring(0, $separator) -notmatch '^[0-7]{6} (blob|commit) ([0-9a-f]{40}|[0-9a-f]{64})$') {
            throw 'Git returned an unexpected tree record for the mount task cache.'
        }
        # Compare the whole tab-delimited path, never a suffix or substring.
        # All other entries, including this helper, the suite, and workflow,
        # retain Git's mode/type/object/path representation and LF termination.
        if ($excluded.Contains($entry.Substring($separator + 1))) { continue }
        [void]$canonical.Append($entry).Append([char]10)
        $included++
    }
    if ($included -eq 0) { throw 'The mount task build-input tree was empty.' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($canonical.ToString()))
        return [Convert]::ToHexString($digest).ToLowerInvariant()
    } finally { $sha.Dispose() }
}

function Assert-MountTaskCacheDirectory {
    param([string]$Directory)
    $ancestors = [Collections.Generic.List[string]]::new()
    $cursor = $Directory
    while (-not [string]::IsNullOrEmpty($cursor)) {
        $ancestors.Add($cursor)
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    $ancestors.Reverse()
    foreach ($ancestor in $ancestors) {
        try { $attributes = [IO.File]::GetAttributes($ancestor) }
        catch [IO.FileNotFoundException] { continue }
        catch [IO.DirectoryNotFoundException] { continue }
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
            ($attributes -band [IO.FileAttributes]::Directory) -eq 0) {
            throw 'The task binary cache crosses a link-like or non-directory path.'
        }
    }
}

function Get-MountTaskCacheLocation {
    param([string]$CacheRoot, [string]$RepositoryRoot)
    $root = [IO.Path]::GetFullPath($CacheRoot).TrimEnd([char[]]'\/')
    $repository = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd([char[]]'\/')
    $volumeRoot = [IO.Path]::GetPathRoot($root).TrimEnd([char[]]'\/')
    if ($root -notmatch '^[A-Za-z]:\\' -or $root.IndexOf(':', 2) -ge 0 -or
        $root.Equals($volumeRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $root.Equals($repository, [StringComparison]::OrdinalIgnoreCase) -or
        $root.StartsWith($repository + '\', [StringComparison]::OrdinalIgnoreCase) -or
        $repository.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'BinaryCacheRoot must be a dedicated local directory outside the checkout and its ancestors.'
    }
    foreach ($broadRoot in @([IO.Path]::GetTempPath(), [Environment]::GetFolderPath('UserProfile'), $env:RUNNER_TEMP)) {
        if (-not [string]::IsNullOrWhiteSpace($broadRoot) -and
            $root.Equals([IO.Path]::GetFullPath($broadRoot).TrimEnd([char[]]'\/'), [StringComparison]::OrdinalIgnoreCase)) {
            throw 'BinaryCacheRoot must be a dedicated subdirectory, not a profile or temporary root.'
        }
    }
    Assert-MountTaskCacheDirectory $root
    return [pscustomobject]@{
        Root = $root
        Executable = [IO.Path]::Combine($root, 'mount-batching-task.exe')
        Provenance = [IO.Path]::Combine($root, 'provenance.json')
    }
}

function Assert-MountTaskCacheFile {
    param([string]$Path)
    $attributes = [IO.File]::GetAttributes($Path)
    if (($attributes -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint)) -ne 0) {
        throw 'A task binary cache file is not a plain file.'
    }
}

function Get-MountTaskCachedBinary {
    param([object]$Cache, [string]$Fingerprint)
    try {
        Assert-MountTaskCacheDirectory $Cache.Root
        Assert-MountTaskCacheFile $Cache.Provenance
        Assert-MountTaskCacheFile $Cache.Executable
        if ([IO.FileInfo]::new($Cache.Provenance).Length -gt 16384) { return $null }
        $provenance = ConvertFrom-Json ([IO.File]::ReadAllText($Cache.Provenance))
        if ($provenance.schema -ne 1 -or $provenance.binary_file -cne 'mount-batching-task.exe' -or
            $provenance.build_inputs_sha256 -cne $Fingerprint -or
            $provenance.binary_sha256 -isnot [string] -or $provenance.binary_sha256 -cnotmatch '^[0-9a-f]{64}$') {
            return $null
        }
        $observed = (Get-FileHash -LiteralPath $Cache.Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($observed -cne $provenance.binary_sha256) { return $null }
        Write-Host "Reusing the retained Windows fixture; build inputs SHA256=$Fingerprint"
        return $Cache.Executable
    } catch {
        # Missing, truncated, malformed, unsafe, or mismatching cache entries
        # never supply an executable. The normal single incremental build runs.
        return $null
    }
}

function Save-MountTaskCachedBinary {
    param([object]$Cache, [string]$Fingerprint, [string]$TestExecutable)
    try {
        Assert-MountTaskCacheDirectory $Cache.Root
        [void][IO.Directory]::CreateDirectory($Cache.Root)
        Assert-MountTaskCacheDirectory $Cache.Root
        foreach ($path in @($Cache.Executable, $Cache.Provenance)) {
            try { Assert-MountTaskCacheFile $path }
            catch [IO.FileNotFoundException] { continue }
            catch [IO.DirectoryNotFoundException] { continue }
        }
        Assert-MountTaskCacheFile $TestExecutable
        $expected = (Get-FileHash -LiteralPath $TestExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        # Only these two exact cache files are overwritten. Write provenance
        # last: an interrupted copy or JSON write becomes a miss on the next run.
        [IO.File]::Copy($TestExecutable, $Cache.Executable, $true)
        $copied = (Get-FileHash -LiteralPath $Cache.Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($copied -cne $expected) { throw 'The retained fixture copy did not match its built executable.' }
        $provenance = [ordered]@{
            schema = 1
            binary_file = 'mount-batching-task.exe'
            build_inputs_sha256 = $Fingerprint
            binary_sha256 = $copied
        }
        [IO.File]::WriteAllText($Cache.Provenance, (ConvertTo-Json -InputObject $provenance), [Text.UTF8Encoding]::new($false))
        Write-Host "Retained the built Windows fixture before execution; SHA256=$copied"
    } catch {
        # Cache availability is optional; failure to retain bytes does not add
        # another build or prevent the already-built candidate from being run.
        Write-Warning 'The built Windows fixture could not be retained in the dedicated binary cache.'
    }
}
