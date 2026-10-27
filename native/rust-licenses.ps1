#Requires -Version 7.2
# Rust crate license notices (plan C4, INT-LIC). Generates or checks
# third-party/rust-crates/THIRD-PARTY-LICENSES.txt: the license texts of every
# Rust crate linked into a shipped program - the desktop app and `se`
# (native/Cargo.toml), the Android library (its workspace member
# native/android-bridge), the context-menu DLL (native/explorer-command) and
# the share server (share-server).
#
#   pwsh native/rust-licenses.ps1 -Mode Generate [-Out PATH] [-EnsureTool]
#   pwsh native/rust-licenses.ps1 -Mode Check [-Out PATH] [-EnsureTool]
#
# Generate writes PATH (default: the checked-in file). Exit 0: complete;
# exit 2: written but incomplete (every problem is listed); exit 1: failed.
# Check generates into a temporary directory and exits 1 when PATH (default:
# the checked-in file) is missing or differs, or when the generation is
# incomplete. Line endings and the "Generated:" date line are not compared.
# -EnsureTool installs the pinned cargo-about when it is missing or another
# version is on PATH. cargo-about reads each manifest's Cargo.lock (--locked)
# and needs network access only for crate sources not yet downloaded.
#
# New release builds generate and then check this input through
# native/publish-release-local.ps1 or android/build-release-apk.sh. Existing
# release candidates and environment-only wrapper runs remain check-only.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Generate", "Check")]
    [string]$Mode,
    [string]$Out = "",
    [switch]$EnsureTool
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version 3.0
$global:LASTEXITCODE = 0
try {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
} catch {
    Write-Host "Console output encoding unchanged: $($_.Exception.Message)"
}

# Pinned generator: the newest cargo-about release that was at least 14 days
# old on 2026-10-17 (0.9.2, published 2026-08-18). Its binary needs the `cli`
# feature; `--locked` builds it with its own published Cargo.lock.
$CargoAboutVersion = "0.9.2"
$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$ConfigPath = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "third-party/rust-crates/about.toml"))
$CheckedInPath = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "third-party/rust-crates/THIRD-PARTY-LICENSES.txt"))
# Shipped manifests; each one labels its crate list in the generated file.
$Manifests = @("native/Cargo.toml", "native/explorer-command/Cargo.toml", "share-server/Cargo.toml")
# Layout shared with src/licenses/mod.rs and native/licenses_task_checks.py.
$Rule = "=" * 80
$ThinRule = "-" * 80
$CratesHeading = "Crates by manifest (name version license)"
$TextsHeading = "License texts"
$NoticesHeading = "NOTICE files"
$GeneratedPrefix = "Generated: "
$NoticeNamePattern = '^notice(\.(txt|md|markdown|rst))?$'
$CratesIoSources = @("registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/")
$Utf8 = [System.Text.UTF8Encoding]::new($false)

# License texts cargo-about cannot attach by itself: a LicenseRef-* id has no
# canonical text, and attaching the crate's own file would need a checksum
# clarification in about.toml that breaks with every update of the file.
$SupplementalLicenses = @{
    "epaint_default_fonts" = @(
        @{ Id = "LicenseRef-UFL-1.0"; Name = "Ubuntu Font Licence 1.0"; File = "fonts/UFL.txt" }
    )
}
# Notices of assets bundled in a crate that its license expression does not
# name: the Hack and emoji icon fonts of eframe's `default_fonts` feature.
$AdditionalNotices = @{
    "epaint_default_fonts" = @("fonts/Hack-Regular.txt", "fonts/emoji-icon-font-mit-license.txt")
}

function Invoke-NativeCapture([string]$Command, [string[]]$Arguments) {
    # Exit code, stdout and stderr of a native command; stderr output never
    # becomes a terminating error here.
    $stderrPath = [System.IO.Path]::GetTempFileName()
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $stdout = & $Command @Arguments 2> $stderrPath
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    try {
        $stderr = [System.IO.File]::ReadAllText($stderrPath)
    } finally {
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
    return @{ Code = $code; Stdout = (@($stdout) -join "`n"); Stderr = $stderr }
}

function Get-CargoAboutVersion {
    $result = Invoke-NativeCapture "cargo" @("about", "--version")
    if ($result.Code -ne 0) {
        return $null
    }
    $match = [regex]::Match($result.Stdout, '\d+\.\d+\.\d+')
    if ($match.Success) {
        return $match.Value
    }
    return $null
}

function Assert-CargoAbout {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo is not on PATH; the Rust crate license notices need cargo and cargo-about $CargoAboutVersion."
    }
    $installed = Get-CargoAboutVersion
    if ($installed -eq $CargoAboutVersion) {
        return
    }
    $found = if ($installed) { $installed } else { "none" }
    $install = "cargo install cargo-about --locked --version $CargoAboutVersion --features cli --force"
    if (-not $EnsureTool) {
        throw "cargo-about $CargoAboutVersion is required (found: $found). Install it with: $install"
    }
    Write-Host "Installing cargo-about $CargoAboutVersion (found: $found)."
    & cargo install cargo-about --locked --version $CargoAboutVersion --features cli --force | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "'$install' failed with exit code $LASTEXITCODE."
    }
    $installed = Get-CargoAboutVersion
    if ($installed -ne $CargoAboutVersion) {
        throw "cargo-about $CargoAboutVersion is not the version on PATH after the installation (found: $installed)."
    }
}

function Get-ConfiguredTargets {
    $toml = [System.IO.File]::ReadAllText($ConfigPath, $Utf8)
    $block = [regex]::Match($toml, '(?ms)^targets\s*=\s*\[(.*?)\]')
    if (-not $block.Success) {
        throw "$ConfigPath has no targets list."
    }
    $targets = @([regex]::Matches($block.Groups[1].Value, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value })
    if ($targets.Count -eq 0) {
        throw "$ConfigPath lists no targets."
    }
    return ,$targets
}

function Get-VersionKey([string]$Version) {
    # Sort key for SemVer strings: numeric parts compare numerically and a
    # pre-release ("-x") sorts before its release.
    $core = $Version
    $suffix = "~"
    $cut = $Version.IndexOfAny([char[]]@('-', '+'))
    if ($cut -ge 0) {
        $core = $Version.Substring(0, $cut)
        $rest = $Version.Substring($cut)
        $suffix = if ($rest.StartsWith("-")) { "-$rest" } else { "~$rest" }
    }
    $parts = foreach ($part in ($core -split '\.')) {
        $number = 0L
        if ([long]::TryParse($part, [ref]$number)) { $number.ToString("D12") } else { $part }
    }
    return (@($parts) -join ".") + $suffix
}

function Sort-Ordinal($Items, [scriptblock]$Key) {
    # Culture-independent order, identical on Windows and Linux.
    $list = [System.Collections.Generic.List[object]]::new()
    if ($null -ne $Items) {
        foreach ($item in $Items) {
            $list.Add($item)
        }
    }
    $values = $list.ToArray()
    $keys = [string[]]::new($values.Length)
    for ($i = 0; $i -lt $values.Length; $i++) {
        $keys[$i] = [string](& $Key $values[$i])
    }
    [System.Array]::Sort($keys, $values, [System.StringComparer]::Ordinal)
    return ,$values
}

function Get-PathKey([string]$Path) {
    $full = [System.IO.Path]::GetFullPath($Path)
    if ($IsWindows) {
        return $full.ToLowerInvariant()
    }
    return $full
}

function Get-List($Value) {
    if ($null -eq $Value) {
        return ,@()
    }
    return ,@($Value)
}

function Get-NormalizedText([string]$Text) {
    # LF line ends, no byte order mark, no leading blank lines and no trailing
    # blanks; native/licenses_task_checks.py applies the same rule.
    $value = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
    $value = $value.TrimStart([char]0xFEFF).TrimStart([char]10)
    return $value.TrimEnd([char[]]@([char]32, [char]9, [char]10))
}

function Get-WorkspaceManifestKeys([string]$Manifest) {
    # Our own crates: every workspace member of a shipped manifest.
    $result = Invoke-NativeCapture "cargo" @("metadata", "--locked", "--format-version", "1", "--no-deps", "--manifest-path", $Manifest)
    if ($result.Code -ne 0) {
        throw "cargo metadata --no-deps failed for ${Manifest}: $($result.Stderr)"
    }
    $metadata = $result.Stdout | ConvertFrom-Json -AsHashtable
    $keys = @(foreach ($package in (Get-List $metadata["packages"])) { Get-PathKey ([string]$package["manifest_path"]) })
    return ,$keys
}

function Invoke-CargoAbout([string]$Manifest, [string]$JsonPath) {
    # cargo-about refuses stdout under a PowerShell parent; it writes a file.
    $arguments = @(
        "about", "generate",
        "--format", "json",
        "--config", $ConfigPath,
        "--manifest-path", $Manifest,
        "--workspace",
        "--locked",
        "--output-file", $JsonPath
    )
    # Stdout goes to the host so that it never joins the return value.
    & cargo @arguments | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "cargo about generate failed for $Manifest with exit code $LASTEXITCODE (diagnostics above)."
    }
    return ([System.IO.File]::ReadAllText($JsonPath, $Utf8) | ConvertFrom-Json -AsHashtable)
}

function Add-LicenseBlock($Blocks, [string]$Id, [string]$Name, [string]$Text, [string[]]$Users) {
    # One block per distinct license text; crates of several manifests merge.
    $normalized = Get-NormalizedText $Text
    $key = "$Id`0$Name`0$normalized"
    $block = $null
    if (-not $Blocks.TryGetValue($key, [ref]$block)) {
        $block = [pscustomobject]@{
            Id = $Id
            Name = $Name
            Text = $normalized
            Users = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        }
        $Blocks[$key] = $block
    }
    foreach ($user in $Users) {
        [void]$block.Users.Add($user)
    }
}

function Test-CrateHasLicense($Blocks, [string]$CrateKey, [string]$Id) {
    foreach ($block in $Blocks.Values) {
        if ($block.Id -eq $Id -and $block.Users.Contains($CrateKey)) {
            return $true
        }
    }
    return $false
}

function Format-LicenseNotices($Targets, $Sections, $Crates, $Blocks, $Notices, [string]$Date) {
    $builder = [System.Text.StringBuilder]::new()
    function Add-Line([string]$Line = "") {
        [void]$builder.Append($Line).Append("`n")
    }
    $crateKey = { param($crate) "$($crate.Name)`0$(Get-VersionKey $crate.Version)" }
    $userKey = {
        param($user)
        $parts = ([string]$user) -split ' ', 2
        "$($parts[0])`0$(Get-VersionKey $parts[1])"
    }
    foreach ($block in $Blocks.Values) {
        $block | Add-Member -NotePropertyName SortedUsers -NotePropertyValue (Sort-Ordinal $block.Users $userKey) -Force
    }
    $sortedBlocks = Sort-Ordinal $Blocks.Values {
        param($block)
        "$($block.Id)`0$($block.Name)`0$(& $userKey $block.SortedUsers[0])`0$($block.Text)"
    }
    $sortedNotices = Sort-Ordinal $Notices {
        param($notice)
        "$($notice.Crate.Name)`0$(Get-VersionKey $notice.Crate.Version)`0$($notice.File)"
    }
    $others = Sort-Ordinal ($Crates.Values | Where-Object { $CratesIoSources -notcontains $_.Source }) $crateKey

    $title = "Third-party licenses of the Rust crates in Smart Explorer"
    Add-Line $title
    Add-Line ("=" * $title.Length)
    Add-Line
    Add-Line "Generator: cargo-about $CargoAboutVersion via native/rust-licenses.ps1 and third-party/rust-crates/about.toml"
    Add-Line "$GeneratedPrefix$Date"
    Add-Line ("Targets: " + ($Targets -join ", "))
    Add-Line "Programs: desktop app and se (native/Cargo.toml), Android library (its workspace member native/android-bridge), context-menu DLL (native/explorer-command/Cargo.toml), share server (share-server/Cargo.toml)"
    Add-Line ("Crates: {0}; license texts: {1}; NOTICE files: {2}" -f $Crates.Count, $sortedBlocks.Length, $sortedNotices.Length)
    Add-Line "Source code: every crate is published at https://crates.io/crates/<name>/<version> unless it is listed under ""Other sources""."
    Add-Line
    if ($others.Length -gt 0) {
        $heading = "Other sources (name version source)"
        Add-Line $heading
        Add-Line ("-" * $heading.Length)
        foreach ($crate in $others) {
            $source = if ($crate.Source) { $crate.Source } else { "local path" }
            Add-Line "$($crate.Name) $($crate.Version) $source"
        }
        Add-Line
    }
    Add-Line $CratesHeading
    Add-Line ("-" * $CratesHeading.Length)
    foreach ($section in $Sections) {
        Add-Line
        Add-Line "[$($section.Manifest)]"
        foreach ($entry in (Sort-Ordinal $section.Entries $crateKey)) {
            Add-Line "$($entry.Name) $($entry.Version) $($entry.License)"
        }
    }
    Add-Line
    Add-Line $TextsHeading
    Add-Line ("-" * $TextsHeading.Length)
    Add-Line
    $number = 0
    foreach ($block in $sortedBlocks) {
        $number++
        Add-Line $Rule
        Add-Line ("License {0} of {1}: {2} [{3}]" -f $number, $sortedBlocks.Length, $block.Name, $block.Id)
        Add-Line "Used by:"
        foreach ($user in $block.SortedUsers) {
            Add-Line "  $user"
        }
        Add-Line $ThinRule
        Add-Line $block.Text
        Add-Line
    }
    Add-Line $NoticesHeading
    Add-Line ("-" * $NoticesHeading.Length)
    Add-Line
    if ($sortedNotices.Length -eq 0) {
        Add-Line "None of the listed crates ships a NOTICE file."
    }
    $number = 0
    foreach ($notice in $sortedNotices) {
        $number++
        Add-Line $Rule
        Add-Line ("NOTICE {0} of {1}: {2} {3} ({4})" -f $number, $sortedNotices.Length, $notice.Crate.Name, $notice.Crate.Version, $notice.File)
        Add-Line $ThinRule
        Add-Line $notice.Text
        Add-Line
    }
    return $builder.ToString()
}

function New-LicenseNotices([string]$OutPath, [string]$WorkDir) {
    # Runs cargo-about for every manifest, merges the results, writes the file
    # and returns the problems (empty when every crate has its texts).
    $targets = Get-ConfiguredTargets
    $firstParty = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($manifest in $Manifests) {
        foreach ($key in (Get-WorkspaceManifestKeys (Join-Path $RepoRoot $manifest))) {
            [void]$firstParty.Add($key)
        }
    }
    $sections = [System.Collections.Generic.List[object]]::new()
    $crates = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::Ordinal)
    $blocks = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::Ordinal)
    $problems = [System.Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $Manifests.Count; $index++) {
        $manifest = $Manifests[$index]
        $about = Invoke-CargoAbout (Join-Path $RepoRoot $manifest) (Join-Path $WorkDir "cargo-about-$index.json")
        $entries = [System.Collections.Generic.List[object]]::new()
        foreach ($entry in (Get-List $about["crates"])) {
            $package = $entry["package"]
            if ($firstParty.Contains((Get-PathKey ([string]$package["manifest_path"])))) {
                continue
            }
            $name = [string]$package["name"]
            $version = [string]$package["version"]
            $expression = [string]$entry["license"]
            $key = "$name $version"
            $entries.Add([pscustomobject]@{ Name = $name; Version = $version; License = $expression })
            if (-not $crates.ContainsKey($key)) {
                $crates[$key] = [pscustomobject]@{
                    Key = $key
                    Name = $name
                    Version = $version
                    License = $expression
                    Dir = Split-Path -Parent ([string]$package["manifest_path"])
                    Source = [string]$package["source"]
                }
            }
            if ([string]::IsNullOrWhiteSpace($expression) -or $expression -eq "Unknown") {
                $problems.Add("${key}: no license expression and no detectable license file ($manifest)")
            }
        }
        $sections.Add([pscustomobject]@{ Manifest = $manifest; Entries = $entries })
        foreach ($text in (Get-List $about["licenses"])) {
            $users = @(foreach ($use in (Get-List $text["used_by"])) {
                    $package = $use["crate"]
                    if (-not $firstParty.Contains((Get-PathKey ([string]$package["manifest_path"])))) {
                        "$($package["name"]) $($package["version"])"
                    }
                })
            if ($users.Count -gt 0) {
                Add-LicenseBlock $blocks ([string]$text["id"]) ([string]$text["name"]) ([string]$text["text"]) $users
            }
        }
    }
    foreach ($crate in @($crates.Values)) {
        if (-not $SupplementalLicenses.ContainsKey($crate.Name)) {
            continue
        }
        foreach ($supplement in $SupplementalLicenses[$crate.Name]) {
            if (Test-CrateHasLicense $blocks $crate.Key $supplement.Id) {
                continue
            }
            $path = Join-Path $crate.Dir $supplement.File
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                $problems.Add("$($crate.Key): $($supplement.File) for $($supplement.Id) is missing")
                continue
            }
            Add-LicenseBlock $blocks $supplement.Id $supplement.Name ([System.IO.File]::ReadAllText($path, $Utf8)) @($crate.Key)
        }
    }
    $covered = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($block in $blocks.Values) {
        foreach ($user in $block.Users) {
            [void]$covered.Add($user)
        }
    }
    $notices = [System.Collections.Generic.List[object]]::new()
    foreach ($crate in $crates.Values) {
        if (-not $covered.Contains($crate.Key)) {
            $problems.Add("$($crate.Key): no license text for '$($crate.License)'")
        }
        $files = [System.Collections.Generic.List[string]]::new()
        if (Test-Path -LiteralPath $crate.Dir -PathType Container) {
            foreach ($item in @(Get-ChildItem -LiteralPath $crate.Dir -File)) {
                if ($item.Name -match $NoticeNamePattern) {
                    $files.Add($item.Name)
                }
            }
        }
        if ($AdditionalNotices.ContainsKey($crate.Name)) {
            foreach ($relative in $AdditionalNotices[$crate.Name]) {
                $files.Add($relative)
            }
        }
        foreach ($relative in $files) {
            $path = Join-Path $crate.Dir $relative
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                $problems.Add("$($crate.Key): notice file $relative is missing")
                continue
            }
            $notices.Add([pscustomobject]@{
                    Crate = $crate
                    File = $relative
                    Text = Get-NormalizedText ([System.IO.File]::ReadAllText($path, $Utf8))
                })
        }
    }
    $date = [DateTime]::UtcNow.ToString("yyyy-MM-dd", [System.Globalization.CultureInfo]::InvariantCulture)
    $content = Format-LicenseNotices $targets $sections $crates $blocks $notices $date
    $directory = Split-Path -Parent $OutPath
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    [System.IO.File]::WriteAllText($OutPath, $content, $Utf8)
    Write-Host ("Rust crate license notices: {0} crates, {1} license texts, {2} notice files -> {3}" -f $crates.Count, $blocks.Count, $notices.Count, $OutPath)
    return ,$problems
}

function Get-ComparableLines([string]$Text) {
    # Lines without line-end differences and without the date line of the header.
    $lines = $Text.Replace("`r`n", "`n").Replace("`r", "`n") -split "`n"
    $result = [System.Collections.Generic.List[string]]::new()
    $dateSkipped = $false
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if (-not $dateSkipped -and $i -lt 12 -and $lines[$i].StartsWith($GeneratedPrefix)) {
            $dateSkipped = $true
            continue
        }
        $result.Add($lines[$i])
    }
    return ,$result.ToArray()
}

function Test-SameLines([string[]]$Expected, [string[]]$Actual) {
    if ($Expected.Length -ne $Actual.Length) {
        return $false
    }
    for ($i = 0; $i -lt $Expected.Length; $i++) {
        if (-not [string]::Equals($Expected[$i], $Actual[$i], [System.StringComparison]::Ordinal)) {
            return $false
        }
    }
    return $true
}

function Write-Difference([string[]]$Expected, [string[]]$Actual) {
    $limit = [Math]::Min($Expected.Length, $Actual.Length)
    $first = $limit
    for ($i = 0; $i -lt $limit; $i++) {
        if (-not [string]::Equals($Expected[$i], $Actual[$i], [System.StringComparison]::Ordinal)) {
            $first = $i
            break
        }
    }
    Write-Host ("First difference at line {0} (checked-in: {1} lines, generated: {2} lines)." -f ($first + 1), $Expected.Length, $Actual.Length)
    $counts = [System.Collections.Generic.Dictionary[string, int]]::new([System.StringComparer]::Ordinal)
    foreach ($line in $Expected) {
        $count = 0
        [void]$counts.TryGetValue($line, [ref]$count)
        $counts[$line] = $count + 1
    }
    $onlyGenerated = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $Actual) {
        $count = 0
        if ($counts.TryGetValue($line, [ref]$count) -and $count -gt 0) {
            $counts[$line] = $count - 1
        } else {
            $onlyGenerated.Add($line)
        }
    }
    $onlyCheckedIn = [System.Collections.Generic.List[string]]::new()
    foreach ($pair in $counts.GetEnumerator()) {
        for ($k = 0; $k -lt $pair.Value; $k++) {
            $onlyCheckedIn.Add($pair.Key)
        }
    }
    foreach ($report in @(@("Only in the checked-in file", $onlyCheckedIn), @("Only in the generated file", $onlyGenerated))) {
        $lines = $report[1]
        Write-Host ("{0}: {1} lines" -f $report[0], $lines.Count)
        for ($k = 0; $k -lt [Math]::Min(20, $lines.Count); $k++) {
            Write-Host "  $($lines[$k])"
        }
    }
}

$exitCode = 1
$workDir = Join-Path ([System.IO.Path]::GetTempPath()) ("se-rust-licenses-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $workDir | Out-Null
try {
    Assert-CargoAbout
    $target = $CheckedInPath
    if ($Out) {
        $target = if ([System.IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path (Get-Location).Path $Out }
        $target = [System.IO.Path]::GetFullPath($target)
    }
    if ($Mode -eq "Generate") {
        $problems = New-LicenseNotices $target $workDir
        if ($problems.Count -gt 0) {
            Write-Host "Written, but incomplete ($($problems.Count) problems):"
            foreach ($problem in $problems) {
                Write-Host "  $problem"
            }
            $exitCode = 2
        } else {
            $exitCode = 0
        }
    } else {
        $generated = Join-Path $workDir "THIRD-PARTY-LICENSES.txt"
        $problems = New-LicenseNotices $generated $workDir
        $current = $true
        if ($problems.Count -gt 0) {
            Write-Host "The generation is incomplete ($($problems.Count) problems):"
            foreach ($problem in $problems) {
                Write-Host "  $problem"
            }
            $current = $false
        }
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
            Write-Host "Missing: $target (new release builds prepare it with -Mode Generate before -Mode Check)."
            $current = $false
        } else {
            $expected = Get-ComparableLines ([System.IO.File]::ReadAllText($target, $Utf8))
            $actual = Get-ComparableLines ([System.IO.File]::ReadAllText($generated, $Utf8))
            if (-not (Test-SameLines $expected $actual)) {
                Write-Host "Stale: $target differs from a fresh generation."
                Write-Difference $expected $actual
                $current = $false
            }
        }
        if ($current) {
            Write-Host "Rust crate license notices are current: $target"
            $exitCode = 0
        }
    }
} catch {
    Write-Host "rust-licenses.ps1 -Mode $Mode failed: $($_.Exception.Message)"
    $exitCode = 1
} finally {
    Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
}
exit $exitCode
