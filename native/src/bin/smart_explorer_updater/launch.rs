use super::hash::verify_sha256;
use std::path::Path;
use std::time::{Duration, Instant};

const ACK_PATH_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_PATH";
const ACK_TOKEN_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_TOKEN";
const ACK_PAYLOAD_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_PAYLOAD";
const ACK_SIGNAL_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_SIGNAL";
const ACK_PREFIX: &str = "update_start_ack_";

struct AckEnvironment<'a> {
    path: &'a Path,
    token: &'a str,
    payload: Option<&'a str>,
    signal: &'a str,
}

pub(crate) fn spawn_verified_detached(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
) -> std::io::Result<()> {
    spawn_child(exe, expected_sha256, args, None).map(|_| ())
}

/// Start a replacement app and retain the child handle until its first GUI
/// frame writes the one-shot acknowledgement. Every error is returned only
/// after the exact child has exited, so callers may safely roll back its EXE.
pub(crate) fn spawn_verified_acknowledged(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
) -> std::io::Result<()> {
    spawn_verified_acknowledged_with(exe, expected_sha256, args, Duration::from_secs(45))
}

/// Use the durable completion receipt itself as the app-written ACK. If this
/// helper exits after the first frame but before cleanup, a serialized worker
/// can still recognize the completed launch without starting a duplicate app.
pub(crate) fn spawn_verified_acknowledged_receipt(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
    receipt_path: &Path,
    receipt_prefix: &[u8],
) -> std::io::Result<()> {
    let token = random_token()?;
    let prefix = std::str::from_utf8(receipt_prefix).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Startabschluss ist kein UTF-8: {error}"),
        )
    })?;
    let payload = format!("{prefix}{token}\n");
    ensure_ack_path_missing(receipt_path)?;
    let (listener, signal) = ack_listener()?;
    let environment = AckEnvironment {
        path: receipt_path,
        token: &token,
        payload: Some(&payload),
        signal: &signal,
    };
    let mut child = spawn_child(exe, expected_sha256, args, Some(environment))?;
    wait_for_ack(
        &mut child,
        &listener,
        receipt_path,
        &payload,
        &token,
        Duration::from_secs(45),
        false,
    )
}

fn spawn_verified_acknowledged_with(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<()> {
    let token = random_token()?;
    let ack_path = super::logging::appdata_dir().join(format!("{ACK_PREFIX}{token}"));
    ensure_ack_path_missing(&ack_path)?;
    let (listener, signal) = ack_listener()?;
    let environment = AckEnvironment {
        path: &ack_path,
        token: &token,
        payload: None,
        signal: &signal,
    };
    let mut child = spawn_child(exe, expected_sha256, args, Some(environment))?;
    wait_for_ack(
        &mut child, &listener, &ack_path, &token, &token, timeout, true,
    )
}

fn spawn_child(
    exe: &Path,
    expected_sha256: &str,
    args: &[&str],
    ack: Option<AckEnvironment<'_>>,
) -> std::io::Result<std::process::Child> {
    validate_before_spawn(exe, expected_sha256)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let mut command = configured_command(exe, args, ack.as_ref());
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Ok(child) => Ok(child),
            Err(_) => {
                validate_before_spawn(exe, expected_sha256)?;
                let mut retry = configured_command(exe, args, ack.as_ref());
                retry.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
                retry.spawn()
            }
        }
    }
    #[cfg(not(windows))]
    configured_command(exe, args, ack.as_ref()).spawn()
}

fn configured_command(
    exe: &Path,
    args: &[&str],
    ack: Option<&AckEnvironment<'_>>,
) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .args(args)
        .env_remove(ACK_PATH_ENV)
        .env_remove(ACK_TOKEN_ENV)
        .env_remove(ACK_PAYLOAD_ENV)
        .env_remove(ACK_SIGNAL_ENV);
    if let Some(ack) = ack {
        command
            .env(ACK_PATH_ENV, ack.path)
            .env(ACK_TOKEN_ENV, ack.token)
            .env(ACK_SIGNAL_ENV, ack.signal);
        if let Some(payload) = ack.payload {
            command.env(ACK_PAYLOAD_ENV, payload);
        }
    }
    command
}

fn wait_for_ack(
    child: &mut std::process::Child,
    listener: &std::net::TcpListener,
    ack_path: &Path,
    expected_content: &str,
    expected_token: &str,
    timeout: Duration,
    remove_on_success: bool,
) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let signal = match receive_signal(listener, expected_token) {
            Ok(signal) => signal,
            Err(error) => {
                return fail_or_accept_published(
                    child,
                    ack_path,
                    expected_content,
                    remove_on_success,
                    error,
                );
            }
        };
        if let Some(mut signal) = signal {
            if !matches!(read_ack(ack_path, expected_content), Ok(true)) {
                super::logging::append_log(
                    "warning: gueltiges Startsignal empfangen, aber Bestaetigungsdatei nicht erneut lesbar; Update bleibt aus Sicherheitsgruenden committed",
                );
            }
            // A durable first-frame ACK is the commit point. Do not roll back
            // after it: the app may now start background children safely.
            use std::io::Write;
            if let Err(error) = signal.write_all(&[1]).and_then(|_| signal.flush()) {
                super::logging::append_log(&format!(
                    "warning: dauerhafte Startbestaetigung wurde akzeptiert, aber die Freigabeantwort schlug fehl: {error}"
                ));
            }
            if remove_on_success {
                cleanup_ack(ack_path);
            }
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return fail_or_accept_published(
                    child,
                    ack_path,
                    expected_content,
                    remove_on_success,
                    std::io::Error::other(format!(
                        "Ersatzprogramm endete vor der Startbestaetigung ({status})"
                    )),
                );
            }
            Ok(None) => {}
            Err(error) => {
                return fail_or_accept_published(
                    child,
                    ack_path,
                    expected_content,
                    remove_on_success,
                    error,
                );
            }
        }
        if Instant::now() >= deadline {
            return fail_or_accept_published(
                child,
                ack_path,
                expected_content,
                remove_on_success,
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Ersatzprogramm bestaetigte seinen ersten GUI-Frame nicht rechtzeitig",
                ),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn fail_or_accept_published(
    child: &mut std::process::Child,
    ack_path: &Path,
    expected_content: &str,
    remove_on_success: bool,
    error: std::io::Error,
) -> std::io::Result<()> {
    if matches!(read_ack(ack_path, expected_content), Ok(true)) {
        super::logging::append_log(&format!(
            "warning: Startsignal fehlgeschlagen, aber dauerhafte Bestaetigung ist gueltig; Update bleibt committed: {error}"
        ));
        if remove_on_success {
            cleanup_ack(ack_path);
        }
        Ok(())
    } else {
        fail_after_stopping(child, ack_path, error)
    }
}

fn ack_listener() -> std::io::Result<(std::net::TcpListener, String)> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    Ok((listener, address))
}

fn receive_signal(
    listener: &std::net::TcpListener,
    expected_token: &str,
) -> std::io::Result<Option<std::net::TcpStream>> {
    use std::io::Read;
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut response = [0u8; 32];
        (&stream).read_exact(&mut response)?;
        if response == expected_token.as_bytes() {
            return Ok(Some(stream));
        }
    }
}

fn read_ack(path: &Path, expected_content: &str) -> std::io::Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Startbestaetigung {} ist keine regulaere Datei",
                path.display()
            ),
        ));
    }
    if metadata.len() != expected_content.len() as u64 {
        return Ok(false);
    }
    use std::io::Read;
    let mut token = String::with_capacity(expected_content.len());
    std::fs::File::open(path)?
        .take(expected_content.len() as u64 + 1)
        .read_to_string(&mut token)?;
    if token != expected_content {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Startbestaetigung enthaelt ein fremdes Token",
        ))
    } else {
        Ok(true)
    }
}

fn fail_after_stopping(
    child: &mut std::process::Child,
    ack_path: &Path,
    error: std::io::Error,
) -> std::io::Result<()> {
    stop_and_reap(child);
    cleanup_ack(ack_path);
    Err(error)
}

fn stop_and_reap(child: &mut std::process::Child) {
    if child.try_wait().is_ok_and(|status| status.is_some()) {
        return;
    }
    let _ = child.kill();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) | Err(_) => {
                // A rollback is unsafe until this exact process is known to
                // have exited. Remaining here retains every rollback file.
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn ensure_ack_path_missing(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("Startbestaetigung {} existiert bereits", path.display()),
        )),
        Err(error) => Err(error),
    }
}

fn cleanup_ack(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            super::logging::append_log(&format!(
                "warning: Startbestaetigung {} entfernen: {error}",
                path.display()
            ));
        }
    }
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| std::io::Error::other(format!("Starttoken erzeugen: {error}")))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(token)
}

fn validate_before_spawn(exe: &Path, expected_sha256: &str) -> std::io::Result<()> {
    verify_sha256(exe, expected_sha256).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Programmdatei vor Start revalidieren: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_tokens_are_128_bit_hex() {
        let token = random_token().unwrap();
        assert_eq!(token.len(), 32);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[cfg(unix)]
    #[test]
    fn acknowledged_launch_waits_for_child_response() {
        let shell = Path::new("/bin/bash");
        let hash = super::super::hash::sha256_file(shell).unwrap();
        let script = "printf '%s' \"$SMART_EXPLORER_UPDATE_ACK_TOKEN\" > \"$SMART_EXPLORER_UPDATE_ACK_PATH\"; host=${SMART_EXPLORER_UPDATE_ACK_SIGNAL%:*}; port=${SMART_EXPLORER_UPDATE_ACK_SIGNAL##*:}; exec 3<>\"/dev/tcp/$host/$port\"; printf '%s' \"$SMART_EXPLORER_UPDATE_ACK_TOKEN\" >&3; dd bs=1 count=1 <&3 >/dev/null 2>&1; sleep 0.1";

        spawn_verified_acknowledged_with(shell, &hash, &["-c", script], Duration::from_secs(2))
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn child_exit_without_ack_is_rejected() {
        let shell = Path::new("/bin/bash");
        let hash = super::super::hash::sha256_file(shell).unwrap();

        assert!(spawn_verified_acknowledged_with(
            shell,
            &hash,
            &["-c", "exit 7"],
            Duration::from_secs(2),
        )
        .is_err());
    }
}
