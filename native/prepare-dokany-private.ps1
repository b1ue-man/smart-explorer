#requires -Version 7.2
[CmdletBinding()]
param(
    [string]$ArtifactDirectory = (Join-Path $PSScriptRoot 'target/dokany-private'),
    [switch]$VerifyOnly,
    [switch]$RequireApproved,
    [ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedDllSha256
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$recipeDirectory = Join-Path $PSScriptRoot 'dokany-private'
$utf8 = [Text.UTF8Encoding]::new($false, $true)

function Get-BytesHash([byte[]]$Bytes) {
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Get-CanonicalText([string]$Path) {
    $value = $utf8.GetString([IO.File]::ReadAllBytes($Path))
    # BOM is an exact format marker, not linguistic text: a culture-sensitive
    # comparison can ignore U+FEFF and report a prefix on a BOM-free file.
    if ($value.StartsWith([string][char]0xfeff, [StringComparison]::Ordinal)) {
        throw "UTF-8 BOM is not permitted: $Path"
    }
    return $value.Replace("`r`n", "`n")
}

function Assert-DirectPath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        $attributes = [IO.File]::GetAttributes($current)
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Reparse-point build input/output rejected: $current"
        }
        $current = [IO.Path]::GetDirectoryName($current)
    }
}

function Read-BoundedFile([string]$Path, [long]$Maximum) {
    Assert-DirectPath $Path
    $item = Get-Item -LiteralPath $Path
    if ($item.PSIsContainer -or $item.Length -le 0 -or $item.Length -gt $Maximum) {
        throw "Invalid artifact type or size: $Path"
    }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $memory = $null
    try {
        if ($stream.Length -ne $item.Length -or $stream.Length -gt $Maximum) {
            throw "Artifact changed while opening: $Path"
        }
        $memory = [IO.MemoryStream]::new([int]$stream.Length)
        $stream.CopyTo($memory)
        [byte[]]$bytes = $memory.ToArray()
        if ($bytes.LongLength -ne $item.Length) { throw "Artifact changed while reading: $Path" }
        return ,$bytes
    } finally {
        if ($memory) { $memory.Dispose() }
        $stream.Dispose()
    }
}

function Invoke-Tool([string]$Executable, [string[]]$Arguments, [string]$WorkingDirectory,
                     [int]$TimeoutSeconds = 1200, [switch]$AllowFailure) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    # Explicit project properties below select the SDK, toolset and empty user
    # property directory. Do not inherit compiler/linker injection variables.
    foreach ($name in @('CL', '_CL_', 'LINK', '_LINK_', 'INCLUDE', 'LIB', 'LIBPATH',
        'VCTargetsPath', 'VCToolsInstallDir', 'VCINSTALLDIR', 'VSINSTALLDIR',
        'WindowsSdkDir', 'WindowsSDKVersion', 'MSBuildExtensionsPath',
        'MSBuildExtensionsPath32', 'MSBuildExtensionsPath64', 'MSBuildSDKsPath',
        'MSBuildUserExtensionsPath', 'DirectoryBuildPropsPath', 'DirectoryBuildTargetsPath')) {
        [void]$start.Environment.Remove($name)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw "Cannot start $Executable" }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            [void]$process.WaitForExit(10000)
            throw "Tool deadline exceeded: $Executable"
        }
        if (-not $stdout.Wait(10000) -or -not $stderr.Wait(10000)) {
            throw "Tool output pipe did not close: $Executable"
        }
        $result = [pscustomobject]@{ ExitCode = $process.ExitCode; StdOut = $stdout.Result; StdErr = $stderr.Result }
        if ($result.ExitCode -ne 0 -and -not $AllowFailure) {
            throw "Tool failed ($($result.ExitCode)): $Executable`n$($result.StdOut)`n$($result.StdErr)"
        }
        return $result
    } finally { $process.Dispose() }
}

function Get-PeIdentity([byte[]]$Bytes) {
    # PE32+ offsets follow Microsoft's PE/COFF specification. This is a bounded
    # file parser, not LoadLibrary: VerifyOnly never executes the candidate DLL.
    function U16([long]$Offset) {
        if ($Offset -lt 0 -or $Offset + 2 -gt $Bytes.LongLength) { throw 'Truncated PE word' }
        return [BitConverter]::ToUInt16($Bytes, [int]$Offset)
    }
    function U32([long]$Offset) {
        if ($Offset -lt 0 -or $Offset + 4 -gt $Bytes.LongLength) { throw 'Truncated PE dword' }
        return [BitConverter]::ToUInt32($Bytes, [int]$Offset)
    }
    if ((U16 0) -ne 0x5a4d) { throw 'Candidate has no DOS header' }
    [long]$pe = U32 0x3c
    if ((U32 $pe) -ne 0x4550 -or (U16 ($pe + 4)) -ne 0x8664 -or
        ((U16 ($pe + 22)) -band 0x2000) -eq 0) { throw 'Candidate is not an AMD64 DLL' }
    [long]$optional = $pe + 24
    [long]$optionalSize = U16 ($pe + 20)
    [long]$sectionCount = U16 ($pe + 6)
    if ((U16 $optional) -ne 0x20b -or $optionalSize -lt 240 -or
        (U32 ($optional + 108)) -lt 14 -or $sectionCount -lt 1 -or $sectionCount -gt 96) {
        throw 'Invalid PE32+ optional header or sections'
    }
    $sections = @()
    for ($index = 0; $index -lt $sectionCount; $index++) {
        [long]$offset = $optional + $optionalSize + $index * 40
        $sections += [pscustomobject]@{ Rva = [long](U32 ($offset + 12));
            Size = [long](U32 ($offset + 16)); Raw = [long](U32 ($offset + 20)) }
    }
    function Rva-Offset([long]$Rva, [long]$Length = 1) {
        foreach ($section in $sections) {
            if ($Rva -ge $section.Rva -and $Rva + $Length -le $section.Rva + $section.Size) {
                [long]$result = $section.Raw + $Rva - $section.Rva
                if ($result + $Length -gt $Bytes.LongLength) { throw 'PE RVA exceeds file' }
                return $result
            }
        }
        throw 'PE RVA is not backed by a file section'
    }
    function Ascii-Name([long]$Rva) {
        $builder = [Text.StringBuilder]::new()
        for ($index = 0; $index -lt 256; $index++) {
            [byte]$value = $Bytes[(Rva-Offset ($Rva + $index))]
            if ($value -eq 0) { return $builder.ToString() }
            if ($value -lt 33 -or $value -gt 126) { throw 'Invalid PE name' }
            [void]$builder.Append([char]$value)
        }
        throw 'Unterminated PE name'
    }
    [long]$exportRva = U32 ($optional + 112)
    [long]$exportSize = U32 ($optional + 116)
    [long]$export = Rva-Offset $exportRva 40
    [long]$exportCount = U32 ($export + 24)
    if ($exportCount -lt 1 -or $exportCount -gt 256 -or (U32 ($export + 20)) -ne $exportCount) {
        throw 'Invalid PE export count or unnamed exports'
    }
    [long]$nameTable = U32 ($export + 32)
    [long]$ordinalTable = U32 ($export + 36)
    [long]$functionTable = U32 ($export + 28)
    $exports = @()
    for ($index = 0; $index -lt $exportCount; $index++) {
        $exports += Ascii-Name (U32 (Rva-Offset ($nameTable + $index * 4) 4))
        [long]$ordinal = U16 (Rva-Offset ($ordinalTable + $index * 2) 2)
        if ($ordinal -ge $exportCount) { throw 'Invalid export ordinal' }
        [long]$targetRva = U32 (Rva-Offset ($functionTable + $ordinal * 4) 4)
        if ($targetRva -eq 0 -or ($targetRva -ge $exportRva -and $targetRva -lt $exportRva + $exportSize)) {
            throw 'Empty or forwarded DLL export rejected'
        }
        $null = Rva-Offset $targetRva
    }
    [long]$importRva = U32 ($optional + 120)
    [long]$importSize = U32 ($optional + 124)
    if ($importSize -lt 20 -or $importSize -gt 65536) { throw 'Invalid import directory' }
    $imports = @()
    $terminated = $false
    for ($index = 0; ($index + 1) * 20 -le $importSize; $index++) {
        [long]$descriptor = Rva-Offset ($importRva + $index * 20) 20
        [long]$nameRva = U32 ($descriptor + 12)
        if ($nameRva -eq 0) {
            if ((U32 $descriptor) -ne 0 -or (U32 ($descriptor + 16)) -ne 0) { throw 'Malformed import terminator' }
            $terminated = $true
            break
        }
        $imports += (Ascii-Name $nameRva).ToLowerInvariant()
    }
    if (-not $terminated -or $imports.Count -eq 0 -or
        (U32 ($optional + 112 + 13 * 8)) -ne 0 -or
        (U32 ($optional + 116 + 13 * 8)) -ne 0) { throw 'Missing imports or unreviewed delay imports' }
    if (@($imports | Where-Object { $_ -notin $recipe.allowed_imports }).Count -ne 0) {
        throw "Unreviewed DLL dependencies: $($imports -join ', ')"
    }
    if ((($exports | Sort-Object) -join "`n") -cne (($recipe.required_exports | Sort-Object) -join "`n")) {
        throw 'DLL export set does not exactly match the private recipe'
    }
    return [pscustomobject]@{ Machine = 'AMD64'; Imports = @($imports | Sort-Object -Unique); Exports = @($exports | Sort-Object) }
}

function Assert-Record($Record, [byte[]]$Bytes, [string]$Filename) {
    if ($Record.file -cne $Filename -or $Record.size -ne $Bytes.LongLength -or
        $Record.sha256 -cne (Get-BytesHash $Bytes)) { throw "Artifact record mismatch: $Filename" }
}

function Assert-Approved([string]$Directory) {
    $approved = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'assets/dokany-private'))
    $comparison = if ($IsWindows) { [StringComparison]::OrdinalIgnoreCase } else { [StringComparison]::Ordinal }
    if (-not $Directory.Equals($approved, $comparison)) { throw 'Approved input must be native/assets/dokany-private' }
    $git = (Get-Command git -CommandType Application -ErrorAction Stop).Source
    $repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $files = @('native/assets/dokany-private/dokan2.dll', 'native/assets/dokany-private/manifest.json',
        'native/assets/dokany-private/corresponding-source.zip')
    $null = Invoke-Tool $git (@('ls-files', '--error-unmatch', '--') + $files) $repo 30
    $status = Invoke-Tool $git (@('status', '--porcelain=v1', '--untracked-files=all', '--') + $files) $repo 30
    if ($status.StdOut.Trim()) { throw 'Approved private-DLL inputs differ from committed HEAD' }
}

function Verify-Artifact([string]$Directory) {
    $dllPath = Join-Path $Directory 'dokan2.dll'
    $manifestPath = Join-Path $Directory 'manifest.json'
    $sourcePath = Join-Path $Directory 'corresponding-source.zip'
    $manifest = $utf8.GetString((Read-BoundedFile $manifestPath 65536)) | ConvertFrom-Json
    if ($manifest.schema -ne 1 -or $manifest.source_commit -cne $recipe.source_commit -or
        $manifest.source_archive_sha256 -cne $recipe.source_archive_sha256 -or
        $manifest.recipe_sha256 -cne $recipeSha -or $manifest.patch_sha256 -cne $patchSha -or
        $manifest.builder_sha256 -cne $builderSha -or $manifest.library_api -ne 231 -or
        $manifest.driver_protocol -ne 400) { throw 'Private-DLL source/recipe provenance mismatch' }
    $chain = $manifest.toolchain
    if ($chain.platform_toolset -cne 'v143' -or $chain.runtime_library -cne 'MultiThreaded' -or
        ([version]$chain.vs_version).Major -ne 17 -or
        [version]$chain.msvc_version -lt [version]$recipe.minimum_msvc_version -or
        [version]$chain.sdk_version -lt [version]$recipe.minimum_sdk_version) { throw 'Unreviewed private-DLL toolchain' }
    [byte[]]$dll = Read-BoundedFile $dllPath (32MB)
    Assert-Record $manifest.payload $dll 'dokan2.dll'
    $identity = Get-PeIdentity $dll
    if ($manifest.payload.machine -cne $identity.Machine -or
        ($manifest.payload.imports -join "`n") -cne ($identity.Imports -join "`n") -or
        ($manifest.payload.exports -join "`n") -cne ($identity.Exports -join "`n")) { throw 'PE identity record mismatch' }
    if ($ExpectedDllSha256 -and $manifest.payload.sha256 -cne $ExpectedDllSha256.ToLowerInvariant()) {
        throw 'Private DLL does not match the trusted expected SHA-256'
    }
    Assert-Record $manifest.source_package (Read-BoundedFile $sourcePath (32MB)) 'corresponding-source.zip'
    if ($RequireApproved) { Assert-Approved $Directory }
    return [pscustomobject]@{ Directory = $Directory; DllPath = $dllPath; ManifestPath = $manifestPath;
        SourcePackagePath = $sourcePath; DllSha256 = $manifest.payload.sha256; SourceCommit = $recipe.source_commit;
        RecipeSha256 = $recipeSha; Toolchain = $chain }
}

function Expand-PinnedSource([string]$Archive, [string]$Destination) {
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    $prefix = "dokany-$($recipe.source_commit)/"
    [long]$total = 0
    try {
        if ($zip.Entries.Count -gt 4096) { throw 'Source archive entry limit exceeded' }
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName
            if (-not $name.StartsWith($prefix, [StringComparison]::Ordinal) -or $name.Contains('\') -or
                $name.Contains(':') -or $name.Contains([char]0)) { throw 'Unsafe source archive path' }
            $relative = $name.Substring($prefix.Length).TrimEnd('/')
            if (-not $relative) { continue }
            if (@($relative.Split('/') | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' }).Count) {
                throw 'Unsafe source archive component'
            }
            $kind = ($entry.ExternalAttributes -shr 16) -band 0xf000
            if ($kind -notin @(0, 0x4000, 0x8000)) { throw 'Linked/special source archive entry rejected' }
            $total += $entry.Length
            if ($entry.Length -gt 8MB -or $total -gt 64MB) { throw 'Source archive expansion limit exceeded' }
            $target = Join-Path $Destination $relative
            if ($name.EndsWith('/')) { [void][IO.Directory]::CreateDirectory($target); continue }
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
            $entryStream = $entry.Open()
            $targetStream = $null
            try {
                $targetStream = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                $entryStream.CopyTo($targetStream)
                if ($targetStream.Length -ne $entry.Length) { throw 'Short archive extraction' }
            } finally {
                if ($targetStream) { $targetStream.Dispose() }
                $entryStream.Dispose()
            }
        }
    } finally { $zip.Dispose() }
}

$recipeText = Get-CanonicalText (Join-Path $recipeDirectory 'recipe.json')
$recipe = $recipeText | ConvertFrom-Json
$patchText = Get-CanonicalText (Join-Path $recipeDirectory 'batching.patch')
$recipeSha = Get-BytesHash ($utf8.GetBytes($recipeText))
$patchSha = Get-BytesHash ($utf8.GetBytes($patchText))
$builderSha = Get-BytesHash ($utf8.GetBytes((Get-CanonicalText $PSCommandPath)))
if ($recipe.schema -ne 1 -or $recipe.source_commit -cne 'f1d5de68ff459af94e309cfdd171e4b8ca2af4dd' -or
    $recipe.source_archive_sha256 -cne 'f07c0a13ef426234b8707862a52f30f177528d6d34294bb0dde1620f681d266b' -or
    $recipe.source_archive_size -ne 728462 -or $recipe.patch_sha256 -cne $patchSha -or
    $recipe.source_url -cne "https://codeload.github.com/dokan-dev/dokany/zip/$($recipe.source_commit)" -or
    $recipe.platform_toolset -cne 'v143' -or $recipe.platform -cne 'x64' -or
    $recipe.configuration -cne 'Release' -or $recipe.runtime_library -cne 'MultiThreaded' -or
    $recipe.library_api -ne 231 -or $recipe.driver_protocol -ne 400 -or
    $recipe.machine -cne 'AMD64' -or $recipe.dll_name -cne 'dokan2.dll' -or $recipe.visual_studio_major -ne 17) {
    throw 'Unsupported or altered private Dokany recipe'
}
$ArtifactDirectory = [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath($ArtifactDirectory))
if ($RequireApproved -and -not $VerifyOnly) { throw 'RequireApproved is verification-only; never rebuild approved bytes' }
if ($VerifyOnly) { Verify-Artifact $ArtifactDirectory; return }
if (-not $IsWindows -or ($env:GITHUB_ACTIONS -ne 'true' -and $env:SMART_EXPLORER_REMOTE_RUNNER -ne '1')) {
    throw 'Preparation builds are restricted to the configured remote Windows runner; use VerifyOnly elsewhere'
}
if ($ArtifactDirectory -notmatch '^[A-Za-z]:\\.+') { throw 'Preparation output must be a local, non-root Windows path' }
if (Test-Path -LiteralPath $ArtifactDirectory) { Verify-Artifact $ArtifactDirectory; return }

# Select installed VS 2022/v143 and a complete SDK explicitly, not the upstream
# project's possibly absent v142/19041 defaults. Record exact selected versions.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsResult = Invoke-Tool $vswhere @('-latest', '-products', '*', '-version', '[17.0,18.0)',
    '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8') $PSScriptRoot 30
$instances = @($vsResult.StdOut | ConvertFrom-Json)
if ($instances.Count -ne 1) { throw 'A VS 2022 C++ installation is required for private DLL preparation' }
$vs = $instances[0]
$msbuild = Join-Path $vs.installationPath 'MSBuild/Current/Bin/MSBuild.exe'
$toolsRoot = Join-Path $vs.installationPath 'VC/Tools/MSVC'
$msvc = Get-ChildItem -LiteralPath $toolsRoot -Directory | Where-Object {
    $_.Name -match '^14\.\d+\.\d+$' -and [version]$_.Name -ge [version]$recipe.minimum_msvc_version -and
    (Test-Path -LiteralPath (Join-Path $_.FullName 'bin/Hostx64/x64/cl.exe'))
} | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $msvc) { throw 'A complete v143 x64 compiler is required' }
$sdkRoot = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots').KitsRoot10
$sdk = Get-ChildItem -LiteralPath (Join-Path $sdkRoot 'Include') -Directory | Where-Object {
    $_.Name -match '^10\.\d+\.\d+\.\d+$' -and [version]$_.Name -ge [version]$recipe.minimum_sdk_version -and
    (Test-Path -LiteralPath (Join-Path $_.FullName 'um/Windows.h')) -and
    (Test-Path -LiteralPath (Join-Path $_.FullName 'ucrt/stdio.h')) -and
    (Test-Path -LiteralPath (Join-Path $sdkRoot "Lib/$($_.Name)/um/x64/kernel32.lib")) -and
    (Test-Path -LiteralPath (Join-Path $sdkRoot "bin/$($_.Name)/x64/rc.exe"))
} | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $sdk) { throw 'A complete Windows 10+ SDK is required' }
$git = (Get-Command git -CommandType Application -ErrorAction Stop).Source
$parent = [IO.Path]::GetDirectoryName($ArtifactDirectory)
$existingParent = $parent
while (-not (Test-Path -LiteralPath $existingParent)) {
    $existingParent = [IO.Path]::GetDirectoryName($existingParent)
    if (-not $existingParent) { throw 'No existing local parent for preparation output' }
}
Assert-DirectPath $existingParent
[void][IO.Directory]::CreateDirectory($parent)
Assert-DirectPath $parent
$stage = Join-Path $parent ('.dokany-private-stage.' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($stage)
Write-Host "Preparing private Dokany in $stage"
try {
    $source = Join-Path $stage 'source'
    $artifact = Join-Path $stage 'artifact'
    $emptyUser = Join-Path $stage 'empty-user'
    foreach ($directory in @($source, $artifact, $emptyUser)) { [void][IO.Directory]::CreateDirectory($directory) }
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [Net.Http.HttpClient]::new($handler)
    try {
        $client.Timeout = [TimeSpan]::FromSeconds(180)
        $client.MaxResponseContentBufferSize = [long]$recipe.source_archive_size + 1
        [byte[]]$archiveBytes = $client.GetByteArrayAsync([uri]$recipe.source_url).GetAwaiter().GetResult()
    } finally { $client.Dispose(); $handler.Dispose() }
    if ($archiveBytes.LongLength -ne $recipe.source_archive_size -or
        (Get-BytesHash $archiveBytes) -cne $recipe.source_archive_sha256) { throw 'Pinned upstream archive mismatch' }
    $archive = Join-Path $stage 'upstream.zip'
    [IO.File]::WriteAllBytes($archive, $archiveBytes)
    Expand-PinnedSource $archive $source
    $patch = Join-Path $stage 'batching.patch'
    [IO.File]::WriteAllText($patch, $patchText, $utf8)
    # A fresh, separate Git directory prevents discovery of the application's
    # enclosing repository. No upstream scripts/hooks/checkouts are executed.
    $gitDirectory = Join-Path $stage 'patch-git'
    $gitArgs = @("--git-dir=$gitDirectory", "--work-tree=$source",
        '-c', "core.hooksPath=$emptyUser", '-c', "init.templateDir=$emptyUser")
    $null = Invoke-Tool $git ($gitArgs + @('init', '--quiet')) $source 30
    $null = Invoke-Tool $git ($gitArgs + @('apply', '--check', '--whitespace=nowarn', $patch)) $source 30
    $null = Invoke-Tool $git ($gitArgs + @('apply', '--whitespace=nowarn', $patch)) $source 30
    $recipeCopy = Join-Path $source 'smartexplorer-build/dokany-private'
    [void][IO.Directory]::CreateDirectory($recipeCopy)
    foreach ($name in @('recipe.json', 'batching.patch', 'README.md', 'LICENSE.LGPL-3.0.txt', 'LICENSE.GPL-3.0.txt')) {
        [IO.File]::WriteAllText((Join-Path $recipeCopy $name), (Get-CanonicalText (Join-Path $recipeDirectory $name)), $utf8)
    }
    [IO.File]::WriteAllText((Join-Path $source 'smartexplorer-build/prepare-dokany-private.ps1'),
        (Get-CanonicalText $PSCommandPath), $utf8)
    $sourcePackage = Join-Path $artifact 'corresponding-source.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($source, $sourcePackage, [IO.Compression.CompressionLevel]::Optimal, $true)
    $out = Join-Path $stage 'dll-output'
    $objects = Join-Path $stage 'dll-objects'
    $buildArgs = @((Join-Path $source 'dokan/dokan.vcxproj'), '/t:Build', '/m:1', '/nr:false', '/noAutoResponse',
        '/p:Configuration=Release', '/p:Platform=x64', '/p:PlatformToolset=v143',
        '/p:PreferredToolArchitecture=x64',
        "/p:VCToolsVersion=$($msvc.Name)", "/p:WindowsTargetPlatformVersion=$($sdk.Name)",
        '/p:RuntimeLibrary=MultiThreaded', '/p:MultiProcessorCompilation=false',
        '/p:ImportDirectoryBuildProps=false', '/p:ImportDirectoryBuildTargets=false',
        "/p:UserRootDir=$emptyUser\", "/p:SolutionDir=$source\", "/p:OutDir=$out\", "/p:IntDir=$objects\")
    $built = Invoke-Tool $msbuild $buildArgs $source -AllowFailure
    [IO.File]::WriteAllText((Join-Path $stage 'msbuild.stdout.log'), $built.StdOut, $utf8)
    [IO.File]::WriteAllText((Join-Path $stage 'msbuild.stderr.log'), $built.StdErr, $utf8)
    if ($built.ExitCode -ne 0) { throw "Private DLL build failed ($($built.ExitCode)); see retained MSBuild logs" }
    $dllPath = Join-Path $artifact 'dokan2.dll'
    [IO.File]::Copy((Join-Path $out 'dokan2.dll'), $dllPath, $false)
    [byte[]]$dll = Read-BoundedFile $dllPath (32MB)
    $identity = Get-PeIdentity $dll
    [byte[]]$sourceBytes = Read-BoundedFile $sourcePackage (32MB)
    $manifest = [ordered]@{ schema = 1; source_commit = $recipe.source_commit;
        source_archive_sha256 = $recipe.source_archive_sha256; recipe_sha256 = $recipeSha;
        patch_sha256 = $patchSha; builder_sha256 = $builderSha; library_api = 231; driver_protocol = 400;
        toolchain = [ordered]@{ vs_version = $vs.installationVersion; msvc_version = $msvc.Name;
            sdk_version = $sdk.Name; platform_toolset = 'v143'; runtime_library = 'MultiThreaded' };
        payload = [ordered]@{ file = 'dokan2.dll'; size = $dll.LongLength; sha256 = (Get-BytesHash $dll);
            machine = $identity.Machine; imports = $identity.Imports; exports = $identity.Exports };
        source_package = [ordered]@{ file = 'corresponding-source.zip'; size = $sourceBytes.LongLength;
            sha256 = (Get-BytesHash $sourceBytes) } }
    [IO.File]::WriteAllText((Join-Path $artifact 'manifest.json'), (($manifest | ConvertTo-Json -Depth 6) + "`n"), $utf8)
    $null = Verify-Artifact $artifact
    [IO.Directory]::Move($artifact, $ArtifactDirectory)
    Verify-Artifact $ArtifactDirectory
    Write-Host "Private DLL prepared; retained source/build evidence: $stage"
} catch {
    Write-Warning "Private DLL preparation failed; retained exact staging evidence: $stage"
    throw
}
