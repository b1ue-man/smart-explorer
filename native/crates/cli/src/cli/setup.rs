use std::io::Read;

use crate::creds::{AuthKind, Protocol, SavedConnection};

#[path = "setup/share_peer.rs"]
mod share_peer;
pub(crate) use share_peer::add_peer;

pub(crate) struct RemoteConnectionInput {
    pub(crate) protocol: Protocol,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) user: String,
    pub(crate) root: String,
    pub(crate) label: String,
    pub(crate) key: Option<String>,
    pub(crate) use_agent: bool,
    /// SFTP: log in with the running SSH agent (`AuthKind::SshAgent`).
    pub(crate) ssh_agent: bool,
    pub(crate) secret: Option<String>,
}

const MAX_STDIN_SECRET_BYTES: u64 = 64 * 1024;

pub(crate) fn read_stdin_secret() -> Result<String, String> {
    read_secret_from(std::io::stdin().lock())
}

fn read_secret_from(reader: impl Read) -> Result<String, String> {
    let mut secret = String::new();
    reader
        .take(MAX_STDIN_SECRET_BYTES + 1)
        .read_to_string(&mut secret)
        .map_err(|e| format!("password stdin: {e}"))?;
    if secret.len() as u64 > MAX_STDIN_SECRET_BYTES {
        return Err(format!(
            "password stdin exceeds the {MAX_STDIN_SECRET_BYTES}-byte limit"
        ));
    }
    Ok(secret.trim_end_matches(['\r', '\n']).to_string())
}

pub(crate) fn add_remote(input: RemoteConnectionInput) -> Result<String, String> {
    super::os::validate_connection_protocol(input.protocol)?;
    if input
        .secret
        .as_ref()
        .is_some_and(|secret| secret.len() as u64 > MAX_STDIN_SECRET_BYTES)
    {
        return Err(format!(
            "password exceeds the {MAX_STDIN_SECRET_BYTES}-byte limit"
        ));
    }
    let conn = build_saved_connection(input)?;
    crate::creds::save_connection_with_secret(&conn.saved, conn.pending_secret.as_deref())
        .map_err(|e| format!("connection speichern: {e}"))?;
    Ok(format!(
        "Saved connection {}\t{}",
        conn.saved.display(),
        conn.saved.account()
    ))
}

/// The saved remote named by its exact label (display name) or account.
pub(crate) fn find_saved_remote(selector: &str) -> Result<SavedConnection, String> {
    let selector = required_selector(selector)?;
    let mut matches = crate::creds::load_connections_checked()?
        .into_iter()
        .filter(|connection| connection.account() == selector || connection.display() == selector)
        .collect::<Vec<_>>();
    matches.sort_by_key(|connection| connection.account());
    matches.dedup_by(|left, right| left.account() == right.account());
    exact_match(matches, &selector, "saved connection", |connection| {
        connection.account()
    })
}

/// The GUI removal (desktop sidebar, Android `conn.delete`): the entry and
/// its secret, then favourites, folder preferences and drives of it; sync
/// setups that use it are kept and named.
pub(crate) fn remove_remote(selector: &str) -> Result<String, String> {
    let connection = find_saved_remote(selector)?;
    let account = connection.account();
    let current = crate::creds::load_connections_checked()?;
    let removal = crate::share::removal::remove_saved_connection(&current, &account)
        .map_err(|error| format!("remove saved connection {account}: {error}"))?;
    let cleanup = removal
        .scope
        .as_ref()
        .map(crate::connect::cleanup_removed_endpoint_state)
        .map(|report| report.summary_suffix())
        .unwrap_or_default();
    Ok(format!("Removed saved connection {account}{cleanup}"))
}

/// Saves a changed remote through the path the GUIs use: a moved entry takes
/// its secret along, a password never becomes a key passphrase.
pub(crate) fn edit_remote(
    previous: &SavedConnection,
    input: RemoteConnectionInput,
) -> Result<String, String> {
    super::os::validate_connection_protocol(input.protocol)?;
    if input
        .secret
        .as_ref()
        .is_some_and(|secret| secret.len() as u64 > MAX_STDIN_SECRET_BYTES)
    {
        return Err(format!(
            "password exceeds the {MAX_STDIN_SECRET_BYTES}-byte limit"
        ));
    }
    let conn = build_saved_connection(input)?;
    let edit = crate::connect::save_edited_connection(
        Some(previous),
        conn.saved,
        conn.pending_secret.as_deref(),
    )
    .map_err(|error| format!("connection speichern: {error}"))?;
    let mut message = format!(
        "Saved connection {}\t{}",
        edit.saved.display(),
        edit.saved.account()
    );
    if let Some(old) = &edit.moved_from {
        message.push_str(&format!("\twas {old}"));
    }
    for warning in &edit.warnings {
        message.push_str(&format!("; warning: {warning}"));
    }
    Ok(message)
}

pub(crate) fn remove_peer(selector: Option<&str>) -> Result<String, String> {
    let selector = selector.map(str::trim).filter(|value| !value.is_empty());
    let profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let matches = profiles
        .direct_contacts
        .iter()
        .filter(|contact| selector.is_none_or(|value| peer_selector_matches(contact, value)))
        .cloned()
        .collect::<Vec<_>>();
    let contact = match matches.as_slice() {
        [] => {
            return Err(match selector {
                Some(value) => format!(
                    "peer not found: {value}; run `se connections list` to see valid selector=<id> values"
                ),
                None => "no Share peers to remove; `se connections list` shows saved peers"
                    .to_string(),
            })
        }
        [contact] => contact.clone(),
        many => {
            return Err(format!(
                "peer selector is ambiguous; choose one selector from `se connections list`: {}",
                many.iter()
                    .map(|contact| contact.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };
    let (_committed, change, forgotten) =
        crate::share::ShareProfiles::forget_direct_peer_persisted(
            Some(default_home()),
            &contact.id,
        )
        .map_err(|error| format!("remove peer {}: {error}", contact.id))?;
    let cleanup = crate::app::cleanup_removed_endpoint_state(
        &crate::app::RemovedEndpointScope::for_direct_contact(&contact.id),
    );
    if let Some(warning) = change.cleanup_warning {
        return Err(format!("removed peer {}, but {warning}", contact.id));
    }
    let details = forgotten
        .map(|peer| {
            format!(
                "; grants={} requests={} legacy={}{}",
                peer.grants_removed,
                peer.requests_removed,
                peer.legacy_requests_removed,
                if peer.identity.is_some() {
                    "; automatic re-pairing blocked until paired again"
                } else {
                    ""
                }
            )
        })
        .unwrap_or_default();
    Ok(format!(
        "Removed peer {}{}{}{}",
        contact.id,
        details,
        cleanup.summary_suffix(),
        worker_refresh_suffix()
    ))
}

fn peer_selector_matches(contact: &crate::share::DirectContact, selector: &str) -> bool {
    let endpoint = format!("share://direct/{}", contact.id);
    contact.display_name.eq_ignore_ascii_case(selector)
        || exact_or_prefix(selector, &contact.id)
        || endpoint.eq_ignore_ascii_case(selector.trim_end_matches('/'))
        || exact_or_prefix(selector, &contact.expected_fingerprint)
        || exact_or_prefix(selector, &contact.expected_node_id)
        || contact
            .remote_device_id
            .as_deref()
            .is_some_and(|device_id| exact_or_prefix(selector, device_id))
}

fn exact_or_prefix(selector: &str, candidate: &str) -> bool {
    candidate.eq_ignore_ascii_case(selector)
        || (selector.len() >= 4
            && candidate
                .to_ascii_lowercase()
                .starts_with(&selector.to_ascii_lowercase()))
}

pub(crate) fn remove_room(selector: &str) -> Result<String, String> {
    let selector = required_selector(selector)?;
    let profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let matches = profiles
        .rooms
        .iter()
        .filter(|room| room.id == selector || room.room_id == selector || room.name == selector)
        .cloned()
        .collect::<Vec<_>>();
    let room = exact_match(matches, &selector, "room", |room| room.id.clone())?;
    let (_committed, change) =
        crate::share::ShareProfiles::remove_room_persisted(Some(default_home()), &room.id)
            .map_err(|error| format!("remove room {}: {error}", room.id))?;
    if let Some(warning) = change.cleanup_warning {
        return Err(format!("removed room {}, but {warning}", room.id));
    }
    Ok(format!(
        "Removed room {}{}",
        room.id,
        worker_refresh_suffix()
    ))
}

fn required_selector(selector: &str) -> Result<String, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        Err("selector must not be empty".to_string())
    } else {
        Ok(selector.to_string())
    }
}

fn exact_match<T>(
    mut matches: Vec<T>,
    selector: &str,
    kind: &str,
    describe: impl Fn(&T) -> String,
) -> Result<T, String> {
    match matches.len() {
        0 => Err(format!("{kind} not found: {selector}")),
        1 => Ok(matches.remove(0)),
        _ => Err(format!(
            "{kind} selector is ambiguous: {}",
            matches.iter().map(describe).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn worker_refresh_suffix() -> String {
    if !crate::support_dirs::app_data_file("share_server.txt").is_file() {
        return "; share worker inactive (no server configured)".to_string();
    }
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => "; share worker refreshed".to_string(),
        Ok(false) => "; share worker is not configured".to_string(),
        Err(error) => format!("; share worker refresh unavailable: {error}"),
    }
}

pub(crate) fn add_room(code: &str, name: &str) -> Result<String, String> {
    let (_profiles, room_id) = crate::share::ShareProfiles::add_room_from_code_persisted(
        Some(default_home()),
        code,
        name,
    )?;
    ensure_share_worker_running()
        .map_err(|e| format!("saved room {room_id}, but share worker was not configured: {e}"))?;
    Ok(format!("Saved room {room_id}; share worker configured"))
}

#[derive(Debug)]
struct PendingSavedConnection {
    saved: SavedConnection,
    pending_secret: Option<String>,
}

fn build_saved_connection(input: RemoteConnectionInput) -> Result<PendingSavedConnection, String> {
    if input.use_agent && input.protocol != Protocol::Sftp {
        return Err("--agent is only supported for sftp connections".into());
    }
    let key = input
        .key
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if key.is_some() && input.protocol != Protocol::Sftp {
        return Err("--key is only supported for sftp connections".into());
    }
    reject_control_characters("user", &input.user)?;
    reject_control_characters("root", &input.root)?;
    reject_control_characters("label", &input.label)?;
    if let Some(host) = input.host.as_deref() {
        reject_control_characters("host", host)?;
    }
    if let Some(key) = key.as_deref() {
        reject_control_characters("key", key)?;
    }
    if input.ssh_agent
        && (input.protocol != Protocol::Sftp || key.is_some() || input.secret.is_some())
    {
        return Err(
            "--ssh-agent is only supported for sftp connections without --key or a password".into(),
        );
    }
    let auth = match key {
        Some(path) => AuthKind::Key { path },
        None if input.ssh_agent => AuthKind::SshAgent,
        None => AuthKind::Password,
    };
    let root = normalize_root(input.protocol, &input.root)?;
    if input.protocol == Protocol::Smb && !crate::smb::root_has_share(&root) {
        return Err("smb root must start with the share: /<share>/<folder>".to_string());
    }
    let host = match input.protocol {
        Protocol::Share => optional_trim(input.host)
            .or_else(|| unc_server(&root))
            .ok_or_else(|| "share root must include a server name".to_string())?,
        _ => required(input.host.unwrap_or_default(), "host")?,
    };
    let port = input.port.unwrap_or_else(|| input.protocol.default_port());
    if input.protocol != Protocol::Share && port == 0 {
        return Err("port must be between 1 and 65535".to_string());
    }
    if input.protocol == Protocol::Share && input.port.is_some() {
        return Err("--port is not supported for Windows UNC shares".to_string());
    }
    let saved = SavedConnection {
        protocol: input.protocol,
        host,
        port,
        user: input.user.trim().to_string(),
        auth,
        root,
        label: input.label.trim().to_string(),
        use_agent: input.use_agent,
    };
    Ok(PendingSavedConnection {
        saved,
        pending_secret: input.secret,
    })
}

fn reject_control_characters(field: &str, value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        Err(format!("{field} must not contain control characters"))
    } else {
        Ok(())
    }
}

fn required(value: String, field: &str) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        Err(format!("{field} is required"))
    } else {
        Ok(value)
    }
}

fn optional_trim(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_root(protocol: Protocol, root: &str) -> Result<String, String> {
    if protocol == Protocol::Share {
        return normalize_share_root(root);
    }
    let root = root.trim().replace('\\', "/");
    Ok(if root.is_empty() {
        "/".to_string()
    } else if root.starts_with('/') {
        root
    } else {
        format!("/{root}")
    })
}

fn normalize_share_root(root: &str) -> Result<String, String> {
    let root = root.trim().replace('/', "\\");
    if crate::net::share_root(&root).is_none() {
        return Err("share root must be a UNC path like \\\\server\\share".into());
    }
    Ok(root)
}

fn ensure_share_worker_running() -> Result<(), String> {
    let running = crate::daemon::refresh_share_worker_checked()?;
    if running {
        Ok(())
    } else {
        Err("Share server is not configured or Auto-Connect is off".into())
    }
}

fn unc_server(root: &str) -> Option<String> {
    root.trim()
        .trim_start_matches(['\\', '/'])
        .split(['\\', '/'])
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
