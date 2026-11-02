#requires -Version 7.2
# Dot-sourced by test-mount-batching-task.ps1. Git execution uses that suite's
# bounded Invoke-TaskProcess. The workflow owns restoring/saving this directory.

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

function Assert-MountTaskCacheFile {
    param([string]$Path)
    $attributes = [IO.File]::GetAttributes($Path)
    if (($attributes -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint)) -ne 0) {
        throw 'A task binary cache file is not a plain file.'
    }
}

