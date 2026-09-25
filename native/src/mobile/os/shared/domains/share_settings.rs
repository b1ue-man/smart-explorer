//! `share.watch/setServer/setOnline/setName` and the discovery (PIN pairing)
//! actions, through the desktop `discovery_events` dispatcher.
use std::io::Write;

use serde_json::{json, Value};

use super::args::{bool_arg, i64_arg, invalid, str_arg};
use super::share_state::{
    committed, default_home, identity, reconfigure, server_path, set_cached_server, set_watch,
    wake, with_state,
};
use crate::mobile::{ApiError, Runtime};
use crate::share::discovery_events::{dispatch_discovery_ui_action, DiscoveryRepaint};
use crate::share::discovery_state::{DiscoveryPublishTarget, DiscoveryUiAction};
use crate::share::{DiscoveryPin, ShareCmd, ShareProfiles, DISCOVERY_PIN_MAX_BYTES};

const MAX_SERVER_BYTES: usize = 16 * 1024;

pub(super) fn watch(args: &Value) -> Result<Value, ApiError> {
    set_watch(bool_arg(args, "active")?);
    Ok(json!({}))
}

/// The desktop server rules: endpoints separated by `,`/`;`, no whitespace or
/// user information, schemes tcp/ws/wss/http/https.
pub(super) fn validate_server(server: &str) -> Result<String, String> {
    let server = server.trim();
    if server.len() > MAX_SERVER_BYTES || server.chars().any(char::is_control) {
        return Err("Die Share-Server-Adresse ist ungültig oder zu lang.".to_string());
    }
    let mut found = false;
    for endpoint in server.split([',', ';']).map(str::trim) {
        if endpoint.is_empty() {
            continue;
        }
        found = true;
        if endpoint.chars().any(char::is_whitespace) || endpoint.contains('@') {
            return Err(
                "Die Share-Server-Adresse darf keine Leerzeichen oder Zugangsdaten enthalten."
                    .to_string(),
            );
        }
        if let Some((scheme, _)) = endpoint.split_once("://") {
            if !matches!(scheme, "tcp" | "ws" | "wss" | "http" | "https") {
                return Err(format!("Nicht unterstütztes Share-Server-Schema: {scheme}"));
            }
        }
    }
    if !found {
        return Err("Die Share-Server-Adresse enthält keinen Endpunkt.".to_string());
    }
    Ok(server.to_string())
}

fn write_atomic(path: &std::path::Path, value: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(value.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

/// Empty removes the server: the worker then only runs for LAN peers.
pub(super) fn set_server(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let raw = str_arg(args, "server")?;
    let path = server_path();
    let server = if raw.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(super::args::io_error("Share-Server entfernen", error)),
        }
        String::new()
    } else {
        let server = validate_server(raw).map_err(invalid)?;
        write_atomic(&path, &server)
            .map_err(|error| super::args::io_error("Share-Server speichern", error))?;
        server
    };
    set_cached_server(server);
    reconfigure(rt);
    Ok(json!({}))
}

fn set_auto_connect(online: bool) -> Result<(), ApiError> {
    let profiles = ShareProfiles::mutate_persisted(default_home(), |profiles| {
        profiles.auto_connect = online;
        Ok(())
    })
    .map_err(|error| ApiError::new("internal", format!("Share-Profile speichern: {error}")))?;
    committed(profiles);
    Ok(())
}

/// Desktop connect/disconnect: the persisted `auto_connect` flag plus a
/// worker reload (online) or a stop command (offline).
pub(super) fn set_online(args: &Value) -> Result<Value, ApiError> {
    if bool_arg(args, "online")? {
        identity()?;
        set_auto_connect(true)?;
        let active = crate::daemon::refresh_share_worker_checked().map_err(|error| {
            ApiError::new(
                "network",
                format!("Share-Worker konnte nicht aktiviert werden: {error}"),
            )
        })?;
        if !active {
            with_state(|state| {
                state.notice(
                    "Share-Worker wurde nicht aktiv (kein Share-Server und keine gekoppelten Geräte im LAN)",
                )
            });
        }
    } else {
        set_auto_connect(false)?;
        crate::daemon::send_share_command(ShareCmd::Stop).map_err(|error| {
            ApiError::new(
                "internal",
                format!("Share-Worker Stop fehlgeschlagen: {error}"),
            )
        })?;
        with_state(|state| state.notice("Getrennt"));
    }
    wake();
    Ok(json!({}))
}

pub(super) fn set_name(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let name = str_arg(args, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(invalid("Der Gerätename darf nicht leer sein."));
    }
    let mut current = identity()?;
    current
        .set_device_name(name)
        .map_err(|error| ApiError::new("invalid", error))?;
    with_state(|state| state.identity = Some(current));
    reconfigure(rt);
    Ok(json!({}))
}

fn repaint() -> DiscoveryRepaint {
    Box::new(wake)
}

fn check_pin(pin: &str) -> Result<(), ApiError> {
    if pin.len() > DISCOVERY_PIN_MAX_BYTES {
        return Err(invalid(format!(
            "PIN ist {} Bytes lang; maximal {DISCOVERY_PIN_MAX_BYTES} Bytes sind erlaubt",
            pin.len()
        )));
    }
    Ok(())
}

/// Queues the action; queue and command failures appear as notices.
fn run_action(action: DiscoveryUiAction) {
    with_state(|state| {
        dispatch_discovery_ui_action(&mut state.discovery, action, repaint());
        state.after_discovery_changes();
    });
    wake();
}

pub(super) fn discoverable(args: &Value) -> Result<Value, ApiError> {
    let target_key = str_arg(args, "target")?.to_string();
    let alias = str_arg(args, "alias")?.trim().to_string();
    let pin = str_arg(args, "pin")?.to_string();
    let minutes = i64_arg(args, "minutes")?;
    let minutes = u64::try_from(minutes)
        .ok()
        .filter(|minutes| *minutes > 0)
        .ok_or_else(|| invalid("Die Sichtbarkeitsdauer muss positiv sein."))?;
    check_pin(&pin)?;
    let target = if target_key == "direct" {
        DiscoveryPublishTarget::Direct
    } else {
        let profiles = ShareProfiles::load_checked(default_home())
            .map_err(|error| ApiError::new("internal", error))?;
        let room = profiles
            .rooms
            .iter()
            .find(|room| room.id == target_key)
            .ok_or_else(|| ApiError::new("not_found", "Raum nicht gefunden."))?;
        DiscoveryPublishTarget::Room {
            room_id: room.id.clone(),
            room_name: room.name.clone(),
        }
    };
    let busy = with_state(|state| {
        let busy = state.discovery.offer_for_target(&target).is_some();
        if !busy {
            state
                .offer_aliases
                .insert(target_key.clone(), alias.clone());
        }
        busy
    });
    if busy {
        return Err(ApiError::new(
            "conflict",
            "Dieses Ziel ist bereits suchbar – zuerst beenden.",
        ));
    }
    run_action(DiscoveryUiAction::Publish {
        target,
        display_alias: alias,
        pin: DiscoveryPin::new(pin),
        duration_secs: minutes.saturating_mul(60),
    });
    Ok(json!({}))
}

pub(super) fn stop_discoverable(args: &Value) -> Result<Value, ApiError> {
    run_action(DiscoveryUiAction::Stop {
        offer_id: str_arg(args, "offerId")?.to_string(),
    });
    Ok(json!({}))
}

pub(super) fn discover() -> Result<Value, ApiError> {
    run_action(DiscoveryUiAction::Refresh);
    Ok(json!({}))
}

pub(super) fn connect(args: &Value) -> Result<Value, ApiError> {
    let discovery_id = str_arg(args, "discoveryId")?.to_string();
    let pin = str_arg(args, "pin")?.to_string();
    check_pin(&pin)?;
    let pending = with_state(|state| {
        state.discovery.starting(&discovery_id)
            || state
                .discovery
                .exchange_for_discovery(&discovery_id)
                .is_some_and(|(_, exchange)| exchange.state.is_pending())
    });
    if pending {
        return Err(ApiError::new(
            "busy",
            "Für dieses Gerät läuft bereits eine Verbindung.",
        ));
    }
    run_action(DiscoveryUiAction::Connect {
        discovery_id,
        pin: DiscoveryPin::new(pin),
    });
    Ok(json!({}))
}

pub(super) fn cancel_connect(args: &Value) -> Result<Value, ApiError> {
    run_action(DiscoveryUiAction::Cancel {
        exchange_id: str_arg(args, "exchangeId")?.to_string(),
    });
    Ok(json!({}))
}
