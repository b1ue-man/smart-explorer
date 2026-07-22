param(
    [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$manifestPath = Join-Path $scriptRoot "dokany-runtime.nsh"

function Get-ManifestValue([string]$Name) {
    $escaped = [regex]::Escape($Name)
    $matches = @(Select-String -LiteralPath $manifestPath -Pattern "^!define\s+$escaped\s+\`"([^\`"]+)\`"\s*$")
    if ($matches.Count -ne 1) {
        throw "Dokany manifest must define $Name exactly once."
    }
    return $matches[0].Matches[0].Groups[1].Value
}

function Test-DokanyMsi([string]$Path, [long]$ExpectedSize, [string]$ExpectedSha256) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    if ((Get-Item -LiteralPath $Path).Length -ne $ExpectedSize) {
        return $false
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $ExpectedSha256
}

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Dokany manifest missing: $manifestPath"
}
$version = Get-ManifestValue "DOKANY_VERSION"
$apiVersion = Get-ManifestValue "DOKANY_API_VERSION"
$driverProtocolVersion = Get-ManifestValue "DOKANY_DRIVER_PROTOCOL_VERSION"
$filename = Get-ManifestValue "DOKANY_MSI_FILENAME"
$url = Get-ManifestValue "DOKANY_MSI_URL"
$sizeText = Get-ManifestValue "DOKANY_MSI_SIZE"
$sha256 = (Get-ManifestValue "DOKANY_MSI_SHA256").ToLowerInvariant()

if ($version -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "Invalid Dokany version in manifest: $version"
}
if ($apiVersion -notmatch '^[1-9][0-9]*$') {
    throw "Invalid Dokany API version in manifest: $apiVersion"
}
if ($driverProtocolVersion -notmatch '^[1-9][0-9]*$') {
    throw "Invalid Dokany driver protocol version in manifest: $driverProtocolVersion"
}
if ($filename -notmatch '^[A-Za-z0-9._-]+\.msi$') {
    throw "Unsafe Dokany MSI filename in manifest: $filename"
}
$expectedUrl = "https://github.com/dokan-dev/dokany/releases/download/v$version/$filename"
if ($url -cne $expectedUrl) {
    throw "Dokany MSI URL is not the pinned official release asset."
}
$size = 0L
if (-not [long]::TryParse($sizeText, [ref]$size) -or $size -le 0) {
    throw "Invalid Dokany MSI size in manifest: $sizeText"
}
if ($sha256 -notmatch '^[0-9a-f]{64}$') {
    throw "Invalid Dokany MSI SHA-256 in manifest."
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $scriptRoot "target\installer-dependencies\$version\$filename"
}
$OutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Path $parent -Force | Out-Null

if (Test-DokanyMsi $OutputPath $size $sha256) {
    Write-Output $OutputPath
    return
}

$temporary = Join-Path $parent (".{0}.partial.{1}.{2}" -f $filename, $PID, [guid]::NewGuid().ToString("N"))
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    for ($attempt = 1; $attempt -le 4; $attempt++) {
        try {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $temporary
            break
        } catch {
            if ($attempt -eq 4) {
                throw
            }
            Start-Sleep -Seconds $attempt
        }
    }
    if (-not (Test-DokanyMsi $temporary $size $sha256)) {
        throw "Downloaded Dokany MSI failed pinned size/SHA-256 verification."
    }
    Move-Item -LiteralPath $temporary -Destination $OutputPath -Force
    if (-not (Test-DokanyMsi $OutputPath $size $sha256)) {
        throw "Promoted Dokany MSI failed verification: $OutputPath"
    }
} finally {
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
}
Write-Output $OutputPath
