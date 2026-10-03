# Immutable installation/probe program embedded in the Rust adapter. Input
# variables are quoted values in the encoded command, never a user script file.
$ErrorActionPreference = 'Stop'
$FileSddl = "O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;$Sid)"
$DirSddl = "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;$Sid)"
$ParentSddl = 'O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;0x1200a9;;;BU)'
$TaskSddl = "O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGX;;;$Sid)"
$TaskName = "Smart Explorer LAN-Uplink $Sid"
$LegacyName = 'Smart Explorer LAN-Uplink'
$Helper = Join-Path $Root 'se.exe'
$Manifest = Join-Path $Root 'installation.json'
$Sections = [System.Security.AccessControl.AccessControlSections]'Owner,Access'
function Canonical-Sddl($Sddl, $Directory) {
    if ($Directory) { $security = New-Object System.Security.AccessControl.DirectorySecurity }
    else { $security = New-Object System.Security.AccessControl.FileSecurity }
    $security.SetSecurityDescriptorSddlForm($Sddl)
    return $security.GetSecurityDescriptorSddlForm($Sections)
}
function Assert-Object($Path, $Directory) {
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $item.PSIsContainer -ne $Directory) {
        throw 'Linked or unexpected installation object'
    }
    $acl = Get-Acl -LiteralPath $Path
    $expected = if ($Directory -and $Path -eq (Split-Path $Root)) { $ParentSddl } elseif ($Directory) { $DirSddl } else { $FileSddl }
    if (!$acl.AreAccessRulesProtected -or $acl.GetSecurityDescriptorSddlForm($Sections) -cne (Canonical-Sddl $expected $Directory)) {
        throw 'Installation owner or permissions require repair'
    }
}
function New-PrivateDirectory($Path) {
    if (Test-Path -LiteralPath $Path) { Assert-Object $Path $true; return }
    $acl = New-Object System.Security.AccessControl.DirectorySecurity
    $sddl = if ($Path -eq (Split-Path $Root)) { $ParentSddl } else { $DirSddl }
    $acl.SetSecurityDescriptorSddlForm($sddl)
    [IO.Directory]::CreateDirectory($Path, $acl) | Out-Null
    Assert-Object $Path $true
}
function New-PrivateFile($Path) {
    $acl = New-Object System.Security.AccessControl.FileSecurity
    $acl.SetSecurityDescriptorSddlForm($FileSddl)
    return New-Object IO.FileStream -ArgumentList @($Path, [IO.FileMode]::CreateNew,
        [Security.AccessControl.FileSystemRights]::FullControl, [IO.FileShare]::Read, 4096,
        [IO.FileOptions]::WriteThrough, $acl)
}
function Assert-RegularFile($Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'Linked or unexpected helper file; operation refused'
    }
}
function File-Hash($Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $sha.Dispose() }
    } finally { $stream.Dispose() }
}
function Get-Task($Name) {
    return Get-ScheduledTask -TaskName $Name -TaskPath '\' -ErrorAction SilentlyContinue
}
function Own-LegacyTask {
    $task = Get-Task $LegacyName
    if (!$task) { return $null }
    $principal = New-Object -TypeName Security.Principal.NTAccount -ArgumentList @($task.Principal.UserId)
    try { $owner = $principal.Translate([Security.Principal.SecurityIdentifier]).Value }
    catch { $owner = $task.Principal.UserId }
    if ($owner -ne $Sid) { return $null }
    return $task
}
function Assert-Installation {
    Assert-Object (Split-Path $Root) $true
    Assert-Object $Root $true
    Assert-Object $Helper $false
    Assert-Object $Manifest $false
    $record = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
    if ($record.version -ne 2 -or $record.sid -ne $Sid -or $record.request_directory -cne $RequestDirectory -or
        $record.sha256 -notmatch '^[0-9a-f]{64}$' -or (File-Hash $Helper) -cne $record.sha256 -or
        ($ExpectedHash -ne '' -and $record.sha256 -cne $ExpectedHash)) {
        throw 'Helper hash or bound request directory requires repair'
    }
    $task = Get-Task $TaskName
    if (!$task -or $task.Actions.Count -ne 1 -or $task.Actions[0].Execute -cne $Helper -or
        $task.Actions[0].Arguments -cne '--lan-uplink-helper' -or $task.Principal.UserId -ne $Sid -or
        $task.Principal.RunLevel -ne 'Highest' -or $task.Principal.LogonType -ne 'Interactive') {
        throw 'Task action or principal requires repair'
    }
    $scheduler = New-Object -ComObject Schedule.Service
    $scheduler.Connect()
    $registered = $scheduler.GetFolder('\').GetTask($TaskName)
    $sd = New-Object -TypeName Security.AccessControl.RawSecurityDescriptor -ArgumentList @($registered.GetSecurityDescriptor(5))
    $wanted = New-Object -TypeName Security.AccessControl.RawSecurityDescriptor -ArgumentList @($TaskSddl)
    if ($sd.GetSddlForm($Sections) -cne $wanted.GetSddlForm($Sections)) { throw 'Task permissions require repair' }
    if (Own-LegacyTask) { throw 'Legacy mutable task requires repair' }
}
if ($Mode -eq 'probe') {
    if (!(Test-Path -LiteralPath $Root) -and !(Get-Task $TaskName) -and !(Own-LegacyTask)) { 'missing'; exit 0 }
    Assert-Installation
    'ready'
    exit 0
}
if ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne $Sid) {
    throw 'Use the same user for elevation; an alternate administrator must not own this task'
}
if ($Mode -eq 'cleanup') {
    if ($PrivateGuid -ne '' -or $PublicGuid -ne '') {
        $manager = New-Object -ComObject HNetCfg.HNetShare
        foreach ($connection in $manager.EnumEveryConnection) {
            $guid = $manager.NetConnectionProps.Invoke($connection).Guid
            if ($guid -eq $PrivateGuid -or $guid -eq $PublicGuid) {
                $config = $manager.INetSharingConfigurationForINetConnection.Invoke($connection)
                if ($config.SharingEnabled) { $config.DisableSharing() }
            }
        }
    }
    $task = Get-Task $TaskName
    if ($task) { Stop-ScheduledTask -TaskName $TaskName -TaskPath '\' -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName $TaskName -TaskPath '\' -Confirm:$false }
    if (Own-LegacyTask) { Stop-ScheduledTask -TaskName $LegacyName -TaskPath '\' -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName $LegacyName -TaskPath '\' -Confirm:$false }
    if (Test-Path -LiteralPath $Root) {
        Assert-Object $Root $true
        foreach ($item in Get-ChildItem -LiteralPath $Root -Force) {
            Assert-RegularFile $item.FullName
            if ($item.Name -notin @('se.exe', 'installation.json')) { throw 'Unexpected helper file; cleanup refused' }
            Remove-Item -LiteralPath $item.FullName -Force
        }
        [IO.Directory]::Delete($Root, $false)
    }
    'removed'
    exit 0
}
if ($Mode -ne 'install') { throw 'Unknown operation' }
# Source is held without write/delete sharing by the requesting process through
# the UAC decision; recheck its SHA-256 in the elevated process before copying.
$sourceHandle = [IO.File]::Open($Source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $actual = ([BitConverter]::ToString($sha.ComputeHash($sourceHandle))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
    if ($actual -cne $ExpectedHash) { throw 'Source hash changed; installation refused' }
    $sourceHandle.Position = 0
    New-PrivateDirectory (Split-Path $Root)
    if (Get-Task $TaskName) { Stop-ScheduledTask -TaskName $TaskName -TaskPath '\' -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName $TaskName -TaskPath '\' -Confirm:$false }
    if (Test-Path -LiteralPath $Root) {
        try { Assert-Object $Root $true }
        catch {
            $item = Get-Item -LiteralPath $Root -Force
            if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Unsafe helper directory' }
            # Replace the directory itself, so an old writable directory handle
            # cannot create/delete files in the new installation after repair.
            $retired = "$Root.retired-$([Guid]::NewGuid().ToString('N'))"
            [IO.Directory]::Move($Root, $retired)
            $acl = New-Object Security.AccessControl.DirectorySecurity
            $acl.SetSecurityDescriptorSddlForm($DirSddl)
            Set-Acl -LiteralPath $retired -AclObject $acl
        }
    }
    New-PrivateDirectory $Root
    foreach ($path in @($Helper, $Manifest)) {
        if (Test-Path -LiteralPath $path) { Assert-RegularFile $path; Remove-Item -LiteralPath $path -Force }
    }
    $dest = New-PrivateFile $Helper
    try { $sourceHandle.CopyTo($dest); $dest.Flush($true) } finally { $dest.Dispose() }
    Assert-Object $Helper $false
    if ((File-Hash $Helper) -cne $ExpectedHash) { throw 'Installed helper hash mismatch' }
    $record = @{ version = 2; sid = $Sid; request_directory = $RequestDirectory; sha256 = $ExpectedHash } | ConvertTo-Json -Compress
    $dest = New-PrivateFile $Manifest
    try { $bytes = [Text.Encoding]::UTF8.GetBytes($record); $dest.Write($bytes, 0, $bytes.Length); $dest.Flush($true) } finally { $dest.Dispose() }
    if (Own-LegacyTask) { Stop-ScheduledTask -TaskName $LegacyName -TaskPath '\' -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName $LegacyName -TaskPath '\' -Confirm:$false }
    $scheduler = New-Object -ComObject Schedule.Service
    $scheduler.Connect()
    $definition = $scheduler.NewTask(0)
    $definition.RegistrationInfo.Author = 'Smart Explorer'
    $definition.Principal.UserId = $Sid
    $definition.Principal.LogonType = 3
    $definition.Principal.RunLevel = 1
    $definition.Settings.DisallowStartIfOnBatteries = $false
    $definition.Settings.StopIfGoingOnBatteries = $false
    $definition.Settings.MultipleInstances = 2
    $definition.Settings.ExecutionTimeLimit = 'PT2M'
    $definition.Settings.AllowDemandStart = $true
    $action = $definition.Actions.Create(0)
    $action.Path = $Helper
    $action.Arguments = '--lan-uplink-helper'
    # CREATE_OR_UPDATE | DONT_ADD_PRINCIPAL_ACE. The protected owner/DACL
    # belongs to the task at registration, with no user-writable interval.
    $scheduler.GetFolder('\').RegisterTaskDefinition($TaskName, $definition, 22, $Sid, $null, 3, $TaskSddl) | Out-Null
    $service = Get-Service -Name SharedAccess
    if ($service.StartType -eq 'Disabled') { Set-Service -Name SharedAccess -StartupType Manual }
    Assert-Installation
    'ready'
} finally { $sourceHandle.Dispose() }
