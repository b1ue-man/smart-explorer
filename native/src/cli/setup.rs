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

pub(crate) fn read_stdin_secret() -> Result<String, String> {
    let mut secret = String::new();
    std::io::stdin()
        .read_to_string(&mut secret)
        .map_err(|e| format!("password stdin: {e}"))?;
    Ok(secret.trim_end_matches(['\r', '\n']).to_string())
}

pub(crate) fn add_remote(input: RemoteConnectionInput) -> Result<String, String> {
    let conn = build_saved_connection(input)?;
    if let Some(secret) = conn.pending_secret.as_deref().filter(|s| !s.is_empty()) {
        crate::creds::set_secret(&conn.saved.account(), secret)
            .map_err(|e| format!("secret speichern: {e}"))?;
    }
    crate::creds::save_connection(&conn.saved).map_err(|e| format!("connection speichern: {e}"))?;
    Ok(format!(
        "Saved connection {}\t{}",
        conn.saved.display(),
        conn.saved.account()
    ))
}

pub(crate) fn add_peer(code: &str, name: &str, request: bool) -> Result<String, String> {
    let mut profiles = crate::share::ShareProfiles::load(Some(default_home()));
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
    let mut profiles = crate::share::ShareProfiles::load(Some(default_home()));
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
    let saved = SavedConnection {
        protocol: input.protocol,
        host,
        port: input.port.unwrap_or_else(|| input.protocol.default_port()),
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

    use super::{build_saved_connection, normalize_root, RemoteConnectionInput};

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
