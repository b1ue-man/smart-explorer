#requires -Version 7.2
# Dot-sourced only by the remote mount task. Invoke-TaskProcess is its bounded
# process/log owner; this helper never runs a build or changes the DLL recipe.
function Ensure-MountTaskDokanyToolchain {
    param([string]$LogRoot, [switch]$Install)
    if (-not $IsWindows -or ($env:GITHUB_ACTIONS -ne 'true' -and
        $env:SMART_EXPLORER_REMOTE_RUNNER -ne '1')) {
        throw 'Private-DLL toolchain setup is restricted to the remote Windows runner.'
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $probeArguments = @('-latest', '-products', '*', '-version', '[17.0,18.0)',
        '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-format', 'json', '-utf8')
    $instances = @()
    if ([IO.File]::Exists($vswhere)) {
        $probe = Invoke-TaskProcess $vswhere $probeArguments 30 'vs2022-before'
        if ($probe.Code -ne 0) { throw 'Could not inspect installed Visual Studio instances.' }
        $instances = @($probe.Output | ConvertFrom-Json)
    }
    if ($instances.Count -eq 1) { return }
    if (-not $Install) { throw 'VS 2022 C++ tools are absent; remote task setup needs -InstallRuntime.' }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'The remote runner needs an administrator token for VS Build Tools setup.'
        }
    } finally { $identity.Dispose() }

    # Fixed 17.14.39 bootstrapper from Microsoft's release-history table, checked
    # 2026-09-08. Keep v143/VS17 provenance rather than relaxing it to VS2026.
    $url = 'https://download.visualstudio.microsoft.com/download/pr/fa619120-9c0e-47e6-bfe0-3ee96fb671b2/236367b68ba9a51708263ab10a1c85546cc4a8eca78b365168811d19c4fb2f29/vs_BuildTools.exe'
    $expected = '236367b68ba9a51708263ab10a1c85546cc4a8eca78b365168811d19c4fb2f29'
    $bootstrapper = Join-Path $LogRoot 'vs2022-buildtools.exe'
    Invoke-WebRequest -Uri $url -OutFile $bootstrapper -TimeoutSec 300
    $sha = (Get-FileHash -LiteralPath $bootstrapper -Algorithm SHA256).Hash.ToLowerInvariant()
    $signature = Get-AuthenticodeSignature -LiteralPath $bootstrapper
    if ($sha -cne $expected -or $signature.Status -ne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -cne 'Microsoft Corporation') {
        throw 'Pinned Microsoft Build Tools bootstrapper failed hash/signature verification.'
    }
    $installPath = Join-Path $env:ProgramFiles 'SmartExplorerTask/VS2022'
    # Fail closed on an unexplained pre-existing path. Never modify a runner's
    # other VS instance or use --force/uninstall/cleanup switches.
    if (Test-Path -LiteralPath $installPath) { throw 'Task-owned VS 2022 installation path already exists without usable tools.' }
    $started = [DateTime]::UtcNow
    Write-Host 'Preparing VS 2022/v143 alongside the runner image toolchain.'
    try {
        $setup = Invoke-TaskProcess $bootstrapper @('--quiet', '--wait', '--norestart',
            '--installPath', $installPath,
            '--add', 'Microsoft.VisualStudio.Workload.VCTools',
            '--add', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
            '--add', 'Microsoft.VisualStudio.Component.Windows11SDK.26100') 1800 'vs2022-install'
        if ($setup.Code -notin @(0, 3010)) { throw "VS 2022 Build Tools installation failed: $($setup.Code)" }
    } finally {
        $setupLogs = Join-Path $LogRoot 'vs2022-setup-logs'
        [void][IO.Directory]::CreateDirectory($setupLogs)
        Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Filter 'dd_*.log' -File |
            Where-Object { $_.LastWriteTimeUtc -ge $started -and
                ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0 } |
            ForEach-Object {
                try { Copy-Item -LiteralPath $_.FullName -Destination $setupLogs }
                catch { Write-Warning "Could not retain setup log $($_.Exception.Message)" }
            }
    }
    $probe = Invoke-TaskProcess $vswhere $probeArguments 30 'vs2022-after'
    if ($probe.Code -ne 0 -or @($probe.Output | ConvertFrom-Json).Count -ne 1) {
        throw 'VS 2022 setup returned without the required usable C++ instance.'
    }
    [ordered]@{ bootstrapper_sha256 = $sha; setup_exit_code = $setup.Code;
        instance = ($probe.Output | ConvertFrom-Json); image = $env:ImageOS;
        image_version = $env:ImageVersion } | ConvertTo-Json -Depth 8 |
        Set-Content -LiteralPath (Join-Path $LogRoot 'vs2022-toolchain.json')
}
