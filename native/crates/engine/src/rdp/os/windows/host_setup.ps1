# Windows RDP self-setup (design 3.2). Fixed source inside se; the only input
# is $UserSid, set by se in front of this script after strict validation.
# Runs elevated (UAC or an already elevated service). Opens no firewall rule:
# se reaches the service over loopback through its own tunnel.
# Exit codes: 0 done, 10 policy denies RDP, 11 Windows Home, 12 edition
# without RDP host, 20 TermService does not run, 21 group membership failed,
# 30 any other failure.
$ErrorActionPreference = 'Stop'
try {
    $policyKey = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services'
    if (Test-Path -LiteralPath $policyKey) {
        $policy = Get-ItemProperty -LiteralPath $policyKey -ErrorAction SilentlyContinue
        if ($null -ne $policy -and $null -ne $policy.fDenyTSConnections -and [int]$policy.fDenyTSConnections -eq 1) { exit 10 }
    }
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $sku = [uint32]$os.OperatingSystemSKU
    $server = [uint32]$os.ProductType -in @(2, 3)
    $homeSkus = @(2, 3, 5, 11, 26, 47, 98, 99, 100, 101)
    $reviewedWorkstationSkus = @(4, 27, 48, 49, 69, 70, 72, 84, 125, 126, 129, 130, 161, 162, 164, 165, 171, 172)
    if ($sku -in $homeSkus) { exit 11 }
    if (-not ($server -or ($sku -in $reviewedWorkstationSkus))) { exit 12 }

    $ts = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server'
    $wasOff = [int](Get-ItemProperty -LiteralPath $ts).fDenyTSConnections -ne 0
    if ($wasOff) {
        Set-ItemProperty -LiteralPath $ts -Name fDenyTSConnections -Value 0 -Type DWord
        # NLA only when se switched RDP on; an administrator's choice stays.
        Set-ItemProperty -LiteralPath "$ts\WinStations\RDP-Tcp" -Name UserAuthentication -Value 1 -Type DWord
    }

    $service = Get-Service -Name TermService
    if ($service.StartType -eq 'Disabled') { Set-Service -Name TermService -StartupType Manual }
    if ($service.Status -ne 'Running') {
        try { Start-Service -Name TermService } catch { exit 20 }
    }
    if ((Get-Service -Name TermService).Status -ne 'Running') { exit 20 }

    # Remote Desktop Users (S-1-5-32-555) unless already there or an
    # Administrator (S-1-5-32-544). Group names are localized; SIDs are not.
    function Test-LocalMember([string]$GroupSid) {
        $name = (New-Object System.Security.Principal.SecurityIdentifier($GroupSid)).Translate([System.Security.Principal.NTAccount]).Value.Split('\')[-1]
        $group = [ADSI]("WinNT://./" + $name + ",group")
        foreach ($member in @($group.psbase.Invoke('Members'))) {
            $bytes = $member.GetType().InvokeMember('objectSid', 'GetProperty', $null, $member, $null)
            if ((New-Object System.Security.Principal.SecurityIdentifier($bytes, 0)).Value -eq $UserSid) { return $true }
        }
        return $false
    }
    $member = $false
    try { $member = (Test-LocalMember 'S-1-5-32-555') -or (Test-LocalMember 'S-1-5-32-544') } catch { $member = $false }
    if (-not $member) {
        try {
            Add-LocalGroupMember -SID 'S-1-5-32-555' -Member $UserSid
        } catch {
            if ($_.FullyQualifiedErrorId -notlike 'MemberExists*') { exit 21 }
        }
    }
    exit 0
} catch {
    exit 30
}
