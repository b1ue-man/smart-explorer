pub(super) fn print_connections(json: bool) -> Result<(), String> {
    let connections = crate::creds::load_connections_checked()?;
    let profiles = crate::share::ShareProfiles::load_checked(None)
        .map_err(|error| format!("share profiles: {error}"))?;
    if json {
        let rows: Vec<_> = connections
            .iter()
            .map(saved_value)
            .chain(share_values(&profiles))
            .collect();
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
    for line in share_text(&profiles) {
        count += 1;
        println!("{line}");
    }
    if count == 0 {
        println!("connections\t0");
    }
    Ok(())
}

fn saved_value(connection: &crate::creds::SavedConnection) -> serde_json::Value {
    serde_json::json!({
        "selector": connection.account(),
        "label": connection.display(),
        "account": connection.account(),
        "protocol": connection.protocol.as_str(),
        "host": connection.host,
        "port": connection.port,
        "user": connection.user,
        "root": connection.root,
        "use_agent": connection.use_agent,
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
