//! `loc.*` (api.md §4.2): storage volumes, favourites in the desktop format
//! (`favorites.txt`, `location_key`), recently visited locations
//! (`<data>/mobile/recent.json`, newest first) and the other sidebar roots.
use super::args::str_arg;
use super::error::ApiError;
use super::location::{Loc, TRASH_LOCATION};
use super::runtime::{lock, Runtime};
use super::store::{read_json, write_json};
use serde_json::{json, Value};
use std::path::PathBuf;

const MAX_RECENT: usize = 10;

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "loc.roots" => Ok(roots(rt)),
        "loc.toggleFavorite" => toggle_favorite(rt, args),
        "loc.isFavorite" => is_favorite(args),
        _ => return None,
    })
}

fn root(id: String, label: &str, subtitle: Option<&str>, location: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "label": label,
        "subtitle": subtitle,
        "location": location,
        "kind": kind,
        "removable": false,
    })
}

/// A sidebar label for a location: its last segment, else the whole text.
fn short_label(location: &str) -> String {
    match Loc::parse(location) {
        Ok(loc) if !loc.is_root() => loc.name().to_string(),
        Ok(loc) => super::crumbs::title(&loc, &[]),
        Err(_) => location.to_string(),
    }
}

fn roots(rt: &Runtime) -> Value {
    let storage: Vec<Value> = rt
        .volumes()
        .iter()
        .map(|volume| {
            let label = if volume.label.trim().is_empty() {
                volume.path.as_str()
            } else {
                volume.label.as_str()
            };
            let mut value = root(
                format!("storage:{}", volume.path),
                label,
                Some(volume.path.as_str()),
                &volume.path,
                "storage",
            );
            value["removable"] = json!(volume.removable);
            value
        })
        .collect();
    let favorites: Vec<Value> = favorite_locations()
        .iter()
        .map(|location| {
            root(
                format!("favorite:{location}"),
                &short_label(location),
                Some(location.as_str()),
                location,
                "favorite",
            )
        })
        .collect();
    let recent: Vec<Value> = load_recent(rt)
        .iter()
        .map(|location| {
            root(
                format!("recent:{location}"),
                &short_label(location),
                Some(location.as_str()),
                location,
                "recent",
            )
        })
        .collect();
    let connections: Vec<Value> = crate::creds::load_connections()
        .iter()
        .filter(|connection| connection.protocol.is_url())
        .map(|connection| {
            let authority = format!("{}@{}", connection.user, connection.host);
            let label = if connection.label.trim().is_empty() {
                authority.as_str()
            } else {
                connection.label.as_str()
            };
            let subtitle = format!(
                "{} · {}",
                connection.protocol.as_str().to_uppercase(),
                authority
            );
            root(
                connection.account(),
                label,
                Some(subtitle.as_str()),
                &crate::connect::remote_endpoint(connection, &connection.root),
                "connection",
            )
        })
        .collect();
    let (devices, rooms) = share_roots(rt);
    json!({
        "storage": storage,
        "favorites": favorites,
        "recent": recent,
        "connections": connections,
        "gdrive": gdrive_root(rt),
        "devices": devices,
        "rooms": rooms,
        "trash": root("trash".into(), "Papierkorb", None, TRASH_LOCATION, "trash"),
    })
}

/// Google Drive when signed in (`gdrive.status` of the domain facade).
fn gdrive_root(rt: &Runtime) -> Value {
    let signed_in = matches!(
        super::domains::dispatch(rt, "gdrive.status", &json!({})),
        Some(Ok(status)) if status.get("signedIn").and_then(Value::as_bool) == Some(true)
    );
    if signed_in {
        root(
            "gdrive".into(),
            "Google Drive",
            None,
            "gdrive:///",
            "gdrive",
        )
    } else {
        Value::Null
    }
}

/// Direct devices and room member devices from the last `share.status`.
fn share_roots(rt: &Runtime) -> (Vec<Value>, Vec<Value>) {
    let Some(Ok(status)) = super::domains::dispatch(rt, "share.status", &json!({})) else {
        return (Vec::new(), Vec::new());
    };
    let text =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let devices = status
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let location = text(device, "location").filter(|location| !location.is_empty())?;
            let name = text(device, "name").unwrap_or_else(|| location.clone());
            Some(root(
                format!("device:{}", text(device, "contactId").unwrap_or_default()),
                &name,
                text(device, "statusText").as_deref(),
                &location,
                "device",
            ))
        })
        .collect();
    let mut rooms = Vec::new();
    for room in status
        .get("rooms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let room_name = text(room, "name").unwrap_or_default();
        let room_id = text(room, "roomId").unwrap_or_default();
        for member in room
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(location) = text(member, "location").filter(|location| !location.is_empty())
            else {
                continue;
            };
            let name = text(member, "name").unwrap_or_else(|| location.clone());
            rooms.push(root(
                format!(
                    "room:{room_id}/{}",
                    text(member, "deviceId").unwrap_or_default()
                ),
                &name,
                Some(room_name.as_str()),
                &location,
                "room",
            ));
        }
    }
    (devices, rooms)
}

/// Favourites as locations (desktop keys are locations without a trailing
/// slash; unusable lines are skipped).
fn favorite_locations() -> Vec<String> {
    crate::connect::load_favorites()
        .into_iter()
        .filter_map(|key| {
            let loc = Loc::parse(&key).ok()?;
            (!loc.is_app_internal()).then(|| loc.location())
        })
        .collect()
}

fn favorite_loc(args: &Value) -> Result<Loc, ApiError> {
    let location = str_arg(args, "location")?;
    let loc = Loc::parse(location)?;
    if loc.is_app_internal() {
        return Err(ApiError::invalid(
            "Archive und der Papierkorb können keine Favoriten sein",
        ));
    }
    Ok(loc)
}

fn toggle_favorite(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let key = favorite_loc(args)?.favorite_key();
    let _serialized = lock(&rt.inner.favorites_lock);
    let mut favorites = crate::connect::load_favorites();
    let favorite = if let Some(index) = favorites.iter().position(|item| *item == key) {
        favorites.remove(index);
        false
    } else {
        favorites.insert(0, key);
        true
    };
    crate::connect::save_favorites(&favorites)
        .map_err(|error| ApiError::from(error).context("Favoriten speichern"))?;
    Ok(json!({ "favorite": favorite }))
}

fn is_favorite(args: &Value) -> Result<Value, ApiError> {
    let key = favorite_loc(args)?.favorite_key();
    let favorite = crate::connect::load_favorites().contains(&key);
    Ok(json!({ "favorite": favorite }))
}

fn recent_path(rt: &Runtime) -> PathBuf {
    rt.config().mobile_dir().join("recent.json")
}

fn load_recent(rt: &Runtime) -> Vec<String> {
    read_json::<Vec<String>>(&recent_path(rt)).unwrap_or_default()
}

/// Puts a visited location at the top of „Zuletzt“ (app-internal ones never).
pub(crate) fn add_recent(rt: &Runtime, loc: &Loc) {
    if loc.is_app_internal() {
        return;
    }
    let location = loc.location();
    let _serialized = lock(&rt.inner.recent_lock);
    let mut recent = load_recent(rt);
    if recent.first() == Some(&location) {
        return;
    }
    recent.retain(|item| *item != location);
    recent.insert(0, location);
    recent.truncate(MAX_RECENT);
    if let Err(error) = write_json(&recent_path(rt), &recent) {
        rt.record_error("Zuletzt besuchte Orte speichern", &error.to_string());
    }
}

impl Runtime {
    /// Removes „Zuletzt“ entries whose location matches (e.g. a removed
    /// connection's `RemovedEndpointScope::matches_key`); returns how many.
    /// For the domain handlers that remove a connection or device.
    #[allow(dead_code)]
    pub(crate) fn forget_recent(&self, matches: &dyn Fn(&str) -> bool) -> usize {
        let _serialized = lock(&self.inner.recent_lock);
        let mut recent = load_recent(self);
        let before = recent.len();
        recent.retain(|location| !matches(location));
        let removed = before - recent.len();
        if removed > 0 {
            if let Err(error) = write_json(&recent_path(self), &recent) {
                self.record_error("Zuletzt besuchte Orte speichern", &error.to_string());
            }
        }
        removed
    }
}
