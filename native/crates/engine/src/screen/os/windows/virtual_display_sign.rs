//! The local signature of the driver catalog (plan 2026-11-17 P5, user
//! decision 2026-11-17): in the elevated step se creates a code-signing
//! certificate on this device, signs `mttvdd.cat` with it, trusts the
//! certificate in `LocalMachine\Root` and `LocalMachine\TrustedPublisher`
//! and deletes its private key before anything else happens, so nothing else
//! can ever be signed with it. Windows PowerShell 5.1 (part of Windows 10
//! and 11, `%SystemRoot%\System32\WindowsPowerShell\v1.0`) does the
//! certificate work (`New-SelfSignedCertificate`, `Set-AuthenticodeSignature`,
//! `Get-AuthenticodeSignature`); the paths reach the script through
//! environment variables, never through its text. The script checks the new
//! signature before it trusts nothing more than that one certificate.
//!
//! Why a local signature suffices: the driver is a user-mode (UMDF 2,
//! IddCx) driver; the only kernel binary it uses is Microsoft's inbox
//! `WUDFRd.sys`. Microsoft's kernel-mode signing policy (Dev Portal
//! signature since Windows 10 1607 with Secure Boot) applies to kernel-mode
//! binaries; PnP installation requires a catalog signature that chains to a
//! trusted root, and a publisher in `TrustedPublisher` installs without a
//! prompt. Windows 10 in S mode and ARM64 editions require WHQL catalogs and
//! are not served (docs/GOTCHAS.md).
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::screen::core::virtual_display::windows_driver::CERT_SUBJECT;

/// The script of the signature. It prints `THUMBPRINT=<hex>` on success.
const SIGN_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$catalog = $env:SE_VDD_CATALOG
$subject = $env:SE_VDD_SUBJECT
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $subject `
    -CertStoreLocation 'Cert:\LocalMachine\My' -KeyExportPolicy NonExportable `
    -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -NotAfter (Get-Date).AddYears(100)
$thumb = $cert.Thumbprint
try {
    $signed = Set-AuthenticodeSignature -FilePath $catalog -Certificate $cert -HashAlgorithm SHA256
    if ($signed.Status -ne 'Valid' -and $signed.Status -ne 'UnknownError') { throw "sign: $($signed.Status) $($signed.StatusMessage)" }
} finally {
    Remove-Item -Path "Cert:\LocalMachine\My\$thumb" -DeleteKey -Force
}
if (Test-Path "Cert:\LocalMachine\My\$thumb") { throw 'private key still present' }
$public = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($cert.RawData)
foreach ($name in 'Root', 'TrustedPublisher') {
    $store = [System.Security.Cryptography.X509Certificates.X509Store]::new($name, 'LocalMachine')
    $store.Open('ReadWrite')
    $store.Add($public)
    $store.Close()
}
$check = Get-AuthenticodeSignature -FilePath $catalog
if ($check.Status -ne 'Valid' -or $check.SignerCertificate.Thumbprint -ne $thumb) {
    throw "check: $($check.Status) $($check.StatusMessage)"
}
"THUMBPRINT=$thumb"
"#;

/// The script that removes se's former certificates from both stores: every
/// certificate with se's subject except `keep`.
const FORGET_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$subject = $env:SE_VDD_SUBJECT
$keep = $env:SE_VDD_KEEP
foreach ($name in 'Root', 'TrustedPublisher') {
    $store = [System.Security.Cryptography.X509Certificates.X509Store]::new($name, 'LocalMachine')
    $store.Open('ReadWrite')
    foreach ($cert in @($store.Certificates)) {
        if ($cert.Subject -eq $subject -and $cert.Thumbprint -ne $keep) { $store.Remove($cert) }
    }
    $store.Close()
}
"OK"
"#;

/// `%SystemRoot%\System32` as the kernel names it (never from the
/// environment, which the caller of an elevation chooses).
pub fn system_directory() -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 1024];
    // SAFETY: the buffer and its length match.
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Systemordner unbekannt".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

fn powershell() -> Result<PathBuf, String> {
    let path = system_directory()?.join(r"WindowsPowerShell\v1.0\powershell.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} fehlt", path.display()))
    }
}

/// `-EncodedCommand` takes the script as Base64 of its UTF-16LE text, so a
/// multi-line script runs as one (`-Command -` would read it line by line).
fn encoded(script: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

fn run(script: &str, env: &[(&str, &str)]) -> Result<String, String> {
    let mut command = Command::new(powershell()?);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
        ])
        .arg(encoded(script))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    // CREATE_NO_WINDOW: the updater has no console to share.
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let output = command
        .output()
        .map_err(|error| format!("PowerShell: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "PowerShell endete mit {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    Ok(stdout)
}

/// Signs `catalog` with a new certificate of this device and trusts it;
/// the private key is gone when this returns. The certificate's thumbprint.
pub fn sign_catalog(catalog: &Path) -> Result<String, String> {
    let catalog = catalog
        .to_str()
        .ok_or("Katalogpfad ist kein Unicode")?
        .to_string();
    let out = run(
        SIGN_SCRIPT,
        &[
            ("SE_VDD_CATALOG", &catalog),
            ("SE_VDD_SUBJECT", CERT_SUBJECT),
        ],
    )?;
    out.lines()
        .find_map(|line| line.trim().strip_prefix("THUMBPRINT="))
        .filter(|thumb| thumb.len() == 40 && thumb.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| format!("Signatur ohne Fingerabdruck: {}", out.trim()))
}

/// Removes se's certificates other than `keep` from `Root` and
/// `TrustedPublisher` (after a new signature replaced the old one).
pub fn forget_other_certificates(keep: &str) -> Result<(), String> {
    run(
        FORGET_SCRIPT,
        &[("SE_VDD_SUBJECT", CERT_SUBJECT), ("SE_VDD_KEEP", keep)],
    )
    .map(|_| ())
}
