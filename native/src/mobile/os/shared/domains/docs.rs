//! `docs.*`: the Android documents provider (DRV-M8, strand S2 C3). The
//! Kotlin `SeDocumentsProvider` asks for roots, documents and listings
//! here; descriptors go through `docs_io.rs`.
//!
//! Only roots the person released in „Ordner für Apps bereitstellen“ exist
//! for other apps (`released`, checked on every call, so a guessed ID of
//! another root gets nothing). Nothing is released by default. A vault
//! root exists only while the vault is unlocked (`docs_vault.rs`).
//! `docs.roots` never touches the network (DocumentsUI waits 5 s); a cold
//! listing answers `loading` at once and `docs.changed` follows when it is
//! there (EXTRA_LOADING, NR N15); an offline device answers its error.
//!
//! Event `{"type":"docs.changed","roots":bool,"documents":[id…],
//! "commitFailed":bool}`.
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::args::{invalid, str_arg};
use super::docs_engines::{self, EngineKey, OFFLINE_TEXT};
use super::docs_ids::{DocId, RootId, DEVICES, GDRIVE};
use super::docs_vault::{self, released, settings};
use crate::mobile::runtime::lock;
use crate::mobile::{ApiError, Runtime};

const DEVICES_TITLE: &str = "Smart Explorer – Geräte";
/// A listing younger than this answers without asking the engine again.
const FRESH: Duration = Duration::from_secs(15);
/// Longest wait of `docs.retryPending` for running releases.
const RELEASE_WAIT: Duration = Duration::from_secs(600);

pub(super) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    docs_vault::install(Arc::new(|root: &RootId| root_changed(root)));
    Some(match method {
        "docs.roots" => Ok(roots()),
        "docs.document" => doc_arg(args).and_then(|doc| document(&doc)),
        "docs.children" => doc_arg(args).map(|doc| children(&doc)),
        "docs.create" => create(args),
        "docs.rename" => rename(args),
        "docs.delete" => doc_arg(args).and_then(|doc| delete(&doc)),
        "docs.settings.get" => Ok(docs_vault::settings_json()),
        "docs.settings.set" => docs_vault::set_settings(rt, args),
        "docs.pending" => Ok(json!({ "pending": docs_engines::pending() })),
        "docs.retryPending" => Ok(retry_pending()),
        _ => return None,
    })
}

/// The ID of a released root; any other ID is `not_found`, as if it never
/// existed.
fn doc_arg(args: &Value) -> Result<DocId, ApiError> {
    let text = str_arg(args, "documentId")?;
    DocId::parse(text)
        .filter(|doc| released(&doc.root))
        .ok_or_else(|| ApiError::not_found("Dokument nicht gefunden"))
}

fn emit(roots: bool, documents: Vec<String>, commit_failed: bool) {
    if let Some(rt) = Runtime::installed() {
        rt.emit(json!({
            "type": "docs.changed",
            "roots": roots,
            "documents": documents,
            "commitFailed": commit_failed,
        }));
    }
}

/// A root came or went (vault unlocked or locked, settings changed).
fn root_changed(root: &RootId) {
    forget_listings(root);
    emit(true, Vec::new(), false);
}

/// A release published (or failed to): listings of `root` are stale.
pub(super) fn committed(root: &RootId, failed: bool) {
    forget_listings(root);
    emit(
        false,
        vec![DocId::root_of(root.clone()).to_string()],
        failed,
    );
}

fn roots() -> Value {
    let settings = settings();
    if !settings.provider {
        return Value::Array(Vec::new());
    }
    let mut roots = Vec::new();
    if settings.devices {
        roots.push(root_json(&RootId::Devices, DEVICES_TITLE, false, false));
    }
    if let Ok(connections) = crate::creds::load_connections_checked() {
        for connection in connections.iter().filter(|c| c.protocol.is_url()) {
            let account = connection.account();
            if settings.connections.contains(&account) {
                let title = connection.display();
                roots.push(root_json(&RootId::Conn(account), &title, true, false));
            }
        }
    }
    if settings.connections.iter().any(|id| id == GDRIVE)
        && crate::cloud::is_connected(crate::cloud::Provider::GDrive)
    {
        roots.push(root_json(&RootId::GDrive, "Google Drive", true, false));
    }
    for vault in docs_vault::unlocked() {
        if settings.vaults.contains(&vault.site) {
            let title = format!("{} (Tresor)", vault.label);
            roots.push(root_json(
                &RootId::Vault(vault.site),
                &title,
                true,
                vault.local,
            ));
        }
    }
    Value::Array(roots)
}

/// `create`: what is known without the network (the open engine, else the
/// kind of root; a refused write then reports its error).
fn root_json(root: &RootId, title: &str, create: bool, local_only: bool) -> Value {
    let create =
        docs_engines::opened(&EngineKey::Root(root.clone())).map_or(create, |slot| slot.writable);
    let kind = match root {
        RootId::Devices => DEVICES,
        RootId::Conn(_) => "conn",
        RootId::GDrive => GDRIVE,
        RootId::Vault(_) => "vault",
    };
    json!({
        "rootId": root.to_string(),
        "documentId": root.to_string(),
        "title": title,
        "kind": kind,
        "create": create,
        "localOnly": local_only,
    })
}

#[derive(Clone)]
struct Item {
    id: String,
    name: String,
    dir: bool,
    size: u64,
    mtime_ms: i64,
    writable: bool,
}

impl Item {
    fn json(&self) -> Value {
        json!({
            "documentId": self.id,
            "name": self.name,
            "dir": self.dir,
            "size": self.size,
            "mtime": self.mtime_ms,
            "writable": self.writable,
        })
    }
}

fn root_title(root: &RootId) -> String {
    let found = roots();
    found
        .as_array()
        .and_then(|roots| {
            roots
                .iter()
                .find(|entry| entry["rootId"] == root.to_string())
                .and_then(|entry| entry["title"].as_str().map(str::to_string))
        })
        .unwrap_or_else(|| root.to_string())
}

fn folder(id: String, name: String, writable: bool) -> Item {
    Item {
        id,
        name,
        dir: true,
        size: 0,
        mtime_ms: 0,
        writable,
    }
}

fn document(doc: &DocId) -> Result<Value, ApiError> {
    let item = match EngineKey::of(doc) {
        None => folder(doc.to_string(), DEVICES_TITLE.to_string(), false),
        Some((EngineKey::Device(key), path)) if path == "/" => {
            let name =
                device_name(&key).ok_or_else(|| ApiError::not_found("Gerät nicht gefunden"))?;
            folder(doc.to_string(), name, false)
        }
        Some((key, path)) if path == "/" => {
            let writable = docs_engines::opened(&key).is_none_or(|slot| slot.writable);
            folder(doc.to_string(), root_title(&doc.root), writable)
        }
        Some((key, path)) => {
            let slot = docs_engines::engine(&key).map_err(|error| failure(doc, error))?;
            let meta = slot
                .fs
                .stat_cached(&path)
                .map_err(|error| failure(doc, error))?;
            Item {
                id: doc.to_string(),
                name: doc.name(),
                dir: meta.is_dir,
                size: meta.size,
                mtime_ms: meta.mtime_ms,
                writable: slot.writable,
            }
        }
    };
    Ok(item.json())
}

fn device_name(key: &str) -> Option<String> {
    let profiles = docs_engines::profiles();
    let row = docs_engines::device_rows(&profiles)
        .into_iter()
        .find(|row| row.key == key)?;
    Some(match &row.room {
        Some(room) => format!("{} ({})", row.name, room.name),
        None => row.name,
    })
}

/// A listing as the provider last saw it.
#[derive(Clone)]
struct Listing {
    items: Vec<Item>,
    error: Option<String>,
    at: Instant,
}

#[derive(Default)]
struct Listings {
    known: HashMap<DocId, Listing>,
    loading: HashSet<DocId>,
}

fn listings() -> &'static Mutex<Listings> {
    static LISTINGS: OnceLock<Mutex<Listings>> = OnceLock::new();
    LISTINGS.get_or_init(|| Mutex::new(Listings::default()))
}

/// Drops the listings of `root` (a vault's cleartext names go with it).
pub(super) fn forget_listings(root: &RootId) {
    let mut listings = lock(listings());
    listings.known.retain(|doc, _| doc.root != *root);
}

fn children(doc: &DocId) -> Value {
    let Some((key, path)) = EngineKey::of(doc) else {
        return device_list();
    };
    if let EngineKey::Device(device) = &key {
        if !device_online(device) {
            return json!({ "loading": false, "error": OFFLINE_TEXT, "children": [] });
        }
    }
    let (known, fresh, start) = {
        let mut listings = lock(listings());
        let known = listings.known.get(doc).cloned();
        let fresh = known
            .as_ref()
            .is_some_and(|listing| listing.at.elapsed() < FRESH);
        let start = !fresh && listings.loading.insert(doc.clone());
        (known, fresh, start)
    };
    if start {
        load(doc.clone(), key, path);
    }
    // While loading, the last listing stays visible without its error.
    let items: Vec<Value> = known
        .iter()
        .flat_map(|listing| listing.items.iter().map(Item::json))
        .collect();
    let error = known.and_then(|listing| listing.error).filter(|_| fresh);
    json!({ "loading": !fresh, "error": error, "children": items })
}

fn device_online(key: &str) -> bool {
    let profiles = docs_engines::profiles();
    docs_engines::device_rows(&profiles)
        .iter()
        .any(|row| row.key == key && row.online)
}

fn device_list() -> Value {
    let profiles = docs_engines::profiles();
    let items: Vec<Value> = docs_engines::device_rows(&profiles)
        .into_iter()
        .filter_map(|row| {
            let id = DocId::root_of(RootId::Devices).child(&row.key)?;
            let name = match &row.room {
                Some(room) => format!("{} ({})", row.name, room.name),
                None => row.name.clone(),
            };
            let mut item = folder(id.to_string(), name, false).json();
            item["online"] = Value::Bool(row.online);
            Some(item)
        })
        .collect();
    json!({ "loading": false, "error": Value::Null, "children": items })
}

/// Lists `doc` in the background, then tells the provider.
fn load(doc: DocId, key: EngineKey, path: String) {
    let spawned = std::thread::Builder::new().name("docs-list".into()).spawn({
        let doc = doc.clone();
        move || {
            let listing = list(&doc, &key, &path);
            {
                let mut listings = lock(listings());
                listings.loading.remove(&doc);
                // A root withdrawn meanwhile keeps no listing.
                if released(&doc.root) {
                    listings.known.insert(doc.clone(), listing);
                }
            }
            emit(false, vec![doc.to_string()], false);
        }
    });
    if spawned.is_err() {
        lock(listings()).loading.remove(&doc);
    }
}

fn list(doc: &DocId, key: &EngineKey, path: &str) -> Listing {
    let result = docs_engines::engine(key).and_then(|slot| {
        let entries = slot.fs.list_dir_cached(path)?;
        Ok(entries
            .iter()
            .filter(|meta| !meta.is_symlink && !meta.special)
            .filter_map(|meta| {
                let id = doc.child(&meta.name)?;
                Some(Item {
                    id: id.to_string(),
                    name: meta.name.clone(),
                    dir: meta.is_dir,
                    size: meta.size,
                    mtime_ms: meta.mtime_ms,
                    writable: slot.writable,
                })
            })
            .collect())
    });
    match result {
        Ok(items) => Listing {
            items,
            error: None,
            at: Instant::now(),
        },
        Err(error) => Listing {
            items: Vec::new(),
            error: Some(failure(doc, error).message),
            at: Instant::now(),
        },
    }
}

/// An error for the picker. A vault's text never carries a path; an
/// unreachable device reads „Gerät nicht erreichbar“.
fn failure(doc: &DocId, error: io::Error) -> ApiError {
    let errno = super::docs_io::errno_for(&error);
    if doc.root == RootId::Devices && errno == libc::EHOSTUNREACH {
        return ApiError::new("network", OFFLINE_TEXT);
    }
    if matches!(doc.root, RootId::Vault(_)) {
        let api = ApiError::from(error);
        let text = match api.kind {
            "not_found" => "Nicht gefunden",
            "permission" => "Keine Berechtigung",
            "exists" => "Existiert bereits",
            _ => "Tresor nicht lesbar oder gesperrt",
        };
        return ApiError::new(api.kind, text);
    }
    ApiError::from(error)
}

/// The engine and path of a document that is no virtual level.
fn engine_doc(doc: &DocId) -> Result<(Arc<docs_engines::Slot>, String), ApiError> {
    let (key, path) = EngineKey::of(doc)
        .ok_or_else(|| ApiError::new("permission", "Hier kann nichts geändert werden"))?;
    let slot = docs_engines::engine(&key).map_err(|error| failure(doc, error))?;
    Ok((slot, path))
}

fn changed_in(parent: &DocId) {
    lock(listings()).known.remove(parent);
    emit(false, vec![parent.to_string()], false);
}

/// `docs.create {documentId (folder), name, dir}`: a free name, counting
/// „Name (2).txt“, „Name (3).txt“ … until one is free.
fn create(args: &Value) -> Result<Value, ApiError> {
    let parent = doc_arg(args)?;
    let name = str_arg(args, "name")?.trim();
    let dir = args.get("dir").and_then(Value::as_bool).unwrap_or(false);
    if EngineKey::of(&parent).is_none() {
        return Err(ApiError::new(
            "permission",
            "Hier kann nichts angelegt werden",
        ));
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !dir && !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let mut number: u64 = 1;
    loop {
        let candidate = if number == 1 {
            name.to_string()
        } else {
            format!("{stem} ({number}){extension}")
        };
        let doc = parent
            .child(&candidate)
            .ok_or_else(|| invalid("Ungültiger Name"))?;
        let (slot, path) = engine_doc(&doc)?;
        match docs_engines::create(&slot, &path, dir) {
            Ok(()) => {
                changed_in(&parent);
                return document(&doc);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                number = number
                    .checked_add(1)
                    .ok_or_else(|| ApiError::internal("Kein freier Name"))?;
            }
            Err(error) => return Err(failure(&doc, error)),
        }
    }
}

/// `docs.rename {documentId, name}` inside the same folder.
fn rename(args: &Value) -> Result<Value, ApiError> {
    let doc = doc_arg(args)?;
    let name = str_arg(args, "name")?.trim();
    let parent = doc
        .parent()
        .filter(|_| EngineKey::of(&doc).is_some_and(|(_, path)| path != "/"))
        .ok_or_else(|| ApiError::new("permission", "Dieser Ordner kann nicht umbenannt werden"))?;
    let target = parent
        .child(name)
        .ok_or_else(|| invalid("Ungültiger Name"))?;
    let (slot, from) = engine_doc(&doc)?;
    let (_, to) = engine_doc(&target)?;
    docs_engines::rename(&slot, &from, &to).map_err(|error| failure(&doc, error))?;
    changed_in(&parent);
    Ok(json!({ "documentId": target.to_string() }))
}

/// `docs.delete {documentId}`: a folder with its content.
fn delete(doc: &DocId) -> Result<Value, ApiError> {
    let parent = doc
        .parent()
        .filter(|_| EngineKey::of(doc).is_some_and(|(_, path)| path != "/"))
        .ok_or_else(|| ApiError::new("permission", "Dieser Ordner kann nicht gelöscht werden"))?;
    let (slot, path) = engine_doc(doc)?;
    docs_engines::delete(&slot, &path).map_err(|error| failure(doc, error))?;
    changed_in(&parent);
    Ok(json!({}))
}

fn retry_pending() -> Value {
    super::docs_io::wait_for_releases(RELEASE_WAIT);
    let failed = docs_engines::retry_pending(&released);
    json!({ "pending": docs_engines::pending(), "failed": failed })
}
