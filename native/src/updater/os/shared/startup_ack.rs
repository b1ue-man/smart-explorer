use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) const ACK_PATH_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_PATH";
pub(crate) const ACK_TOKEN_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_TOKEN";
pub(crate) const ACK_PAYLOAD_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_PAYLOAD";
pub(crate) const ACK_SIGNAL_ENV: &str = "SMART_EXPLORER_UPDATE_ACK_SIGNAL";
const ACK_PREFIX: &str = "update_start_ack_";
const COMPLETION_SCHEMA: &str = "SMART-EXPLORER-LAUNCH-COMPLETE-V1";

#[derive(Clone)]
struct AckRequest {
    path: PathBuf,
    token: String,
    response: String,
    signal: std::net::SocketAddr,
}

static REQUEST: OnceLock<Option<AckRequest>> = OnceLock::new();
static RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Capture and clear the one-shot challenge before startup can create threads
/// or child processes that would otherwise inherit it.
pub(crate) fn capture_update_startup_ack(just_updated: bool) -> Result<bool, String> {
    let request = requested_acknowledgement(just_updated)?;
    let pending = request.is_some();
    REQUEST
        .set(request)
        .map_err(|_| "Update-Startbestaetigung wurde mehrfach initialisiert".to_string())?;
    Ok(pending)
}

pub(crate) fn update_startup_ack_pending() -> bool {
    REQUEST.get().is_some_and(Option::is_some)
}

/// Confirm that the replacement reached the point where the GUI state was
/// constructed. The updater retains rollback files until this acknowledgement
/// arrives, so merely creating a process is not treated as a successful start.
pub(crate) fn acknowledge_update_startup() -> Result<(), String> {
    RESULT.get_or_init(write_requested_acknowledgement).clone()
}

fn write_requested_acknowledgement() -> Result<(), String> {
    let Some(request) = REQUEST
        .get()
        .ok_or_else(|| "Update-Startbestaetigung wurde nicht initialisiert".to_string())?
    else {
        return Ok(());
    };
    let name = request
        .path
        .file_name()
        .ok_or_else(|| "Update-Startbestaetigung hat keinen Dateinamen".to_string())?
        .to_string_lossy();
    let pending = request
        .path
        .with_file_name(format!(".{name}.ack-pending-{}", request.token));
    let mut file = super::os::create_startup_ack(&pending).map_err(|error| {
        format!(
            "Update-Startbestaetigung {} anlegen: {error}",
            pending.display()
        )
    })?;
    file.write_all(request.response.as_bytes())
        .map_err(|error| format!("Update-Startbestaetigung schreiben: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Update-Startbestaetigung synchronisieren: {error}"))?;
    drop(file);
    super::os::publish_startup_ack(&pending, &request.path).map_err(|error| {
        let _ = std::fs::remove_file(&pending);
        format!("Update-Startbestaetigung veroeffentlichen: {error}")
    })?;
    let commit_result = (|| {
        let mut signal = std::net::TcpStream::connect_timeout(
            &request.signal,
            std::time::Duration::from_secs(5),
        )
        .map_err(|error| format!("Update-Startsignal verbinden: {error}"))?;
        signal
            .set_write_timeout(Some(std::time::Duration::from_secs(5)))
            .and_then(|_| signal.set_read_timeout(Some(std::time::Duration::from_secs(50))))
            .map_err(|error| format!("Update-Startsignal begrenzen: {error}"))?;
        signal
            .write_all(request.token.as_bytes())
            .and_then(|_| signal.flush())
            .map_err(|error| format!("Update-Startsignal senden: {error}"))?;
        let mut committed = [0u8; 1];
        signal
            .read_exact(&mut committed)
            .map_err(|error| format!("Update-Startfreigabe empfangen: {error}"))?;
        (committed == [1])
            .then_some(())
            .ok_or_else(|| "Update-Startfreigabe war ungueltig".to_string())
    })();
    if let Err(error) = commit_result {
        // Publishing the nonce-bound receipt is irrevocable. A live helper
        // will accept it before any rollback; a dead helper cannot roll back.
        eprintln!("Smart Explorer: {error}; dauerhafter Start bleibt gueltig");
    }
    Ok(())
}

fn requested_acknowledgement(just_updated: bool) -> Result<Option<AckRequest>, String> {
    let path = std::env::var_os(ACK_PATH_ENV);
    let token = std::env::var(ACK_TOKEN_ENV).ok();
    let payload = std::env::var(ACK_PAYLOAD_ENV).ok();
    let signal = std::env::var(ACK_SIGNAL_ENV).ok();
    std::env::remove_var(ACK_PATH_ENV);
    std::env::remove_var(ACK_TOKEN_ENV);
    std::env::remove_var(ACK_PAYLOAD_ENV);
    std::env::remove_var(ACK_SIGNAL_ENV);
    let (path, token, signal) = match (path, token, signal, payload.as_ref()) {
        (None, None, None, None) => return Ok(None),
        (Some(path), Some(token), Some(signal), _) => (path, token, signal),
        _ => return Err("Update-Startbestaetigung ist unvollstaendig".to_string()),
    };
    if !just_updated {
        return Err(
            "Update-Startbestaetigung ist nur fuer einen --updated-Start erlaubt".to_string(),
        );
    }
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Update-Startbestaetigung enthaelt ein ungueltiges Token".to_string());
    }
    let signal = signal
        .parse::<std::net::SocketAddr>()
        .map_err(|error| format!("Update-Startsignal ist ungueltig: {error}"))?;
    if !signal.ip().is_loopback() {
        return Err("Update-Startsignal ist nicht an Loopback gebunden".to_string());
    }
    let path = std::path::PathBuf::from(path);
    let response = if let Some(payload) = payload {
        validate_completion_payload(&path, &payload, &token)?;
        payload
    } else {
        let expected = crate::support_dirs::app_data_dir().join(format!("{ACK_PREFIX}{token}"));
        if path != expected {
            return Err(format!(
                "Update-Startbestaetigung liegt ausserhalb des vorgesehenen Pfads: {}",
                path.display()
            ));
        }
        token.clone()
    };
    Ok(Some(AckRequest {
        path,
        token,
        response,
        signal,
    }))
}

fn validate_completion_payload(
    path: &std::path::Path,
    payload: &str,
    token: &str,
) -> Result<(), String> {
    let mut lines = payload.lines();
    let schema = lines.next();
    let target_id = lines.next();
    let version = lines.next();
    let app_sha256 = lines.next();
    let challenge = lines.next();
    let valid_hash = |value: Option<&str>| {
        value.is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    if schema != Some(COMPLETION_SCHEMA)
        || !valid_hash(target_id)
        || version
            .is_none_or(|value| value.is_empty() || value.contains('\r') || value.contains('\n'))
        || !valid_hash(app_sha256)
        || challenge != Some(token)
        || lines.next().is_some()
    {
        return Err("Update-Startabschluss ist ungueltig".to_string());
    }
    let target_id = target_id.unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let appdata = crate::support_dirs::app_data_dir();
    if path.parent() != Some(appdata.as_path())
        || !name.contains(".launch-complete.")
        || !name.ends_with(target_id)
    {
        return Err(format!(
            "Update-Startbestaetigung liegt ausserhalb des vorgesehenen Pfads: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn no_request_is_a_noop() {
        let _guard = env_lock();
        std::env::remove_var(ACK_PATH_ENV);
        std::env::remove_var(ACK_TOKEN_ENV);
        std::env::remove_var(ACK_SIGNAL_ENV);
        assert!(requested_acknowledgement(false).unwrap().is_none());
    }

    #[test]
    fn arbitrary_ack_path_is_rejected() {
        let _guard = env_lock();
        std::env::set_var(ACK_TOKEN_ENV, "a".repeat(32));
        std::env::set_var(ACK_PATH_ENV, std::env::temp_dir().join("outside-ack"));
        std::env::set_var(ACK_SIGNAL_ENV, "127.0.0.1:1");
        assert!(requested_acknowledgement(true).is_err());
        std::env::remove_var(ACK_PATH_ENV);
        std::env::remove_var(ACK_TOKEN_ENV);
    }

    #[test]
    fn valid_request_is_captured_and_removed_from_environment() {
        let _guard = env_lock();
        let token = "c".repeat(32);
        let path = crate::support_dirs::app_data_dir().join(format!("{ACK_PREFIX}{token}"));
        std::env::set_var(ACK_TOKEN_ENV, &token);
        std::env::set_var(ACK_PATH_ENV, &path);
        std::env::set_var(ACK_SIGNAL_ENV, "127.0.0.1:1");

        let request = requested_acknowledgement(true).unwrap().unwrap();

        assert_eq!(request.path, path);
        assert_eq!(request.response, token);
        assert!(std::env::var_os(ACK_PATH_ENV).is_none());
        assert!(std::env::var_os(ACK_TOKEN_ENV).is_none());
        assert!(std::env::var_os(ACK_SIGNAL_ENV).is_none());
    }

    #[test]
    fn ack_request_requires_updated_mode() {
        let _guard = env_lock();
        let token = "b".repeat(32);
        std::env::set_var(ACK_TOKEN_ENV, &token);
        std::env::set_var(
            ACK_PATH_ENV,
            crate::support_dirs::app_data_dir().join(format!("{ACK_PREFIX}{token}")),
        );
        std::env::set_var(ACK_SIGNAL_ENV, "127.0.0.1:1");
        assert!(requested_acknowledgement(false).is_err());
    }

    #[test]
    fn completion_receipt_is_bound_to_appdata_target_and_nonce() {
        let _guard = env_lock();
        let token = "d".repeat(32);
        let target_id = "e".repeat(64);
        let payload = format!(
            "{COMPLETION_SCHEMA}\n{target_id}\n0.5.121\n{}\n{token}\n",
            "f".repeat(64)
        );
        let path = crate::support_dirs::app_data_dir()
            .join(format!(".last.txt.launch-complete.{target_id}"));
        std::env::set_var(ACK_TOKEN_ENV, &token);
        std::env::set_var(ACK_PAYLOAD_ENV, &payload);
        std::env::set_var(ACK_PATH_ENV, &path);
        std::env::set_var(ACK_SIGNAL_ENV, "127.0.0.1:1");

        let request = requested_acknowledgement(true).unwrap().unwrap();

        assert_eq!(request.path, path);
        assert_eq!(request.response, payload);
        assert!(std::env::var_os(ACK_PAYLOAD_ENV).is_none());
    }
}
