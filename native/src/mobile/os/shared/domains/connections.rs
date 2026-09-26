//! `conn.*`: saved SFTP/FTP/FTPS/WebDAV connections in the desktop store
//! (`creds`), tested and removed through the desktop helpers. The connection
//! id is the desktop account key (`SavedConnection::account`).
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{invalid, opt_bool, opt_i64, opt_str, str_arg};
use super::locations::forget_endpoint;
use crate::connect::{ConnectForm, ConnectResult};
use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::mobile::{ApiError, Runtime};

/// Longest wait for a connection test (SFTP agent deployment included).
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn connection_json(connection: &SavedConnection) -> Value {
    let (auth, key_path) = match &connection.auth {
        AuthKind::Password => ("password", None),
        AuthKind::Key { path } => ("key", Some(path.clone())),
    };
    json!({
        "id": connection.account(),
        "label": connection.display(),
        "protocol": connection.protocol.as_str(),
        "host": connection.host,
        "port": connection.port,
        "user": connection.user,
        "root": connection.root,
        "auth": auth,
        "keyPath": key_path,
        "useAgent": connection.use_agent,
        // The desktop connects to WebDAV over HTTPS only.
        "https": connection.protocol == Protocol::Webdav,
        "location": connection.to_target(),
    })
}

fn load_all() -> Result<Vec<SavedConnection>, ApiError> {
    crate::creds::load_connections_checked().map_err(|error| ApiError::new("internal", error))
}

pub(super) fn find(id: &str) -> Result<SavedConnection, ApiError> {
    load_all()?
        .into_iter()
        .find(|connection| connection.account() == id)
        .ok_or_else(|| ApiError::new("not_found", "Verbindung nicht gefunden."))
}

pub(super) fn list() -> Result<Value, ApiError> {
    let list: Vec<Value> = load_all()?
        .iter()
        .filter(|connection| connection.protocol.is_url())
        .map(connection_json)
        .collect();
    Ok(Value::Array(list))
}

/// A validated `ConnectionInput`: the desktop connect form plus the secret
/// the input carries (empty = none given).
struct Input {
    form: ConnectForm,
    port: u16,
    secret: String,
    id: Option<String>,
}

fn clean(value: &str, label: &str) -> Result<String, ApiError> {
    if value.chars().any(char::is_control) {
        return Err(invalid(format!("{label} enthält Steuerzeichen.")));
    }
    Ok(value.trim().to_string())
}

fn parse_input(args: &Value) -> Result<Input, ApiError> {
    let input = args
        .get("input")
        .filter(|input| input.is_object())
        .ok_or_else(|| invalid("Parameter „input“ fehlt."))?;
    let protocol = match opt_str(input, "protocol").and_then(Protocol::parse) {
        Some(protocol) if protocol.is_url() => protocol,
        _ => return Err(invalid("Unbekannter Verbindungstyp.")),
    };
    let host = clean(opt_str(input, "host").unwrap_or(""), "Host")?;
    if host.is_empty() {
        return Err(invalid("Host fehlt."));
    }
    let port = match opt_i64(input, "port").unwrap_or(0) {
        0 => protocol.default_port(),
        port => u16::try_from(port)
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| invalid("Ungültiger Port (erwartet: 1–65535)"))?,
    };
    let use_key = opt_str(input, "auth") == Some("key");
    if use_key && protocol != Protocol::Sftp {
        return Err(invalid("Schlüsseldateien gibt es nur bei SFTP."));
    }
    let key_path = clean(opt_str(input, "keyPath").unwrap_or(""), "Schlüsseldatei")?;
    if use_key && key_path.is_empty() {
        return Err(invalid("Bitte eine Schlüsseldatei wählen."));
    }
    if protocol == Protocol::Webdav && !opt_bool(input, "https", true) {
        return Err(ApiError::new(
            "unsupported",
            "WebDAV wird wie am Desktop nur über HTTPS verbunden.",
        ));
    }
    let root = clean(opt_str(input, "root").unwrap_or("/"), "Startordner")?;
    let secret = if use_key {
        opt_str(input, "passphrase")
    } else {
        opt_str(input, "password")
    }
    .unwrap_or("")
    .to_string();
    let form = ConnectForm {
        protocol,
        host,
        port: port.to_string(),
        user: clean(opt_str(input, "user").unwrap_or(""), "Benutzer")?,
        password: if use_key {
            String::new()
        } else {
            secret.clone()
        },
        use_key,
        keyfile: key_path,
        passphrase: if use_key {
            secret.clone()
        } else {
            String::new()
        },
        root: if root.is_empty() { "/".into() } else { root },
        unc: String::new(),
        save: false,
        label: clean(opt_str(input, "label").unwrap_or(""), "Name")?,
        use_agent: protocol == Protocol::Sftp && opt_bool(input, "useAgent", false),
    };
    let id = opt_str(input, "id")
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    Ok(Input {
        form,
        port,
        secret,
        id,
    })
}

/// Password and key passphrase are different secrets; a key file that only
/// moved keeps its passphrase.
fn same_secret_kind(left: &AuthKind, right: &AuthKind) -> bool {
    matches!(
        (left, right),
        (AuthKind::Password, AuthKind::Password) | (AuthKind::Key { .. }, AuthKind::Key { .. })
    )
}

/// The secret stored for `id`, if it is the kind of secret `auth` needs.
fn stored_secret(id: &str, auth: &AuthKind) -> Result<Option<String>, ApiError> {
    let same_kind = load_all()?
        .iter()
        .find(|connection| connection.account() == id)
        .is_some_and(|previous| same_secret_kind(&previous.auth, auth));
    if !same_kind {
        // A password must never be used as a key passphrase or vice versa.
        return Ok(None);
    }
    crate::creds::get_secret_checked(id).map_err(|error| ApiError::new("internal", error))
}

pub(super) fn test(args: &Value) -> Result<Value, ApiError> {
    let input = parse_input(args)?;
    let secret = if !input.secret.is_empty() {
        Some(input.secret.clone())
    } else if let Some(id) = &input.id {
        let candidate = crate::connect::build_saved(&input.form, input.port);
        stored_secret(id, &candidate.auth)?
    } else {
        None
    };
    let receiver = crate::connect::spawn_connect(input.form, secret)
        .map_err(|error| ApiError::new("internal", error))?;
    match receiver.recv_timeout(TEST_TIMEOUT) {
        Ok(ConnectResult::Ok(connected)) => Ok(json!({
            "message": format!("Verbindung OK ({})", connected.label),
        })),
        Ok(ConnectResult::Err(error)) => Err(ApiError::connection(error)),
        Err(_) => Err(ApiError::new(
            "network",
            "Zeitüberschreitung beim Verbindungstest.",
        )),
    }
}

pub(super) fn save(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let input = parse_input(args)?;
    let saved = crate::connect::build_saved(&input.form, input.port);
    let account = saved.account();
    let previous = match &input.id {
        Some(id) => Some(find(id)?),
        None => None,
    };
    let old_account = previous.as_ref().map(SavedConnection::account);
    let moved = old_account.as_ref().is_some_and(|old| *old != account);
    let kind_changed = previous
        .as_ref()
        .is_some_and(|previous| !same_secret_kind(&previous.auth, &saved.auth));
    // An empty secret keeps the stored one; a moved entry takes it along.
    let secret = if !input.secret.is_empty() {
        Some(input.secret.clone())
    } else if moved && !kind_changed {
        match &old_account {
            Some(old) => crate::creds::get_secret_checked(old)
                .map_err(|error| ApiError::new("internal", error))?,
            None => None,
        }
    } else {
        None
    };
    crate::creds::save_connection_with_secret(&saved, secret.as_deref())
        .map_err(|error| ApiError::new("internal", error))?;
    if kind_changed && input.secret.is_empty() {
        // A password must never be used as a key passphrase or vice versa.
        if let Err(error) = crate::creds::delete_secret_checked(&account) {
            rt.log_error("conn.save", &error);
        }
    }
    if moved {
        if let Some(old) = &old_account {
            if let Err(error) = crate::creds::remove_connection(old) {
                rt.log_error(
                    "conn.save",
                    &format!("Alte Verbindung nicht entfernt: {error}"),
                );
            }
        }
    }
    if let Some(previous) = &previous {
        // Open sessions still use the old settings or credentials.
        let scope = crate::connect::RemovedEndpointScope::for_saved_connection(previous);
        rt.drop_backends(&|key| scope.matches_key(key));
    }
    Ok(connection_json(&saved))
}

/// Desktop removal: the connection and its secret, then favourites, folder
/// preferences and mounts of it; sync jobs that use it are only reported.
pub(super) fn delete(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "id")?;
    let current = load_all()?;
    if !current.iter().any(|connection| connection.account() == id) {
        return Err(ApiError::new("not_found", "Verbindung nicht gefunden."));
    }
    let removal = crate::share::removal::remove_saved_connection(&current, id)
        .map_err(|error| ApiError::new("internal", error))?;
    let report = match &removal.scope {
        Some(scope) => {
            forget_endpoint(rt, scope);
            crate::connect::cleanup_removed_endpoint_state(scope)
        }
        None => crate::connect::CleanupReport::default(),
    };
    let notice = crate::share::removal::cleanup_notice(removal.headline, &report);
    if let Some(error) = &notice.error {
        rt.log_error("conn.delete", error);
    }
    Ok(json!({
        "removedFavorites": report.favorites_removed,
        "orphanedJobs": report.orphaned_sync_jobs,
    }))
}
