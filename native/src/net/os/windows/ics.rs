//! Windows Internet Connection Sharing through the `HNetCfg.HNetShare` COM
//! automation object, driven by PowerShell exactly like the existing
//! elevated firewall helper. Enabling/disabling needs administrator rights
//! and therefore runs inside the scheduled-task helper (`uplink_helper.rs`);
//! probing works unelevated.
use std::io;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ICS_PUBLIC: u8 = 0;
const ICS_PRIVATE: u8 = 1;

fn run_powershell(script: &str) -> io::Result<String> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(io::Error::other(if stderr.is_empty() {
            format!("PowerShell beendete mit {}", output.status)
        } else {
            stderr
        }))
    }
}

fn require_guid(id: &str) -> io::Result<String> {
    let id = id.trim();
    if !crate::net::valid_adapter_id(id) || !id.starts_with('{') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Adapter-GUID ist ungueltig",
        ));
    }
    Ok(id.to_uppercase())
}

/// `HNetCfg.HNetShare` reports whether ICS is installed on this edition.
pub(crate) fn sharing_installed() -> io::Result<bool> {
    let out = run_powershell(
        "$m = New-Object -ComObject HNetCfg.HNetShare; if ($m.SharingInstalled) { 'yes' } else { 'no' }",
    )?;
    Ok(out.trim() == "yes")
}

/// `Disabled`, `Manual`, `Automatic`, or an error when the ICS service is
/// missing.
pub(crate) fn shared_access_startup() -> io::Result<String> {
    run_powershell("(Get-Service -Name SharedAccess -ErrorAction Stop).StartType.ToString()")
}

fn connection_script(guid: &str, variable: &str) -> String {
    format!(
        "${variable} = $null; foreach ($c in $m.EnumEveryConnection) {{ if ($m.NetConnectionProps.Invoke($c).Guid -eq '{guid}') {{ ${variable} = $c }} }}; \
         if ($null -eq ${variable}) {{ throw 'Netzwerkverbindung {guid} nicht gefunden' }}; \
         ${variable}cfg = $m.INetSharingConfigurationForINetConnection.Invoke(${variable}); "
    )
}

pub(crate) fn enable_sharing(public_guid: &str, private_guid: &str) -> io::Result<()> {
    let public_guid = require_guid(public_guid)?;
    let private_guid = require_guid(private_guid)?;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $m = New-Object -ComObject HNetCfg.HNetShare; \
         if (-not $m.SharingInstalled) {{ throw 'Internetverbindungsfreigabe ist nicht installiert' }}; \
         {}{}\
         foreach ($c in $m.EnumEveryConnection) {{ $cfg = $m.INetSharingConfigurationForINetConnection.Invoke($c); if ($cfg.SharingEnabled) {{ $g = $m.NetConnectionProps.Invoke($c).Guid; if ($g -ne '{public_guid}' -and $g -ne '{private_guid}') {{ $cfg.DisableSharing() }} }} }}; \
         if ($pubcfg.SharingEnabled -and $pubcfg.SharingConnectionType -ne {ICS_PUBLIC}) {{ $pubcfg.DisableSharing() }}; \
         if ($privcfg.SharingEnabled -and $privcfg.SharingConnectionType -ne {ICS_PRIVATE}) {{ $privcfg.DisableSharing() }}; \
         if (-not $pubcfg.SharingEnabled) {{ $pubcfg.EnableSharing({ICS_PUBLIC}) }}; \
         if (-not $privcfg.SharingEnabled) {{ $privcfg.EnableSharing({ICS_PRIVATE}) }}; 'ok'",
        connection_script(&public_guid, "pub"),
        connection_script(&private_guid, "priv"),
    );
    let out = run_powershell(&script)?;
    if out.trim().ends_with("ok") {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ICS-Aktivierung ohne Bestaetigung: {out}"
        )))
    }
}

pub(crate) fn disable_sharing(public_guid: &str, private_guid: &str) -> io::Result<()> {
    let public_guid = require_guid(public_guid)?;
    let private_guid = require_guid(private_guid)?;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $m = New-Object -ComObject HNetCfg.HNetShare; \
         foreach ($c in $m.EnumEveryConnection) {{ $g = $m.NetConnectionProps.Invoke($c).Guid; if ($g -eq '{public_guid}' -or $g -eq '{private_guid}') {{ $cfg = $m.INetSharingConfigurationForINetConnection.Invoke($c); if ($cfg.SharingEnabled) {{ $cfg.DisableSharing() }} }} }}; 'ok'"
    );
    let out = run_powershell(&script)?;
    if out.trim().ends_with("ok") {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ICS-Deaktivierung ohne Bestaetigung: {out}"
        )))
    }
}

/// Whether the private connection currently has ICS enabled as the private
/// (shared-to) side.
pub(crate) fn private_sharing_active(private_guid: &str) -> io::Result<bool> {
    let private_guid = require_guid(private_guid)?;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $m = New-Object -ComObject HNetCfg.HNetShare; $r = 'no'; \
         foreach ($c in $m.EnumEveryConnection) {{ if ($m.NetConnectionProps.Invoke($c).Guid -eq '{private_guid}') {{ $cfg = $m.INetSharingConfigurationForINetConnection.Invoke($c); if ($cfg.SharingEnabled -and $cfg.SharingConnectionType -eq {ICS_PRIVATE}) {{ $r = 'yes' }} }} }}; $r"
    );
    Ok(run_powershell(&script)?.trim() == "yes")
}
