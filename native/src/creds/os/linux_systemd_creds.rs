//! Linux adapter: protects the credential store's data key with
//! `systemd-creds --with-key=host` (systemd(1) credentials, systemd-creds(1)
//! "latest", read 2026-10-16). Root uses the system scope (systemd ≥ 250),
//! every other user the user scope (`--user`, systemd ≥ 256): the host
//! secret `/var/lib/systemd/credential.secret` belongs to root, the user
//! scope also binds UID, user name and machine-id. A protector is used only
//! after a full encrypt/decrypt round trip on this machine succeeded.
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::key_protector::{Protected, SYSTEMD_CREDS_SYSTEM, SYSTEMD_CREDS_USER};
use crate::creds::sealed_record::{hex, hex_decode, DataKey, KeyringDescriptor};

/// Absolute locations only: the store never runs a `systemd-creds` from PATH.
const BINARIES: [&str; 2] = ["/usr/bin/systemd-creds", "/bin/systemd-creds"];
const CREDENTIAL_NAME: &str = "--name=smart-explorer-secrets";
/// Host key only, never TPM2 (see `key_protector.rs`).
const WITH_KEY: &str = "--with-key=host";
/// Encryption reads the host secret, decryption of a user credential goes
/// through the socket-activated `io.systemd.Credentials` service; both take
/// milliseconds. The bound only keeps a hung service from blocking a login.
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    System,
    User,
}

impl Scope {
    fn protector(self) -> &'static str {
        match self {
            Scope::System => SYSTEMD_CREDS_SYSTEM,
            Scope::User => SYSTEMD_CREDS_USER,
        }
    }
}

/// `Some` when systemd credentials protect `key` on this machine.
pub(super) fn protect(directory: &Path, key: &DataKey) -> Option<Protected> {
    let scope = usable_scope(directory)?;
    let plain = Zeroizing::new(hex(&key.expose()[..]));
    let wrapped = run(scope, "encrypt", plain.as_bytes()).ok()?;
    let wrapped = String::from_utf8(wrapped.to_vec()).ok()?;
    let wrapped = wrapped.trim().to_string();
    let reopened = run(scope, "decrypt", wrapped.as_bytes()).ok()?;
    let reopened = key_from_hex(std::str::from_utf8(&reopened).ok()?).ok()?;
    (reopened.id() == key.id()).then(|| Protected {
        protector: scope.protector(),
        wrapped,
    })
}

pub(super) fn recover(descriptor: &KeyringDescriptor) -> Result<DataKey, String> {
    let scope = if descriptor.protector == SYSTEMD_CREDS_USER {
        Scope::User
    } else {
        Scope::System
    };
    let opened = run(scope, "decrypt", descriptor.wrapped.as_bytes()).map_err(|error| {
        format!(
            "Geheimnisspeicher gesperrt: systemd-creds entschlüsselt den Schlüssel nicht: {error}"
        )
    })?;
    let text = std::str::from_utf8(&opened)
        .map_err(|_| "Geheimnisspeicher: systemd-creds lieferte keinen Text".to_string())?;
    key_from_hex(text)
}

/// Root: system scope, but only while `/var/lib/systemd` lives on the same
/// file system as the store, so a container whose state volume outlives an
/// ephemeral `/var/lib` never seals its keys to a secret it will lose.
/// Others: user scope where systemd runs as the system manager.
fn usable_scope(directory: &Path) -> Option<Scope> {
    binary()?;
    if unsafe { libc::geteuid() } == 0 {
        let store = std::fs::metadata(directory).ok()?;
        let state = std::fs::metadata("/var/lib/systemd").ok()?;
        (store.dev() == state.dev()).then_some(Scope::System)
    } else {
        Path::new("/run/systemd/system")
            .is_dir()
            .then_some(Scope::User)
    }
}

fn binary() -> Option<&'static str> {
    BINARIES.into_iter().find(|path| {
        std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.uid() == 0)
    })
}

fn run(scope: Scope, verb: &str, input: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    let binary = binary().ok_or_else(|| "systemd-creds ist nicht installiert".to_string())?;
    let mut command = Command::new(binary);
    if scope == Scope::User {
        command.arg("--user");
    }
    command.arg(verb).arg(CREDENTIAL_NAME);
    if verb == "encrypt" {
        command.arg(WITH_KEY);
    }
    command
        .args(["-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("systemd-creds starten: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .map_err(|error| format!("systemd-creds Eingabe: {error}"))?;
    }
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "systemd-creds ohne Ausgabe".to_string())?;
    let reader = std::thread::Builder::new()
        .name("systemd-creds-out".into())
        .spawn(move || {
            let mut output = Zeroizing::new(Vec::new());
            let result = (&mut stdout)
                .take(MAX_OUTPUT_BYTES)
                .read_to_end(&mut output);
            result.map(|_| output)
        })
        .map_err(|error| format!("systemd-creds lesen: {error}"))?;
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("systemd-creds antwortet nicht".into());
            }
            Err(error) => return Err(format!("systemd-creds warten: {error}")),
        }
    };
    let output = reader
        .join()
        .map_err(|_| "systemd-creds lesen abgebrochen".to_string())?
        .map_err(|error| format!("systemd-creds lesen: {error}"))?;
    if !status.success() {
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = (&mut stderr).take(4096).read_to_string(&mut detail);
        }
        return Err(format!(
            "systemd-creds {verb} endete mit {status}: {}",
            detail.trim()
        ));
    }
    Ok(output)
}

fn key_from_hex(text: &str) -> Result<DataKey, String> {
    let bytes = Zeroizing::new(
        hex_decode(text.trim())
            .ok_or_else(|| "Geheimnisspeicher: Schlüssel ist kein Hex".to_string())?,
    );
    DataKey::from_slice(&bytes)
}
