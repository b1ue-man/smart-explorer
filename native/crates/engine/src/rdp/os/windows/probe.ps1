# Read-only Windows host inventory. No credentials, key export, service,
# registry, NLA, TLS or firewall mutation. Evaluated only as fixed source.
# The self-setup (host_setup.ps1) changes RDP; this probe never does.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$notes = [System.Collections.Generic.List[string]]::new()
$os = Get-CimInstance -ClassName Win32_OperatingSystem
$sku = [uint32]$os.OperatingSystemSKU
$server = [uint32]$os.ProductType -in @(2, 3)
$reviewedWorkstationSkus = @(4, 27, 48, 49, 69, 70, 72, 84, 125, 126, 129, 130, 161, 162, 164, 165, 171, 172)
$homeSkus = @(2, 3, 5, 11, 26, 47, 98, 99, 100, 101)
$supported = $server -or ($sku -in $reviewedWorkstationSkus)
$enabled = $false
$running = $false
$port = $null
$tls = $null
$nla = $null
$sha256 = $null
$sha1 = $null
$certSource = $null
$termServicePid = $null
$policyDenies = $false
$policyKey = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Terminal Services'
if (Test-Path -LiteralPath $policyKey) {
    $policy = Get-ItemProperty -LiteralPath $policyKey -ErrorAction SilentlyContinue
    if ($null -ne $policy -and $null -ne $policy.fDenyTSConnections -and [int]$policy.fDenyTSConnections -eq 1) { $policyDenies = $true }
}
$passwordless = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\Device' -ErrorAction SilentlyContinue
if ($null -ne $passwordless -and [int]$passwordless.DevicePasswordLessBuildVersion -eq 2) {
    $notes.Add('Kennwortlose Windows-Anmeldung aktiv: RDP braucht das Kontokennwort')
}
$apps = [System.Collections.Generic.List[object]]::new()
if ($supported) {
    try {
        $settings = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server'
        $listener = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp'
        $enabled = [int]$settings.fDenyTSConnections -eq 0
        $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='TermService'"
        $running = $service.State -eq 'Running'
        if ($running) { $termServicePid = [uint32]$service.ProcessId }
        $configuredPort = [int]$listener.PortNumber
        if ($running -and $configuredPort -gt 0 -and $configuredPort -le 65535) {
            $sockets = @(Get-NetTCPConnection -State Listen -LocalPort $configuredPort -ErrorAction Stop |
                Where-Object { $_.OwningProcess -eq $service.ProcessId -and $_.LocalAddress -in @('0.0.0.0', '::', '127.0.0.1', '::1') })
            if ($sockets.Count -gt 0) { $port = $configuredPort }
        }
        $general = Get-CimInstance -Namespace 'root/cimv2/TerminalServices' -ClassName Win32_TSGeneralSetting -Filter "TerminalName='RDP-Tcp'"
        $tls = [uint32]$general.SecurityLayer -in @(1, 2)
        $nla = [uint32]$general.UserAuthenticationRequired -eq 1
        $thumbprint = ([string]$general.SSLCertificateSHA1Hash).Replace(' ', '')
        if ($thumbprint -match '^[0-9A-Fa-f]{40}$') {
            # The listener's own thumbprint is what mstsc stores as CertHash.
            $sha1 = $thumbprint.ToUpperInvariant()
            # SHA-1 is only the OS certificate lookup key; the advertised pin
            # is independently SHA-256 over the bound certificate DER.
            $certificates = @()
            foreach ($storePath in @('Cert:\LocalMachine\Remote Desktop', 'Cert:\LocalMachine\My')) {
                if (Test-Path -LiteralPath $storePath) {
                    $certificates += @(Get-ChildItem -LiteralPath $storePath | Where-Object { $_.Thumbprint -eq $thumbprint })
                }
            }
            if ($certificates.Count -gt 0) {
                $hash = [System.Security.Cryptography.SHA256]::Create()
                try { $sha256 = [BitConverter]::ToString($hash.ComputeHash($certificates[0].RawData)).Replace('-', '') }
                finally { $hash.Dispose() }
                $certSource = 'Win32_TSGeneralSetting.SSLCertificateSHA1Hash -> matching LocalMachine certificate -> SHA256(DER)'
            } else { $notes.Add('The certificate selected by the RDP listener is absent from the readable LocalMachine certificate inventory') }
        } else { $notes.Add('The RDP listener did not disclose an unambiguous bound certificate; no certificate-store default is guessed') }
    } catch { $notes.Add('RDP listener inventory is incomplete: ' + $_.Exception.Message) }
    if ($server) {
        try {
            if ((Get-WindowsFeature -Name RDS-RD-Server).Installed -and (Get-Command Get-RDRemoteApp -ErrorAction SilentlyContinue)) {
                $machine = Get-CimInstance -ClassName Win32_ComputerSystem
                $localNames = @([string]$machine.Name, [string]$machine.DNSHostName, ([string]$machine.DNSHostName + '.' + [string]$machine.Domain))
                foreach ($collection in @(Get-RDSessionCollection -ErrorAction Stop)) {
                    $hosts = @(Get-RDSessionHost -CollectionName $collection.CollectionName -ErrorAction Stop)
                    $isLocal = @($hosts | Where-Object { [string]$_.SessionHost -in $localNames }).Count -gt 0
                    if (-not $isLocal) { continue }
                    foreach ($app in @(Get-RDRemoteApp -CollectionName $collection.CollectionName -ErrorAction Stop)) {
                        $alias = [string]$app.Alias
                        if ($alias -match '^[A-Za-z0-9_.-]{1,256}$') {
                            $apps.Add([pscustomobject]@{ alias = $alias; display_name = [string]$app.DisplayName; collection = [string]$collection.CollectionName })
                        } else { $notes.Add('A published RDS alias cannot be represented safely by this client adapter') }
                    }
                }
            }
        } catch { $notes.Add('Local RDS RemoteApp publication inventory is unavailable: ' + $_.Exception.Message) }
    }
}
# E29 / O10: facts of the firewall reading for the RDP port. Rule names stay on
# this host: only profile names, the active network profiles, booleans and the
# rule's profile list leave it. probe.rs decides; a fact that was not read is null.
# Every query reads the ActiveStore, the effective policy (group policy and the
# local stores); without -PolicyStore the cmdlets read only the PersistentStore.
function Test-RdpPortFilter([object[]]$values, [int]$rdpPort) {
    foreach ($value in $values) {
        $text = ([string]$value).Trim()
        if ($text -eq 'Any') { return $true }
        if ($text -match '^(\d{1,5})-(\d{1,5})$') {
            if ($rdpPort -ge [int]$Matches[1] -and $rdpPort -le [int]$Matches[2]) { return $true }
        } elseif ($text -match '^\d{1,5}$' -and [int]$text -eq $rdpPort) { return $true }
    }
    return $false
}
$firewallProfiles = $null
$firewallActive = $null
$firewallCandidates = $null
$firewallErrors = [System.Collections.Generic.List[string]]::new()
if ($supported -and $null -ne $port) {
    try {
        $profileRows = [System.Collections.Generic.List[object]]::new()
        foreach ($firewallProfile in @(Get-NetFirewallProfile -All -PolicyStore ActiveStore -ErrorAction Stop)) {
            $enabledText = [string]$firewallProfile.Enabled
            $profileEnabled = $null
            if ($enabledText -eq 'True') { $profileEnabled = $true }
            if ($enabledText -eq 'False') { $profileEnabled = $false }
            $inboundText = [string]$firewallProfile.DefaultInboundAction
            $profileInbound = $null
            if ($inboundText -eq 'Allow') { $profileInbound = 'allow' }
            if ($inboundText -eq 'Block') { $profileInbound = 'block' }
            $profileRows.Add([pscustomobject]@{ name = [string]$firewallProfile.Name; enabled = $profileEnabled; inbound = $profileInbound })
        }
        $firewallProfiles = @($profileRows.ToArray())
    } catch { $firewallErrors.Add('Firewallprofile nicht lesbar: ' + $_.Exception.Message) }
    try {
        $activeRows = [System.Collections.Generic.List[string]]::new()
        $activeKnown = $true
        foreach ($connection in @(Get-NetConnectionProfile -ErrorAction Stop)) {
            switch ([string]$connection.NetworkCategory) {
                'DomainAuthenticated' { $activeRows.Add('Domain') }
                'Private' { $activeRows.Add('Private') }
                'Public' { $activeRows.Add('Public') }
                default { $activeKnown = $false }
            }
        }
        if ($activeKnown) { $firewallActive = @($activeRows.ToArray()) }
    } catch { $firewallErrors.Add('Netzprofil nicht lesbar: ' + $_.Exception.Message) }
    try {
        $seenRules = @{}
        $candidateRows = [System.Collections.Generic.List[object]]::new()
        foreach ($portFilter in @(Get-NetFirewallPortFilter -All -PolicyStore ActiveStore -ErrorAction Stop)) {
            if ([string]$portFilter.Protocol -notin @('TCP', '6', 'Any')) { continue }
            if (-not (Test-RdpPortFilter @($portFilter.LocalPort) ([int]$port))) { continue }
            foreach ($rule in @(Get-NetFirewallRule -AssociatedNetFirewallPortFilter $portFilter -PolicyStore ActiveStore -ErrorAction Stop)) {
                if ([string]$rule.Enabled -ne 'True' -or [string]$rule.Direction -ne 'Inbound' -or [string]$rule.Action -ne 'Allow') { continue }
                $ruleKey = [string]$rule.InstanceID
                if ($seenRules.ContainsKey($ruleKey)) { continue }
                $seenRules[$ruleKey] = $true
                $addressFilters = @(Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $rule -PolicyStore ActiveStore -ErrorAction Stop)
                $fromNetwork = $addressFilters.Count -eq 0
                foreach ($addressFilter in $addressFilters) {
                    foreach ($address in @($addressFilter.RemoteAddress)) {
                        if (@('127.0.0.1', '::1') -notcontains ([string]$address)) { $fromNetwork = $true }
                    }
                }
                $programFilters = @(Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $rule -PolicyStore ActiveStore -ErrorAction Stop)
                $serviceFilters = @(Get-NetFirewallServiceFilter -AssociatedNetFirewallRule $rule -PolicyStore ActiveStore -ErrorAction Stop)
                $programScoped = @($programFilters | Where-Object { [string]$_.Program -ne 'Any' }).Count -gt 0
                $serviceScoped = @($serviceFilters | Where-Object { [string]$_.Service -ne 'Any' }).Count -gt 0
                $candidateRows.Add([pscustomobject]@{ profile = [string]$rule.Profile; network = [bool]$fromNetwork; scoped = [bool]($programScoped -or $serviceScoped) })
            }
        }
        $firewallCandidates = @($candidateRows.ToArray())
    } catch { $firewallErrors.Add('Firewallregeln nicht lesbar: ' + $_.Exception.Message) }
}
$firewallNote = $null
if ($firewallErrors.Count -gt 0) { $firewallNote = $firewallErrors -join '; ' }
[pscustomobject]@{
    supported = [bool]$supported
    home = [bool]($sku -in $homeSkus)
    server = [bool]$server
    edition = [string]$os.Caption
    sku = $sku
    version = [string]$os.Version
    enabled = [bool]$enabled
    running = [bool]$running
    port = $port
    tls = $tls
    nla = $nla
    certificate_sha256 = $sha256
    certificate_source = $certSource
    certificate_sha1 = $sha1
    termservice_pid = $termServicePid
    policy_denies = [bool]$policyDenies
    remote_apps = @($apps.ToArray())
    notes = @($notes.ToArray())
    firewall_profiles = $firewallProfiles
    firewall_active = $firewallActive
    firewall_candidates = $firewallCandidates
    firewall_note = $firewallNote
} | ConvertTo-Json -Depth 5 -Compress
