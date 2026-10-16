pub(super) fn print_connections(json: bool) -> Result<(), String> {
    let (connections, profiles) = load()?;
    if json {
        let rows = rows(&connections, &profiles);
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let mut count = 0usize;
    for connection in connections {
        count += 1;
        println!(
            "{}\t{}\t{}\t{}\tselector={}",
            clean(&connection.display()),
            connection.protocol.as_str(),
            clean(&connection.to_target()),
            if connection.use_agent { "agent" } else { "" },
            clean(&connection.account()),
        );
    }
    if gdrive_value().is_some() {
        count += 1;
        println!("Google Drive\tgdrive\tgdrive:///\t\tselector=gdrive");
    }
    for line in share_text(&profiles) {
        count += 1;
        println!("{line}");
    }
    if count == 0 {
        println!("connections\t0");
    }
    Ok(())
}

/// The rows of `se connections list --json`: saved remotes, Google Drive while
/// an account is signed in, then Share peers.
pub(in crate::cli) fn connections_value() -> Result<serde_json::Value, String> {
    let (connections, profiles) = load()?;
    Ok(serde_json::Value::Array(rows(&connections, &profiles)))
}

fn load() -> Result<
    (
        Vec<crate::creds::SavedConnection>,
        crate::share::ShareProfiles,
    ),
    String,
> {
    let connections = crate::creds::load_connections_checked()?;
    let profiles = crate::share::ShareProfiles::load_checked(
        crate::support_dirs::home_dir().map(|home| home.to_string_lossy().replace('\\', "/")),
    )
    .map_err(|error| format!("share profiles: {error}"))?;
    Ok((connections, profiles))
}

fn rows(
    connections: &[crate::creds::SavedConnection],
    profiles: &crate::share::ShareProfiles,
) -> Vec<serde_json::Value> {
    connections
        .iter()
        .map(saved_value)
        .chain(gdrive_value())
        .chain(share_values(profiles))
        .collect()
}

/// Google Drive as the GUI sidebar lists it, while an account is signed in.
fn gdrive_value() -> Option<serde_json::Value> {
    crate::cloud::is_connected(crate::cloud::Provider::GDrive).then(|| {
        serde_json::json!({
            "selector": "gdrive",
            "label": "Google Drive",
            "account": "gdrive:///",
            "endpoint": "gdrive:///",
            "protocol": "gdrive",
            "kind": "cloud",
            "settings_command": "se connections gdrive status",
            "remove_command": "se connections gdrive sign-out",
        })
    })
}

fn saved_value(connection: &crate::creds::SavedConnection) -> serde_json::Value {
    // The login kind as the Android list shows it (`conn.list`: auth, keyPath).
    let (auth, key_path) = match &connection.auth {
        crate::creds::AuthKind::Password => ("password", None),
        crate::creds::AuthKind::Key { path } => ("key", Some(path.as_str())),
        crate::creds::AuthKind::SshAgent => ("ssh-agent", None),
    };
    serde_json::json!({
        "selector": connection.account(),
        "label": connection.display(),
        "account": connection.account(),
        "protocol": connection.protocol.as_str(),
        "host": connection.host,
        "port": connection.port,
        "user": connection.user,
        "root": connection.root,
        "auth": auth,
        "key_path": key_path,
        "use_agent": connection.use_agent,
        "edit_command": format!("se connections edit {}", connection.account()),
        "remove_command": format!("se connections remove {}", connection.account()),
    })
}

fn share_values<'a>(
    profiles: &'a crate::share::ShareProfiles,
) -> impl Iterator<Item = serde_json::Value> + 'a {
    let direct = profiles.direct_contacts.iter().map(|contact| {
        let endpoint = direct_endpoint(&contact.id);
        serde_json::json!({
            "selector": contact.id,
            "id": contact.id,
            "label": contact.display_name,
            "account": endpoint,
            "endpoint": endpoint,
            "protocol": "share",
            "kind": "direct",
            "status": contact.status.label(),
            "access_state": contact.access_state.label(),
            "fingerprint": contact.expected_fingerprint,
            "device_id": contact.remote_device_id,
            "remove_command": format!("se connections remove-peer {}", contact.id),
        })
    });
    let rooms = profiles.rooms.iter().flat_map(|room| {
        if room.members.is_empty() {
            return vec![serde_json::json!({
                "selector": room.id,
                "id": room.id,
                "label": room.name,
                "account": format!("share://room/{}", room.id),
                "protocol": "share",
                "kind": "room",
                "status": room.status.label(),
                "remove_command": format!("se connections remove-room {}", room.id),
            })];
        }
        room.members
            .iter()
            .map(|member| {
                let endpoint = crate::share::PeerOpenTarget::RoomDevice {
                    room_id: room.id.clone(),
                    device_id: member.device_id.clone(),
                }
                .endpoint_prefix();
                serde_json::json!({
                    "selector": room.id,
                    "label": format!("{}/{}", room.name, member.device_name),
                    "account": endpoint,
                    "endpoint": endpoint,
                    "protocol": "share",
                    "kind": "room-member",
                    "status": member.status.label(),
                    "blocked": member.blocked,
                    "remove_command": format!("se connections remove-room {}", room.id),
                })
            })
            .collect::<Vec<_>>()
    });
    direct.chain(rooms)
}

fn share_text(profiles: &crate::share::ShareProfiles) -> Vec<String> {
    let mut rows = Vec::new();
    for contact in &profiles.direct_contacts {
        let endpoint = direct_endpoint(&contact.id);
        rows.push(format!(
            "{}\tshare\t{}\t{}\tselector={}\tfingerprint={}\tdevice_id={}",
            clean(&contact.display_name),
            endpoint,
            clean(contact.access_state.label()),
            clean(&contact.id),
            clean(&contact.expected_fingerprint),
            contact
                .remote_device_id
                .as_deref()
                .map(clean)
                .unwrap_or_else(|| "-".into()),
        ));
    }
    for room in &profiles.rooms {
        if room.members.is_empty() {
            rows.push(format!(
                "{}\tshare-room\tshare://room/{}\t{}\tselector={}",
                clean(&room.name),
                clean(&room.id),
                clean(&room.status.label()),
                clean(&room.id),
            ));
            continue;
        }
        for member in &room.members {
            let endpoint = crate::share::PeerOpenTarget::RoomDevice {
                room_id: room.id.clone(),
                device_id: member.device_id.clone(),
            }
            .endpoint_prefix();
            rows.push(format!(
                "{}/{}\tshare\t{}\t{}\tselector={}",
                clean(&room.name),
                clean(&member.device_name),
                endpoint,
                clean(&member.status.label()),
                clean(&room.id),
            ));
        }
    }
    rows
}

fn direct_endpoint(contact_id: &str) -> String {
    crate::share::PeerOpenTarget::Direct {
        contact_id: contact_id.to_string(),
    }
    .endpoint_prefix()
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
