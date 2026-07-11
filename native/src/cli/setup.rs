use std::io::Read;

use crate::creds::{AuthKind, Protocol, SavedConnection};

pub(crate) struct RemoteConnectionInput {
    pub(crate) protocol: Protocol,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) user: String,
    pub(crate) root: String,
    pub(crate) label: String,
    pub(crate) key: Option<String>,
    pub(crate) use_agent: bool,
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

pub(crate) fn remove_remote(selector: &str) -> Result<String, String> {
    let selector = required_selector(selector)?;
    let mut matches = crate::creds::load_connections_checked()?
        .into_iter()
        .filter(|connection| connection.account() == selector || connection.display() == selector)
        .collect::<Vec<_>>();
    matches.sort_by_key(|connection| connection.account());
    matches.dedup_by(|left, right| left.account() == right.account());
    let connection = exact_match(matches, &selector, "saved connection", |connection| {
        connection.account()
    })?;
    let account = connection.account();
    crate::creds::remove_connection(&account)
        .map_err(|error| format!("remove saved connection {account}: {error}"))?;
    Ok(format!("Removed saved connection {account}"))
}

pub(crate) fn remove_peer(selector: &str) -> Result<String, String> {
    let selector = required_selector(selector)?;
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let matches = profiles
        .direct_contacts
        .iter()
        .filter(|contact| contact.id == selector || contact.display_name == selector)
        .cloned()
        .collect::<Vec<_>>();
    let contact = exact_match(matches, &selector, "peer", |contact| contact.id.clone())?;
    let change = profiles
        .remove_direct_contact(&contact.id)
        .map_err(|error| format!("remove peer {}: {error}", contact.id))?;
    if let Some(warning) = change.cleanup_warning {
        return Err(format!("removed peer {}, but {warning}", contact.id));
    }
    Ok(format!(
        "Removed peer {}{}",
        contact.id,
        worker_refresh_suffix()
    ))
}

pub(crate) fn remove_room(selector: &str) -> Result<String, String> {
    let selector = required_selector(selector)?;
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let matches = profiles
        .rooms
        .iter()
        .filter(|room| room.id == selector || room.room_id == selector || room.name == selector)
        .cloned()
        .collect::<Vec<_>>();
    let room = exact_match(matches, &selector, "room", |room| room.id.clone())?;
    let change = profiles
        .remove_room(&room.id)
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
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => "; share worker refreshed".to_string(),
        Ok(false) => "; share worker is not configured".to_string(),
        Err(error) => format!("; share worker refresh unavailable: {error}"),
    }
}

pub(crate) fn add_peer(code: &str, name: &str, request: bool) -> Result<String, String> {
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let existing = profiles.direct_contact_id_from_code(code)?;
    let (contact_id, created, request_needed) = match existing {
        Some(id) => {
            let request_needed = prepare_direct_request(&mut profiles, &id)?;
            (id, false, request_needed)
        }
        None => {
            let id = profiles.add_direct_from_code(code, name)?;
            let request_needed = prepare_direct_request(&mut profiles, &id)?;
            (id, true, request_needed)
        }
    };
    profiles
        .save()
        .map_err(|e| format!("share profile speichern: {e}"))?;
    if request {
        ensure_share_worker_running().map_err(|e| {
            format!("saved peer contact {contact_id}, but access request was not queued: {e}")
        })?;
        let action = if created { "Saved" } else { "Updated" };
        if !request_needed {
            return Ok(format!(
                "{action} peer contact {contact_id}; already accepted, share worker configured"
            ));
        }
        return Ok(format!(
            "{action} peer contact {contact_id}; access request queued through the share worker"
        ));
    }
    let action = if created { "Saved" } else { "Updated" };
    Ok(format!(
        "{action} peer contact {contact_id}; request not queued (--no-request)"
    ))
}

pub(crate) fn add_room(code: &str, name: &str) -> Result<String, String> {
    let mut profiles = crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profile laden: {error}"))?;
    let room_id = profiles.add_room_from_code(code, name)?;
    profiles
        .save()
        .map_err(|e| format!("share profile speichern: {e}"))?;
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
    let auth = match key {
        Some(path) => AuthKind::Key { path },
        None => AuthKind::Password,
    };
    let root = normalize_root(input.protocol, &input.root)?;
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

fn prepare_direct_request(
    profiles: &mut crate::share::ShareProfiles,
    contact_id: &str,
) -> Result<bool, String> {
    let contact = profiles
        .direct_contacts
        .iter_mut()
        .find(|c| c.id == contact_id)
        .ok_or_else(|| format!("peer contact not found: {contact_id}"))?;
    contact.auto_connect = true;
    contact.auto_open = false;
    if contact.access_state == crate::share::DirectAccessState::Accepted {
        return Ok(false);
    }
    contact.status = crate::share::ShareStatus::WaitingForAccess;
    contact.access_state = crate::share::DirectAccessState::Pending;
    contact.request_sent_at = Some(crate::share::core_now_secs());
    Ok(true)
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
mod tests {
    use crate::creds::{AuthKind, Protocol};

    use super::{
        build_saved_connection, exact_match, normalize_root, read_secret_from,
        RemoteConnectionInput, MAX_STDIN_SECRET_BYTES,
    };

    #[test]
    fn stdin_secret_is_trimmed_and_bounded() {
        assert_eq!(read_secret_from(&b"secret\r\n"[..]).unwrap(), "secret");
        let oversized = vec![b'x'; MAX_STDIN_SECRET_BYTES as usize + 1];
        assert!(read_secret_from(&oversized[..])
            .unwrap_err()
            .contains("byte limit"));
    }

    #[test]
    fn exact_selectors_reject_missing_and_ambiguous_matches() {
        assert!(exact_match(Vec::<String>::new(), "x", "item", Clone::clone).is_err());
        assert_eq!(
            exact_match(vec!["a".to_string()], "a", "item", Clone::clone).unwrap(),
            "a"
        );
        assert!(exact_match(
            vec!["a".to_string(), "b".to_string()],
            "name",
            "item",
            Clone::clone
        )
        .unwrap_err()
        .contains("ambiguous"));
    }

    #[test]
    fn remote_builder_applies_defaults() {
        let built = build_saved_connection(RemoteConnectionInput {
            protocol: Protocol::Sftp,
            host: Some(" example.com ".into()),
            port: None,
            user: " alice ".into(),
            root: "srv".into(),
            label: " prod ".into(),
            key: None,
            use_agent: true,
            secret: Some("pw".into()),
        })
        .unwrap();
        assert_eq!(built.saved.host, "example.com");
        assert_eq!(built.saved.port, 22);
        assert_eq!(built.saved.user, "alice");
        assert_eq!(built.saved.root, "/srv");
        assert_eq!(built.saved.label, "prod");
        assert_eq!(built.saved.auth, AuthKind::Password);
        assert!(built.saved.use_agent);
        assert_eq!(built.pending_secret.as_deref(), Some("pw"));
    }

    #[test]
    fn remote_builder_rejects_non_sftp_key_and_agent() {
        let err = build_saved_connection(RemoteConnectionInput {
            protocol: Protocol::Webdav,
            host: Some("dav.example.com".into()),
            port: None,
            user: String::new(),
            root: "/".into(),
            label: String::new(),
            key: Some("id".into()),
            use_agent: false,
            secret: None,
        })
        .unwrap_err();
        assert!(err.contains("--key"));

        let err = build_saved_connection(RemoteConnectionInput {
            protocol: Protocol::Ftp,
            host: Some("ftp.example.com".into()),
            port: None,
            user: String::new(),
            root: "/".into(),
            label: String::new(),
            key: None,
            use_agent: true,
            secret: None,
        })
        .unwrap_err();
        assert!(err.contains("--agent"));
    }

    #[test]
    fn roots_are_normalized_for_remote_paths() {
        assert_eq!(normalize_root(Protocol::Sftp, "").unwrap(), "/");
        assert_eq!(normalize_root(Protocol::Sftp, "docs").unwrap(), "/docs");
        assert_eq!(
            normalize_root(Protocol::Sftp, r"\docs\team").unwrap(),
            "/docs/team"
        );
        assert_eq!(
            normalize_root(Protocol::Sftp, "/already").unwrap(),
            "/already"
        );
    }

    #[test]
    fn share_connections_preserve_unc_roots_and_derive_host() {
        let built = build_saved_connection(RemoteConnectionInput {
            protocol: Protocol::Share,
            host: None,
            port: None,
            user: "DOMAIN\\alice".into(),
            root: "//srv/pub/team".into(),
            label: "files".into(),
            key: None,
            use_agent: false,
            secret: None,
        })
        .unwrap();
        assert_eq!(built.saved.host, "srv");
        assert_eq!(built.saved.port, 0);
        assert_eq!(built.saved.root, r"\\srv\pub\team");
    }
}
