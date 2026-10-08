//! `share.watch/setServer/serverInfo/setOnline/setName` and the discovery
//! (PIN pairing) actions, through the desktop `discovery_events` dispatcher.
use std::io::Write;

use serde_json::{json, Value};

use super::args::{bool_arg, i64_arg, invalid, opt_bool, str_arg};
use super::share_state::{
    committed, default_home, identity, reconfigure, server_path, set_cached_server, set_watch,
    wake, with_state,
};
use crate::mobile::{ApiError, Runtime};
use crate::share::discovery_events::{dispatch_discovery_ui_action, DiscoveryRepaint};
use crate::share::discovery_state::{DiscoveryPublishTarget, DiscoveryUiAction};
use crate::share::server_address::SignalServerConfig;
use crate::share::{
    DiscoveryPin, DiscoveryRelationOutcome, ShareCmd, ShareProfiles, DISCOVERY_MAX_OFFER_SECS,
    DISCOVERY_PIN_MAX_BYTES,
};

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
            if !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "tcp" | "ws" | "wss" | "http" | "https"
            ) {
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

/// Empty removes the server: the worker then only runs for LAN peers. An
/// address without scheme means TLS; `tcp://`/`ws://` need `allowPlaintext`
/// (FC3). Answers like `share.serverInfo`.
pub(super) fn set_server(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let raw = str_arg(args, "server")?;
    let allow_plaintext = opt_bool(args, "allowPlaintext", false);
    let path = server_path();
    let config = if raw.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(super::args::io_error("Share-Server entfernen", error)),
        }
        SignalServerConfig::default()
    } else {
        validate_server(raw).map_err(invalid)?;
        let config = SignalServerConfig::parse_input(raw, allow_plaintext).map_err(invalid)?;
        write_atomic(&path, &config.canonical())
            .map_err(|error| super::args::io_error("Share-Server speichern", error))?;
        config
    };
    set_cached_server(config.canonical());
    reconfigure(rt);
    Ok(server_json(&config, false))
}

/// The stored address with its transport security; a legacy value without
/// scheme is rewritten as `tcp://host:port` first (B21).
pub(super) fn server_info() -> Result<Value, ApiError> {
    let path = server_path();
    let migrated = crate::share::migrate_server_file(&path)
        .map_err(|error| super::args::io_error("Share-Server-Adresse umschreiben", error))?;
    if let Some(canonical) = &migrated {
        set_cached_server(canonical.clone());
    }
    let stored = match std::fs::read_to_string(&path) {
        Ok(stored) => stored,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(super::args::io_error("Share-Server lesen", error)),
    };
    let config = SignalServerConfig::parse_stored(&stored).map_err(invalid)?;
    Ok(server_json(&config, migrated.is_some()))
}

fn server_json(config: &SignalServerConfig, migrated: bool) -> Value {
    json!({
        "server": config.canonical(),
        "security": config.security().wire(),
        "summary": config.summary(),
        "plaintext": config.endpoints().iter().any(|endpoint| !endpoint.is_encrypted()),
        "ignoredPlaintext": config.ignored_plaintext(),
        "migrated": migrated,
        "encryptedAlternative": config.encrypted_alternative(),
        "namesIpAddress": config.names_ip_address(),
    })
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
    if pin.is_empty() {
        return Err(invalid("Eine leere PIN ist nicht erlaubt."));
    }
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
    let allow_weak_pin = opt_bool(args, "allowWeakPin", false);
    let minutes = i64_arg(args, "minutes")?;
    let minutes = u64::try_from(minutes)
        .ok()
        .filter(|minutes| *minutes > 0)
        .ok_or_else(|| invalid("Die Sichtbarkeitsdauer muss positiv sein."))?;
    if minutes.saturating_mul(60) > DISCOVERY_MAX_OFFER_SECS {
        return Err(invalid(format!(
            "Die Sichtbarkeit dauert höchstens {} Minuten.",
            DISCOVERY_MAX_OFFER_SECS / 60
        )));
    }
    check_pin(&pin)?;
    if !allow_weak_pin {
        if let Some(problem) = crate::share::discovery_pin_strength(pin.as_bytes()).problem() {
            return Err(ApiError::new("weak_pin", problem));
        }
    }
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
        allow_weak_pin,
    });
    Ok(json!({}))
}

/// A random six-digit PIN for the next offer (FC2).
pub(super) fn suggest_pin() -> Result<Value, ApiError> {
    let pin =
        crate::share::suggest_discovery_pin().map_err(|error| ApiError::new("internal", error))?;
    Ok(json!({ "pin": pin }))
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
        share_back: opt_bool(args, "shareBack", false),
    });
    Ok(json!({}))
}

/// Pairings installed without the other side's confirmation (S24).
pub(super) fn unconfirmed_pairings() -> Result<Value, ApiError> {
    let pairings: Vec<Value> = with_state(|state| {
        state
            .discovery
            .unconfirmed
            .iter()
            .map(|(exchange_id, outcome)| unconfirmed_json(exchange_id, outcome))
            .collect()
    });
    Ok(json!({ "pairings": pairings }))
}

fn unconfirmed_json(exchange_id: &str, outcome: &DiscoveryRelationOutcome) -> Value {
    let (kind, contact_id, room_profile_id) = match outcome {
        DiscoveryRelationOutcome::DirectInstalled { contact_id, .. } => {
            ("direct", Some(contact_id), None)
        }
        DiscoveryRelationOutcome::RoomInstalled {
            room_profile_id, ..
        } => ("roomInstalled", None, Some(room_profile_id)),
        DiscoveryRelationOutcome::RoomShared {
            room_profile_id, ..
        } => ("roomShared", None, Some(room_profile_id)),
    };
    json!({
        "exchangeId": exchange_id,
        "kind": kind,
        "contactId": contact_id,
        "roomProfileId": room_profile_id,
        "label": crate::share::discovery_state::unconfirmed_label(outcome),
        "revocable": crate::share::discovery_state::revocable(outcome),
    })
}

/// "Widerrufen" (`revoke: true`) removes the installed contact or room like
/// "Entfernen"; "Behalten" only drops the notice.
pub(super) fn resolve_pairing(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let exchange_id = str_arg(args, "exchangeId")?;
    let revoke = bool_arg(args, "revoke")?;
    let outcome = with_state(|state| state.discovery.unconfirmed.get(exchange_id).cloned())
        .ok_or_else(|| ApiError::new("not_found", "Kopplung nicht gefunden."))?;
    if !revoke {
        with_state(|state| state.discovery.take_unconfirmed(exchange_id));
        wake();
        return Ok(json!({}));
    }
    let result = match outcome {
        DiscoveryRelationOutcome::DirectInstalled { contact_id, .. } => {
            super::share_peers::remove_device(rt, &json!({ "contactId": contact_id }))
        }
        DiscoveryRelationOutcome::RoomInstalled {
            room_profile_id, ..
        } => super::share_peers::remove_room(rt, &json!({ "profileId": room_profile_id })),
        DiscoveryRelationOutcome::RoomShared { .. } => Err(invalid(
            "Übergebene Raumdaten lassen sich nicht zurückholen; den Raum bei Bedarf neu anlegen.",
        )),
    };
    if result.is_ok() {
        with_state(|state| state.discovery.take_unconfirmed(exchange_id));
        wake();
    }
    result
}

pub(super) fn cancel_connect(args: &Value) -> Result<Value, ApiError> {
    run_action(DiscoveryUiAction::Cancel {
        exchange_id: str_arg(args, "exchangeId")?.to_string(),
    });
    Ok(json!({}))
}
