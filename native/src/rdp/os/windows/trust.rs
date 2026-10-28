//! mstsc trust for one device address (design 2.7.2, 2.8).
//!
//! `CertHash`: Microsoft documents no certificate pin for `.rdp` files; the
//! store mstsc itself fills after „Don't ask me again“ is
//! `HKCU\Software\Microsoft\Terminal Server Client\Servers\<name>` with the
//! REG_BINARY value `CertHash` (the 20-byte SHA-1 thumbprint). se writes the
//! thumbprint of the authenticated probe v2 under `<device address>` and
//! `<device address>:<port>` (assumption A6: mstsc then skips the warning,
//! also on a name mismatch, under one of the two names).
//!
//! se-managed service credentials (GNOME desktop sharing) are handed to
//! mstsc as a generic credential `TERMSRV/<device address>` that lives only
//! for this logon session (`CRED_PERSIST_SESSION`) and is deleted when the
//! RDP session ends. Account passwords never pass through se.
use crate::rdp::core::{CertificateSha1, HostService};
use crate::rdp::os::shared::clients::Credentials;
use std::net::{IpAddr, SocketAddr};
use winreg::enums::{RegType, HKEY_CURRENT_USER};
use winreg::{RegKey, RegValue};

const SERVERS: &str = r"Software\Microsoft\Terminal Server Client\Servers";

/// The registry key names mstsc may use for this listener.
pub(crate) fn server_names(listener: SocketAddr) -> [String; 2] {
    let address = listener.ip();
    [
        address.to_string(),
        format!("{address}:{}", listener.port()),
    ]
}

/// Writes `CertHash` for both names; any failure aborts the start before
/// the client runs (no native certificate question as a fallback).
pub(crate) fn write_cert_hash(listener: SocketAddr, sha1: &CertificateSha1) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for name in server_names(listener) {
        let path = format!(r"{SERVERS}\{name}");
        let (key, _) = hkcu
            .create_subkey(&path)
            .map_err(|error| format!(r"HKCU\{path}: {error}"))?;
        let value = RegValue {
            bytes: sha1.bytes().to_vec(),
            vtype: RegType::REG_BINARY,
        };
        key.set_raw_value("CertHash", &value)
            .map_err(|error| format!(r"HKCU\{path}\CertHash: {error}"))?;
    }
    Ok(())
}

/// The stored thumbprint of one name, for the task tests.
pub(crate) fn read_cert_hash(name: &str) -> Option<Vec<u8>> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"{SERVERS}\{name}"))
        .ok()?;
    let value = key.get_raw_value("CertHash").ok()?;
    (value.vtype == RegType::REG_BINARY).then_some(value.bytes)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A session-only `TERMSRV/<address>` credential, deleted on drop.
struct SessionCredential {
    target: Vec<u16>,
}
impl SessionCredential {
    fn write(address: IpAddr, credentials: &Credentials) -> Result<Self, String> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_SESSION, CRED_TYPE_GENERIC,
        };
        let mut target = wide(&format!("TERMSRV/{address}"));
        let user = match &credentials.domain {
            Some(domain) => format!("{domain}\\{}", credentials.username),
            None => credentials.username.clone(),
        };
        let mut user = wide(&user);
        let mut blob: Vec<u8> = credentials
            .password
            .expose()
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let size = u32::try_from(blob.len()).map_err(|_| "RDP password is too long".to_string())?;
        // SAFETY: a zeroed CREDENTIALW is a valid empty record before the
        // fields below are set.
        let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = target.as_mut_ptr();
        credential.CredentialBlobSize = size;
        credential.CredentialBlob = blob.as_mut_ptr();
        credential.Persist = CRED_PERSIST_SESSION;
        credential.UserName = user.as_mut_ptr();
        // SAFETY: every pointer refers to a live, terminated buffer.
        let written = unsafe { CredWriteW(&credential, 0) };
        let error = std::io::Error::last_os_error();
        blob.fill(0);
        if written == 0 {
            return Err(format!("RDP-Zugangsdaten nicht übergeben: {error}"));
        }
        Ok(Self { target })
    }
}
impl Drop for SessionCredential {
    fn drop(&mut self) {
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        // SAFETY: terminated target name of the credential written above.
        unsafe {
            CredDeleteW(self.target.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
    }
}

/// Trust and credentials for one start; the returned guard lives as long
/// as the client (`ClientProcess`/`RdpSession` resources). A host without
/// probe v2 has no SHA-1 thumbprint: no start (design 2.7.2).
pub(crate) fn prepare_trust(
    listener: SocketAddr,
    service: &HostService,
    credentials: Option<&Credentials>,
) -> Result<Box<dyn Send>, String> {
    let sha1 = service
        .certificate_sha1
        .as_ref()
        .ok_or_else(|| crate::rdp::OLD_HOST.to_string())?;
    write_cert_hash(listener, sha1)?;
    let credential = credentials
        .map(|credentials| SessionCredential::write(listener.ip(), credentials))
        .transpose()?;
    Ok(Box::new(credential))
}
