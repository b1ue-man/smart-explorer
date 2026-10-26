# Optional explicit administrative installation; never run by the per-user updater.
# .\install-login-service.ps1 -Action Install -Image <release exe> -ExpectedSha256 <approved hash> -Version <version> -PolicyFile <administrative policy>
# .\install-login-service.ps1 -Action Uninstall
[CmdletBinding()]
param(
    [ValidateSet('Install','Uninstall')][string]$Action = 'Install',
    [string]$Image,
    [ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedSha256,
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+$')][string]$Version,
    [string]$PolicyFile,
    [string]$MachineIdentityFile,
    [string]$NetworkFile
)
$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Login service installation requires an elevated administrator.' }
$broker = 'SmartExplorerLoginBroker'
$gateway = 'SmartExplorerLoginGateway'
$marker = 'Smart Explorer optional login service v1'
$root = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Smart Explorer Login'
$config = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'Smart Explorer Login'
$state = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'Smart Explorer Login Gateway'
$publicStatus = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'Smart Explorer Login Status'
$journal = [Collections.Generic.List[object]]::new()
$saved = @{}
$trustedOwners = @('S-1-5-18','S-1-5-32-544','S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
function Save-Path([string]$Path, [bool]$Contents = $false) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $saved.ContainsKey($full)) {
        Assert-NoRedirect $full
        $exists = Test-Path -LiteralPath $full
        $entry = [pscustomobject]@{ Path=$full; Exists=$exists; Directory=$false; Acl=$null; Data=$null; HasData=$false }
        if ($exists) { $entry.Directory = (Get-Item -Force -LiteralPath $full).PSIsContainer; $entry.Acl = Get-Acl -LiteralPath $full }
        $saved[$full] = $entry; $journal.Add($entry)
    }
    $entry = $saved[$full]
    if ($Contents -and $entry.Exists -and -not $entry.HasData) {
        if ($entry.Directory -or (Get-Item -LiteralPath $full).Length -gt 1048576) { throw 'Administrative rollback record exceeds its bound.' }
        $entry.Data = [IO.File]::ReadAllBytes($full); $entry.HasData = $true
    }
}
function Save-TreeAcl([string]$Path) {
    Save-Path $Path
    if (Test-Path -LiteralPath $Path) {
        foreach ($item in Get-ChildItem -Force -LiteralPath $Path) {
            Assert-NoRedirect $item.FullName
            if ($item.PSIsContainer) { Save-TreeAcl $item.FullName } else { Save-Path $item.FullName }
        }
    }
}
function Restore-Paths {
    foreach ($entry in $journal) {
        if ($entry.Path -eq $stage -or $entry.Path.StartsWith($stage + '\',[StringComparison]::OrdinalIgnoreCase)) { continue }
        if ($entry.Exists -and $entry.HasData) {
            $same = (Test-Path -LiteralPath $entry.Path) -and ([Convert]::ToBase64String([IO.File]::ReadAllBytes($entry.Path)) -ceq [Convert]::ToBase64String($entry.Data))
            if (-not $same) { [IO.File]::WriteAllBytes($entry.Path,$entry.Data) }
        }
    }
    # Parent inheritance can change descendants: restore their exact saved ACLs last.
    foreach ($entry in $journal) {
        if ($entry.Path -eq $stage -or $entry.Path.StartsWith($stage + '\',[StringComparison]::OrdinalIgnoreCase)) { continue }
        if ($entry.Exists) {
            $current = Get-Acl -LiteralPath $entry.Path
            if ([Convert]::ToBase64String($current.GetSecurityDescriptorBinaryForm()) -cne [Convert]::ToBase64String($entry.Acl.GetSecurityDescriptorBinaryForm())) { Set-Acl -LiteralPath $entry.Path -AclObject $entry.Acl }
        }
    }
    for ($index=$journal.Count-1; $index -ge 0; $index--) {
        $entry=$journal[$index]
        if ($entry.Path -eq $stage -or $entry.Path.StartsWith($stage + '\',[StringComparison]::OrdinalIgnoreCase)) { continue }
        if (-not $entry.Exists -and (Test-Path -LiteralPath $entry.Path)) { Remove-Item -Force -Recurse -LiteralPath $entry.Path }
    }
}
function Write-RollbackJournal {
    if (-not $stage -or -not (Test-Path -LiteralPath $stage)) { return }
    $sections = [Security.AccessControl.AccessControlSections]::Access -bor [Security.AccessControl.AccessControlSections]::Owner -bor [Security.AccessControl.AccessControlSections]::Group
    $records = foreach ($entry in $journal) {
        if ($entry.Path -eq $stage -or $entry.Path.StartsWith($stage + '\',[StringComparison]::OrdinalIgnoreCase)) { continue }
        [ordered]@{ path=$entry.Path; existed=$entry.Exists; directory=$entry.Directory; has_data=$entry.HasData;
            data=$(if ($entry.HasData) { [Convert]::ToBase64String($entry.Data) } else { $null });
            sddl=$(if ($entry.Acl) { $entry.Acl.GetSecurityDescriptorSddlForm($sections) } else { $null }) }
    }
    $evidence = [ordered]@{ records=@($records); broker_created=$brokerCreated; gateway_created=$gatewayCreated;
        broker=$(if ($oldBroker) { [ordered]@{ path=$oldBroker.PathName; state=$oldBroker.State; account=$oldBroker.StartName; start=$oldBrokerSettings.Start; delayed=$oldBrokerSettings.DelayedAutoStart; sid_type=$oldBrokerSettings.ServiceSidType; dependencies=@($oldBrokerSettings.DependOnService) } } else { $null });
        gateway=$(if ($oldGateway) { [ordered]@{ path=$oldGateway.PathName; state=$oldGateway.State; account=$oldGateway.StartName; start=$oldGatewaySettings.Start; delayed=$oldGatewaySettings.DelayedAutoStart; sid_type=$oldGatewaySettings.ServiceSidType; dependencies=@($oldGatewaySettings.DependOnService) } } else { $null }) }
    [IO.File]::WriteAllText((Join-Path $stage 'rollback.json'),(ConvertTo-Json -InputObject $evidence -Depth 5),[Text.UTF8Encoding]::new($false))
}
function Invoke-Candidate([string]$Executable, [string[]]$Arguments) {
    $encoded = foreach ($argument in $Arguments) { '"' + ([regex]::Replace([regex]::Replace($argument, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1')) + '"' }
    $start = [Diagnostics.ProcessStartInfo]::new(); $start.FileName=$Executable; $start.Arguments=$encoded -join ' '
    $start.UseShellExecute=$false; $start.CreateNoWindow=$true
    $process=[Diagnostics.Process]::Start($start)
    try { if (-not $process.WaitForExit(30000)) { $process.Kill(); $process.WaitForExit(); throw 'Offline login candidate validation timed out.' }
        if ($process.ExitCode -ne 0) { throw 'Offline login candidate validation rejected the executable version, manifest, policy, key or network records.' }
    } finally { $process.Dispose() }
}
function Assert-Ready([string]$Name, [string]$Command, [string]$Executable, [datetime]$StartedAfter) {
    $service=Get-CimInstance Win32_Service -Filter "Name='$Name'"
    if ($service.State -ne 'Running' -or $service.PathName -cne $Command -or -not $service.ProcessId) { throw "Login service has no owned ready instance: $Name" }
    $process=[Diagnostics.Process]::GetProcessById($service.ProcessId)
    try {
        $created=$process.StartTime.ToUniversalTime()
        if ($created -lt $StartedAfter -or $process.MainModule.FileName -ine $Executable -or $process.HasExited) { throw "Login service readiness belongs to a stale or different image: $Name" }
        $again=Get-CimInstance Win32_Service -Filter "Name='$Name'"
        if ($again.State -ne 'Running' -or $again.ProcessId -ne $service.ProcessId -or $process.HasExited) { throw "Login service instance changed during readiness verification: $Name" }
        return "$($service.ProcessId):$($created.ToFileTimeUtc())"
    } finally { $process.Dispose() }
}
function Restore-Service($Previous, [string]$Name, [bool]$Created, $Settings) {
    if ($Created) { if (Get-Service -Name $Name -ErrorAction SilentlyContinue) { Invoke-Sc @('delete',$Name) }; return }
    if (-not $Previous) { return }
    $mode = switch ([int]$Settings.Start) { 2 { if ($Settings.DelayedAutoStart) { 'delayed-auto' } else { 'auto' } }; 3 { 'demand' }; 4 { 'disabled' }; default { throw 'Unsupported original login service start mode.' } }
    Invoke-Sc @('config',$Name,'binPath=',$Previous.PathName,'start=',$mode,'depend=',($Settings.DependOnService -join '/'))
    $sidType = switch ([int]$Settings.ServiceSidType) { 0 {'none'}; 1 {'unrestricted'}; 3 {'restricted'}; default {throw 'Unsupported original service SID type.'} }
    Invoke-Sc @('sidtype',$Name,$sidType)
}
function Invoke-Sc([string[]]$Arguments) {
    # Explicit Win32 argument quoting also preserves quoted ImagePath values
    # under Windows PowerShell 5.1's legacy native argument handling.
    $encoded = foreach ($argument in $Arguments) {
        $escaped = [regex]::Replace($argument, '(\\*)"', '$1$1\"')
        $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
        '"' + $escaped + '"'
    }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = Join-Path ([Environment]::SystemDirectory) 'sc.exe'
    $start.Arguments = $encoded -join ' '
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Unable to start the Service Control Manager client.' }
        $outputTask = $process.StandardOutput.ReadToEndAsync()
        $errorTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) { $process.Kill(); $process.WaitForExit(); throw 'Service Control Manager client timed out.' }
        $errorText = $errorTask.GetAwaiter().GetResult()
        $null = $outputTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "Service Control Manager command failed: $($Arguments[0]) $($Arguments[1]); exit $($process.ExitCode); $errorText" }
    } finally { $process.Dispose() }
}
function Assert-NoRedirect([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -Force -LiteralPath $current
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Installation path contains a reparse point: $current" }
        }
        $parent = [IO.Path]::GetDirectoryName($current.TrimEnd('\'))
        if ($parent -eq $current) { break }
        $current = $parent
    }
}
function Set-ProtectedDirectory([string]$Path, [string]$ServiceSid, [bool]$Writable, [bool]$PublicRead) {
    Assert-NoRedirect $Path
    Save-TreeAcl $Path
    if ($Path -ne $stage) { Write-RollbackJournal }
    $existed = Test-Path -LiteralPath $Path
    if ($existed) {
        $owner = (Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($owner -notin $trustedOwners) { throw "Existing installation directory is not administratively owned: $Path" }
    } else { [IO.Directory]::CreateDirectory($Path) | Out-Null }
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    if (-not $existed) { $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')) }
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18','S-1-5-32-544')) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    if ($ServiceSid) {
        $rights = if ($Writable) { 'FullControl' } else { 'ReadAndExecute' }
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($ServiceSid), $rights, 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    if ($PublicRead) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'), 'ReadAndExecute', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}
function Set-ProtectedFile([string]$Path, [string]$ServiceSid, [bool]$PublicRead) {
    Assert-NoRedirect $Path
    Save-Path $Path
    $acl = [Security.AccessControl.FileSecurity]::new()
    $owner = (Get-Acl -LiteralPath $Path).GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($owner -notin $trustedOwners) { $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')) }
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18','S-1-5-32-544')) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid),'FullControl','Allow'))
    }
    if ($ServiceSid) { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($ServiceSid),'ReadAndExecute','Allow')) }
    if ($PublicRead) { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'),'ReadAndExecute','Allow')) }
    Set-Acl -LiteralPath $Path -AclObject $acl
}
function Assert-ManagedService([string]$Name) {
    $existing = Get-CimInstance Win32_Service -Filter "Name='$Name'"
    if ($existing -and ($existing.Description -ne $marker -or -not $existing.PathName.StartsWith('"' + $root + '\releases\', [StringComparison]::OrdinalIgnoreCase))) { throw "Refusing to alter unmanaged service: $Name" }
    return $existing
}
function Stop-Managed([string]$Name) {
    $service = Get-Service -Name $Name -ErrorAction SilentlyContinue
    if ($service -and $service.Status -ne 'Stopped') { Stop-Service -Name $Name; $service.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30)) }
}
$oldBroker = Assert-ManagedService $broker
$oldGateway = Assert-ManagedService $gateway
if ($Action -eq 'Uninstall') {
    $uninstallLock = [Threading.Mutex]::new($false,'Global\SmartExplorerLoginInstall')
    $uninstallLocked = $false
    try {
        try { $uninstallLocked=$uninstallLock.WaitOne(0) } catch [Threading.AbandonedMutexException] { $uninstallLocked=$true }
        if (-not $uninstallLocked) { throw 'An administrative login installation is already running.' }
        $oldBroker = Assert-ManagedService $broker
        $oldGateway = Assert-ManagedService $gateway
        Stop-Managed $gateway
        Stop-Managed $broker
        if ($oldGateway) { Invoke-Sc @('delete',$gateway) }
        if ($oldBroker) { Invoke-Sc @('delete',$broker) }
        Write-Output 'Optional login services removed; protected grants, network configuration, release generations and private machine identity retained.'
    } finally { if ($uninstallLocked) { $uninstallLock.ReleaseMutex() }; $uninstallLock.Dispose() }
    return
}
if (-not $Image -or -not $ExpectedSha256 -or -not $Version -or -not $PolicyFile) { throw 'Install requires Image, ExpectedSha256, Version and PolicyFile.' }
$stage = $null
$changed = $false
$brokerCreated = $false
$gatewayCreated = $false
$oldRecords = @{}
$oldKey = $false
$oldNetwork = $false
$servicesChanged = $false
$retainStage = $false
$oldBrokerSettings = if ($oldBroker) { Get-ItemProperty -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$broker" } else { $null }
$oldGatewaySettings = if ($oldGateway) { Get-ItemProperty -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$gateway" } else { $null }
$lock = [Threading.Mutex]::new($false,'Global\SmartExplorerLoginInstall')
$locked = $false
try {
    try { $locked=$lock.WaitOne(0) } catch [Threading.AbandonedMutexException] { $locked=$true }
    if (-not $locked) { throw 'An administrative login installation is already running.' }
    $oldBroker = Assert-ManagedService $broker
    $oldGateway = Assert-ManagedService $gateway
    if (($oldBroker -and $oldBroker.State -notin @('Running','Stopped')) -or ($oldGateway -and $oldGateway.State -notin @('Running','Stopped'))) { throw 'Existing login services are changing state; no installation changes made.' }
    if (($oldBroker -and $oldBroker.StartName -ne 'LocalSystem') -or ($oldGateway -and $oldGateway.StartName -ne 'NT AUTHORITY\LocalService')) { throw 'Existing login service identities differ from the fixed broker/gateway contract.' }
    $oldBrokerSettings = if ($oldBroker) { Get-ItemProperty -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$broker" } else { $null }
    $oldGatewaySettings = if ($oldGateway) { Get-ItemProperty -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\$gateway" } else { $null }
    $expected = $ExpectedSha256.ToLowerInvariant()
    $generation = "$Version-$($expected.Substring(0,16))"
    $stage = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) ('.SmartExplorerLoginInstall-' + [Guid]::NewGuid().ToString('N'))
    Set-ProtectedDirectory $stage '' $false $false
    $stagedImage = Join-Path $stage 'smart-explorer-login.exe'
    # Deny source writes/deletion while copying, then verify the protected copy.
    $source = [IO.File]::Open([IO.Path]::GetFullPath($Image),[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        $target = [IO.File]::Open($stagedImage,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try {
            $source.CopyTo($target); $target.Flush($true); $target.Position = 0
            $sha = [Security.Cryptography.SHA256]::Create()
            try { $actual = [BitConverter]::ToString($sha.ComputeHash($target)).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
            if ($actual -ne $expected) { throw 'Protected staged executable SHA256 does not match the approved release.' }
        } finally { $target.Dispose() }
    } finally { $source.Dispose() }
    Set-ProtectedFile $stagedImage '' $true
    $policySource = [IO.File]::OpenRead([IO.Path]::GetFullPath($PolicyFile))
    try {
        if ($policySource.Length -gt 1048576) { throw 'Administrative policy exceeds one MiB.' }
        $reader = [IO.StreamReader]::new($policySource,[Text.Encoding]::UTF8)
        try { $policy = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    } finally { $policySource.Dispose() }
    if ($policy.format -ne 1 -or -not $policy.revision -or -not $policy.machine_id -or -not $policy.machine_public_key -or -not $policy.grants) { throw 'Policy requires explicit machine identity, revision and peer grants.' }
    foreach ($grant in $policy.grants) {
        if (-not $grant.peer_id -or -not $grant.public_key -or -not $grant.target_users -or -not $grant.rights.login) { throw 'Every login grant requires an explicit peer, target-user list and login right.' }
        foreach ($user in $grant.target_users) { if ($user -notmatch '^S-1-5-21-[0-9-]+$') { throw 'Every Windows target user must be an explicit account SID.' } }
    }
    $keyPath = Join-Path $state 'machine-key.json'
    $keySource = if ($MachineIdentityFile) { [IO.Path]::GetFullPath($MachineIdentityFile) } else { $keyPath }
    if (-not (Test-Path -LiteralPath $keySource)) { throw 'First login installation requires an explicit newly provisioned MachineIdentity JSON.' }
    Assert-NoRedirect $keySource
    $keyStream = [IO.File]::OpenRead($keySource)
    try {
        if ($keyStream.Length -gt 8192) { throw 'Machine identity exceeds its limit.' }
        $keyReader = [IO.StreamReader]::new($keyStream,[Text.Encoding]::UTF8)
        try { $identity = $keyReader.ReadToEnd() | ConvertFrom-Json } finally { $keyReader.Dispose() }
    } finally { $keyStream.Dispose() }
    $identityFields = ($identity.PSObject.Properties.Name | Sort-Object) -join ','
    if ($identityFields -cne 'format,machine_id,public_key,secret_key' -or $identity.format -ne 1 -or $identity.machine_id -cne $policy.machine_id -or $identity.public_key -cne $policy.machine_public_key -or $identity.secret_key -isnot [string] -or $identity.secret_key -notmatch '^[0-9a-fA-F]{64}$') { throw 'Private machine identity does not match the explicit administrative policy.' }
    $networkPath = Join-Path $state 'gateway-network.json'
    $networkSource = if ($NetworkFile) { [IO.Path]::GetFullPath($NetworkFile) } else { $networkPath }
    if (-not (Test-Path -LiteralPath $networkSource)) { throw 'First installation requires explicit gateway-network.json from the private provision bundle.' }
    Assert-NoRedirect $networkSource
    $networkStream = [IO.File]::Open($networkSource,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        if ($networkStream.Length -gt 32768) { throw 'Gateway network configuration exceeds 32 KiB.' }
        $networkReader = [IO.StreamReader]::new($networkStream,[Text.Encoding]::UTF8)
        try { $network = $networkReader.ReadToEnd() | ConvertFrom-Json } finally { $networkReader.Dispose() }
    } finally { $networkStream.Dispose() }
    $networkFields = ($network.PSObject.Properties.Name | Sort-Object) -join ','
    if ($networkFields -cne 'allow_plaintext_relays,format,relay_pins,relay_urls,use_default_relays' -or $network.format -ne 1 -or
        ($network.format -isnot [int] -and $network.format -isnot [long]) -or
        $network.use_default_relays -isnot [bool] -or $network.allow_plaintext_relays -isnot [bool] -or
        $network.relay_urls -isnot [Array] -or $network.relay_pins -isnot [Array] -or
        $network.relay_urls.Count -gt 16 -or $network.relay_pins.Count -gt 16 -or ($network.use_default_relays -and $network.relay_urls.Count -ne 0)) { throw 'Gateway network configuration has invalid fields or relay limits.' }
    foreach ($url in $network.relay_urls) {
        $uri = $null
        if ($url -isnot [string] -or $url.Length -gt 2048 -or $url.IndexOfAny([char[]]@([char]0,[char]10,[char]13)) -ge 0 -or
            -not [Uri]::TryCreate($url,[UriKind]::Absolute,[ref]$uri)) { throw 'Invalid gateway relay URL.' }
        if (-not $uri.Host -or $uri.UserInfo -or $uri.Query -or $uri.Fragment -or
            ($uri.Scheme -ne 'https' -and -not ($uri.Scheme -eq 'http' -and $network.allow_plaintext_relays))) { throw 'Gateway relay URL needs HTTPS or explicit plaintext permission and cannot contain credentials, a query or a fragment.' }
    }
    foreach ($pin in $network.relay_pins) {
        if ($pin -isnot [Array] -or $pin.Count -ne 32) { throw 'Gateway relay certificate pin must contain exactly 32 bytes.' }
        foreach ($byte in $pin) { if (($byte -isnot [int] -and $byte -isnot [long]) -or $byte -lt 0 -or $byte -gt 255) { throw 'Invalid gateway relay certificate pin byte.' } }
    }
    $manifest = [ordered]@{ format=1; version=$Version; generation=$generation; sha256=$expected }
    $utf8 = [Text.UTF8Encoding]::new($false)
    $networkJson = $network | ConvertTo-Json -Depth 16 -Compress
    if ($utf8.GetByteCount($networkJson) -gt 32768) { throw 'Canonical gateway network configuration exceeds its byte limit.' }
    [IO.File]::WriteAllText((Join-Path $stage 'gateway-network.json'),$networkJson,$utf8)
    [IO.File]::WriteAllText((Join-Path $stage 'machine-key.json'),($identity | ConvertTo-Json -Compress),$utf8)
    [IO.File]::WriteAllText((Join-Path $stage 'policy.json'),($policy | ConvertTo-Json -Depth 16 -Compress),$utf8)
    [IO.File]::WriteAllText((Join-Path $stage 'manifest.json'),($manifest | ConvertTo-Json -Compress),$utf8)
    foreach ($record in @('policy','machine-key','gateway-network','manifest')) { Set-ProtectedFile (Join-Path $stage "$record.json") '' $false }
    $validationArgs=@('--login-install-validate',$Version,$expected,(Join-Path $stage 'policy.json'),(Join-Path $stage 'machine-key.json'),(Join-Path $stage 'gateway-network.json'),(Join-Path $stage 'manifest.json'))
    Invoke-Candidate $stagedImage $validationArgs
    Set-ProtectedDirectory $root '' $false $true
    Set-ProtectedDirectory (Join-Path $root 'releases') '' $false $true
    $final = Join-Path (Join-Path $root 'releases') $generation
    $imagePath = Join-Path $final 'smart-explorer-login.exe'
    if (Test-Path -LiteralPath $final) {
        Assert-NoRedirect $imagePath
        Set-ProtectedDirectory $final '' $false $true
        $owner = (Get-Acl -LiteralPath $imagePath).GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($owner -notin $trustedOwners) { throw 'Existing release executable is not administratively owned.' }
        Set-ProtectedFile $imagePath '' $true
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $imagePath).Hash.ToLowerInvariant() -ne $expected) { throw 'Existing protected generation has different executable bytes.' }
    } else {
        Set-ProtectedDirectory $final '' $false $true
        Save-Path $imagePath
        Copy-Item -LiteralPath $stagedImage -Destination $imagePath
        Set-ProtectedFile $imagePath '' $true
    }
    $brokerCommand = '"' + $imagePath + '" --login-broker-service'
    $gatewayCommand = '"' + $imagePath + '" --login-gateway-service'
    # Register disabled first, then resolve the service SID and install policy.
    if (-not $oldBroker) { $brokerCreated = $true; Write-RollbackJournal; Invoke-Sc @('create',$broker,'binPath=',$brokerCommand,'start=','disabled','obj=','LocalSystem'); Invoke-Sc @('description',$broker,$marker) }
    if (-not $oldGateway) { $gatewayCreated = $true; Write-RollbackJournal; Invoke-Sc @('create',$gateway,'binPath=',$gatewayCommand,'start=','disabled','obj=','NT AUTHORITY\LocalService'); Invoke-Sc @('description',$gateway,$marker) }
    $serviceSid = ([Security.Principal.NTAccount]::new("NT SERVICE\$gateway")).Translate([Security.Principal.SecurityIdentifier]).Value
    $policy | Add-Member -NotePropertyName gateway_identity -NotePropertyValue $serviceSid -Force
    [IO.File]::WriteAllText((Join-Path $stage 'policy.json'),($policy | ConvertTo-Json -Depth 16 -Compress),$utf8)
    Invoke-Candidate $stagedImage $validationArgs
    # Every active record and ACL is journaled before the first service/ACL mutation.
    foreach ($directory in @($config,$state,$publicStatus)) { Save-TreeAcl $directory }
    foreach ($name in @('policy','manifest')) { foreach ($suffix in @('.json','.next.json','.previous.json')) { Save-Path (Join-Path $config "$name$suffix") $true } }
    foreach ($name in @('machine-key','gateway-network')) { foreach ($suffix in @('.json','.next.json','.previous.json')) { Save-Path (Join-Path $state "$name$suffix") $true } }
    Save-Path (Join-Path $publicStatus 'owner.marker') $true
    Write-RollbackJournal
    $servicesChanged = $true
    Stop-Managed $gateway
    Stop-Managed $broker
    Set-ProtectedDirectory $config $serviceSid $false $false
    Set-ProtectedDirectory $state $serviceSid $true $false
    Set-ProtectedDirectory $publicStatus '' $false $true
    $statusMarker=Join-Path $publicStatus 'owner.marker'
    if (-not (Test-Path -LiteralPath $statusMarker)) { [IO.File]::WriteAllBytes($statusMarker,[byte[]]@()) }
    Set-ProtectedFile $statusMarker '' $true
    foreach ($name in @('policy','manifest')) { Copy-Item -LiteralPath (Join-Path $stage "$name.json") -Destination (Join-Path $config "$name.next.json"); Set-ProtectedFile (Join-Path $config "$name.next.json") $serviceSid $false }
    foreach ($name in @('machine-key','gateway-network')) { Copy-Item -LiteralPath (Join-Path $stage "$name.json") -Destination (Join-Path $state "$name.next.json"); Set-ProtectedFile (Join-Path $state "$name.next.json") $serviceSid $false }
    foreach ($name in @('policy','manifest')) {
        $path = Join-Path $config "$name.json"
        $oldRecords[$name] = Test-Path -LiteralPath $path
        $backup = Join-Path $config "$name.previous.json"
        if (Test-Path -LiteralPath $backup) { Remove-Item -Force -LiteralPath $backup }
        if ($oldRecords[$name]) { Copy-Item -LiteralPath $path -Destination $backup; Set-ProtectedFile $backup $serviceSid $false }
    }
    $oldKey = Test-Path -LiteralPath $keyPath
    $keyBackup = Join-Path $state 'machine-key.previous.json'
    if (Test-Path -LiteralPath $keyBackup) { Remove-Item -Force -LiteralPath $keyBackup }
    if ($oldKey) { Copy-Item -LiteralPath $keyPath -Destination $keyBackup; Set-ProtectedFile $keyBackup $serviceSid $false }
    $oldNetwork = Test-Path -LiteralPath $networkPath
    $networkBackup = Join-Path $state 'gateway-network.previous.json'
    if (Test-Path -LiteralPath $networkBackup) { Remove-Item -Force -LiteralPath $networkBackup }
    if ($oldNetwork) { Copy-Item -LiteralPath $networkPath -Destination $networkBackup; Set-ProtectedFile $networkBackup $serviceSid $false }
    $changed = $true
    if ($oldKey) { [IO.File]::Replace((Join-Path $state 'machine-key.next.json'),$keyPath,$null) }
    else { [IO.File]::Move((Join-Path $state 'machine-key.next.json'),$keyPath) }
    if ($oldNetwork) { [IO.File]::Replace((Join-Path $state 'gateway-network.next.json'),$networkPath,$null) }
    else { [IO.File]::Move((Join-Path $state 'gateway-network.next.json'),$networkPath) }
    foreach ($name in @('policy','manifest')) {
        $path = Join-Path $config "$name.json"
        if ($oldRecords[$name]) { [IO.File]::Replace((Join-Path $config "$name.next.json"),$path,$null) }
        else { [IO.File]::Move((Join-Path $config "$name.next.json"),$path) }
    }
    Invoke-Sc @('config',$broker,'binPath=',$brokerCommand,'start=','auto','obj=','LocalSystem')
    Invoke-Sc @('config',$gateway,'binPath=',$gatewayCommand,'start=','auto','obj=','NT AUTHORITY\LocalService','depend=',$broker)
    Invoke-Sc @('sidtype',$gateway,'restricted')
    $startedAfter=[DateTime]::UtcNow
    Start-Service -Name $broker
    (Get-Service $broker).WaitForStatus('Running',[TimeSpan]::FromSeconds(30))
    Start-Service -Name $gateway
    (Get-Service $gateway).WaitForStatus('Running',[TimeSpan]::FromSeconds(30))
    $brokerInstance=Assert-Ready $broker $brokerCommand $imagePath $startedAfter
    $gatewayInstance=Assert-Ready $gateway $gatewayCommand $imagePath $startedAfter
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $imagePath).Hash.ToLowerInvariant() -ne $expected) { throw 'Protected login release changed before installation commit.' }
    if ((Assert-Ready $broker $brokerCommand $imagePath $startedAfter) -cne $brokerInstance -or
        (Assert-Ready $gateway $gatewayCommand $imagePath $startedAfter) -cne $gatewayInstance) { throw 'Owned login service instance changed before installation commit.' }
    Write-Output "Optional login services ready for approved release $Version; current owned SCM instances verified."
 } catch {
    $originalFailure=$_
    try {
        if ($changed) { Stop-Managed $gateway; Stop-Managed $broker }
        Restore-Paths
        if ($servicesChanged -or $brokerCreated) { Restore-Service $oldBroker $broker $brokerCreated $oldBrokerSettings }
        if ($servicesChanged -or $gatewayCreated) { Restore-Service $oldGateway $gateway $gatewayCreated $oldGatewaySettings }
        if ($servicesChanged) {
            if ($oldBroker -and $oldBroker.State -eq 'Running') { Start-Service $broker; (Get-Service $broker).WaitForStatus('Running',[TimeSpan]::FromSeconds(30)) }
            if ($oldGateway -and $oldGateway.State -eq 'Running') { Start-Service $gateway; (Get-Service $gateway).WaitForStatus('Running',[TimeSpan]::FromSeconds(30)) }
        }
    } catch {
        $retainStage=$true
        throw "Login installation failed: $originalFailure. Administrative rollback failed: $_. Protected rollback evidence retained at $stage."
    }
    throw $originalFailure
} finally {
    if (-not $retainStage -and $stage -and (Test-Path -LiteralPath $stage)) { Remove-Item -Recurse -Force -LiteralPath $stage }
    if ($locked) { $lock.ReleaseMutex() }; $lock.Dispose()
}
