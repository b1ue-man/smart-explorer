# Read-only Windows host inventory. No credentials, key export, service,
# registry, NLA, TLS or firewall mutation. Evaluated only as fixed source.
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
$certSource = $null
$apps = [System.Collections.Generic.List[object]]::new()
if ($supported) {
    try {
        $settings = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server'
        $listener = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp'
        $enabled = [int]$settings.fDenyTSConnections -eq 0
        $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='TermService'"
        $running = $service.State -eq 'Running'
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
    remote_apps = @($apps.ToArray())
    notes = @($notes.ToArray())
} | ConvertTo-Json -Depth 5 -Compress
