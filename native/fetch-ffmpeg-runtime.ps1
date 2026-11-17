param(
    [string]$OutputDirectory = ""
)

# Fetch the pinned FFmpeg runtime of the Windows viewer (plan 2026-11-17 P8)
# into the ignored build cache: the BtbN LGPL shared archive of
# ffmpeg-runtime.nsh (exact size and SHA-256), and from it the three DLLs and
# the licence, each checked against its own pin. Prints the directory that
# holds avutil-60.dll, swresample-6.dll, avcodec-62.dll and LICENSE.txt.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$manifestPath = Join-Path $scriptRoot "ffmpeg-runtime.nsh"

function Get-ManifestValue([string]$Name) {
    $escaped = [regex]::Escape($Name)
    $found = @(Select-String -LiteralPath $manifestPath -Pattern "^!define\s+$escaped\s+\`"([^\`"]+)\`"\s*$")
    if ($found.Count -ne 1) {
        throw "FFmpeg manifest must define $Name exactly once."
    }
    return $found[0].Matches[0].Groups[1].Value
}

function Get-Pin([string]$Prefix, [string]$Name) {
    $sizeText = Get-ManifestValue "${Prefix}_SIZE"
    $size = 0L
    if (-not [long]::TryParse($sizeText, [ref]$size) -or $size -le 0) {
        throw "Invalid size of $Name in the FFmpeg manifest: $sizeText"
    }
    $sha256 = (Get-ManifestValue "${Prefix}_SHA256").ToLowerInvariant()
    if ($sha256 -notmatch '^[0-9a-f]{64}$') {
        throw "Invalid SHA-256 of $Name in the FFmpeg manifest."
    }
    return [pscustomobject]@{ Name = $Name; Size = $size; Sha256 = $sha256 }
}

function Test-Pin([string]$Path, $Pin) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    if ((Get-Item -LiteralPath $Path).Length -ne $Pin.Size) {
        return $false
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $Pin.Sha256
}

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "FFmpeg manifest missing: $manifestPath"
}
$release = Get-ManifestValue "FFMPEG_RELEASE"
$zipName = Get-ManifestValue "FFMPEG_ZIP_FILENAME"
$url = Get-ManifestValue "FFMPEG_ZIP_URL"
$root = Get-ManifestValue "FFMPEG_ZIP_ROOT"
if ($zipName -notmatch '^[A-Za-z0-9._-]+\.zip$' -or $root -cne $zipName.Substring(0, $zipName.Length - 4)) {
    throw "Unsafe FFmpeg archive name in manifest: $zipName"
}
if (-not $url.StartsWith("https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-") -or
    -not $url.EndsWith("/$zipName")) {
    throw "FFmpeg archive URL is not a pinned BtbN release asset."
}
$zipPin = Get-Pin "FFMPEG_ZIP" $zipName
$pins = @(
    (Get-Pin "FFMPEG_AVUTIL" (Get-ManifestValue "FFMPEG_AVUTIL_DLL")),
    (Get-Pin "FFMPEG_SWRESAMPLE" (Get-ManifestValue "FFMPEG_SWRESAMPLE_DLL")),
    (Get-Pin "FFMPEG_AVCODEC" (Get-ManifestValue "FFMPEG_AVCODEC_DLL")),
    (Get-Pin "FFMPEG_LICENSE" "LICENSE.txt")
)

$cache = Join-Path $scriptRoot "target\installer-dependencies\ffmpeg-$release"
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $cache "files"
}
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $cache, $OutputDirectory -Force | Out-Null

$complete = $true
foreach ($pin in $pins) {
    if (-not (Test-Pin (Join-Path $OutputDirectory $pin.Name) $pin)) {
        $complete = $false
    }
}
if ($complete) {
    Write-Output $OutputDirectory
    return
}

$archive = Join-Path $cache $zipName
if (-not (Test-Pin $archive $zipPin)) {
    $temporary = Join-Path $cache (".{0}.partial.{1}.{2}" -f $zipName, $PID, [guid]::NewGuid().ToString("N"))
    try {
        for ($attempt = 1; $attempt -le 4; $attempt++) {
            try {
                Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
                Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $temporary
                break
            } catch {
                if ($attempt -eq 4) {
                    throw
                }
                Start-Sleep -Seconds (5 * $attempt)
            }
        }
        if (-not (Test-Pin $temporary $zipPin)) {
            throw "Downloaded FFmpeg archive does not match its pinned size and SHA-256."
        }
        Move-Item -LiteralPath $temporary -Destination $archive -Force
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($archive)
try {
    foreach ($pin in $pins) {
        $inner = if ($pin.Name -eq "LICENSE.txt") { "$root/LICENSE.txt" } else { "$root/bin/$($pin.Name)" }
        $entry = $zip.GetEntry($inner)
        if (-not $entry) {
            throw "FFmpeg archive lacks $inner"
        }
        $target = Join-Path $OutputDirectory $pin.Name
        $partial = "$target.partial"
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $partial, $true)
        if (-not (Test-Pin $partial $pin)) {
            Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue
            throw "$($pin.Name) from the FFmpeg archive does not match its pin."
        }
        Move-Item -LiteralPath $partial -Destination $target -Force
    }
} finally {
    $zip.Dispose()
}
Write-Output $OutputDirectory
