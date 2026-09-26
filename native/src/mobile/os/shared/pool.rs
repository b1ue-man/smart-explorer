//! One live backend per connection, reused across calls like a desktop tab
//! keeps its `RemoteState`. Remote backends are wrapped in `CachingBackend`
//! (the desktop browsing cache); local paths share one `LocalBackend`.
use super::error::ApiError;
use super::location::{Loc, LocKind};
use super::runtime::lock;
use crate::vfs::{BackendHandle, CachingBackend, LocalBackend};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

/// Browsed archives kept open; the least recently used one goes first.
const MAX_POOLED_ZIPS: usize = 4;

struct Pooled {
    backend: BackendHandle,
    /// Archive size and modification time a `ZipBackend` was opened with.
    zip_stamp: Option<(u64, Option<SystemTime>)>,
    last_used: Instant,
}

pub(crate) struct BackendPool {
    local: BackendHandle,
    entries: Mutex<HashMap<String, Pooled>>,
}

enum Plan {
    Saved(crate::creds::SavedConnection, String),
    GDrive(String),
    Share(crate::share::PeerOpenTarget, String),
    Zip(String, String),
}

impl BackendPool {
    pub(crate) fn new() -> Self {
        Self {
            local: Arc::new(LocalBackend::new("/")),
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn resolve(&self, loc: &Loc) -> Result<(BackendHandle, String), ApiError> {
        let (key, plan) = match self.plan(loc)? {
            Some(planned) => planned,
            None => return Ok((self.local.clone(), loc.path.clone())),
        };
        let zip_stamp = match &plan {
            Plan::Zip(archive, _) => match archive_stamp(archive) {
                Ok(stamp) => Some(stamp),
                Err(error) => {
                    // A deleted or replaced archive releases its backend.
                    lock(&self.entries).remove(&key);
                    return Err(error);
                }
            },
            _ => None,
        };
        if let Some(pooled) = lock(&self.entries).get_mut(&key) {
            if pooled.zip_stamp == zip_stamp {
                pooled.last_used = Instant::now();
                return Ok((pooled.backend.clone(), plan.path().to_string()));
            }
        }
        // Connect without holding the pool lock; a concurrent winner is kept.
        let (backend, path) = open(plan)?;
        let mut entries = lock(&self.entries);
        if zip_stamp.is_some() && !entries.contains_key(&key) {
            make_room_for_zip(&mut entries);
        }
        let pooled = entries
            .entry(key)
            .and_modify(|pooled| {
                if pooled.zip_stamp != zip_stamp {
                    pooled.backend = backend.clone();
                    pooled.zip_stamp = zip_stamp;
                }
                pooled.last_used = Instant::now();
            })
            .or_insert_with(|| Pooled {
                backend: backend.clone(),
                zip_stamp,
                last_used: Instant::now(),
            });
        Ok((pooled.backend.clone(), path))
    }

    /// Forgets the connection of `loc`; the next call reconnects.
    pub(crate) fn evict(&self, loc: &Loc) {
        if let Ok(Some((key, _))) = self.plan(loc) {
            lock(&self.entries).remove(&key);
        }
    }

    pub(crate) fn retain(&self, keep: impl Fn(&str) -> bool) {
        lock(&self.entries).retain(|key, _| keep(key));
    }

    /// The pool key and how to open the backend; `None` for local paths.
    fn plan(&self, loc: &Loc) -> Result<Option<(String, Plan)>, ApiError> {
        let location = loc.location();
        Ok(Some(match loc.kind {
            LocKind::Local => return Ok(None),
            LocKind::Trash => {
                return Err(ApiError::unsupported("Der Papierkorb ist kein Dateiort"))
            }
            LocKind::Sftp | LocKind::Ftp | LocKind::Ftps | LocKind::Webdav => {
                let (connection, path) =
                    crate::connect::saved_and_path(&location).ok_or_else(|| {
                        ApiError::not_found(
                            "Keine gespeicherte Verbindung für diese Remote-Adresse gefunden",
                        )
                    })?;
                (connection.account(), Plan::Saved(connection, path))
            }
            LocKind::GDrive => ("gdrive://".to_string(), Plan::GDrive(loc.path.clone())),
            LocKind::Share => {
                let (target, path) = crate::share::PeerOpenTarget::from_endpoint(&location)
                    .ok_or_else(|| ApiError::invalid("Ungültige Share-Adresse"))?;
                (target.endpoint_prefix(), Plan::Share(target, path))
            }
            LocKind::Zip => {
                let archive = loc
                    .zip_archive()
                    .ok_or_else(|| ApiError::invalid("Ungültiger ZIP-Ort"))?
                    .to_string();
                (
                    format!("zip://{archive}"),
                    Plan::Zip(archive, loc.path.clone()),
                )
            }
        }))
    }
}

impl Plan {
    fn path(&self) -> &str {
        match self {
            Plan::Saved(_, path)
            | Plan::GDrive(path)
            | Plan::Share(_, path)
            | Plan::Zip(_, path) => path,
        }
    }
}

fn open(plan: Plan) -> Result<(BackendHandle, String), ApiError> {
    let (backend, path) = match plan {
        Plan::Saved(connection, path) => {
            let (backend, _) =
                crate::connect::open_saved_at(&connection, &path).map_err(ApiError::connection)?;
            (backend, path)
        }
        Plan::GDrive(path) => {
            let (backend, _) = crate::connect::open_gdrive(&path).map_err(ApiError::connection)?;
            (backend, path)
        }
        Plan::Share(target, path) => {
            let (_, backend, _) =
                crate::daemon::open_share_backend(target).map_err(ApiError::connection)?;
            (backend, path)
        }
        Plan::Zip(archive, path) => {
            let backend = crate::zipfs::ZipBackend::open(&archive)
                .map_err(|error| ApiError::from(error).context("ZIP öffnen"))?;
            // The archive map is already in memory; no browsing cache needed.
            return Ok((Arc::new(backend), path));
        }
    };
    let backend: BackendHandle = if backend.is_local() {
        backend
    } else {
        Arc::new(CachingBackend::new(backend))
    };
    Ok((backend, path))
}

/// Closes the least recently used archives until a new one fits below
/// `MAX_POOLED_ZIPS`. Remote connections are never touched; running tasks
/// keep their own handle.
fn make_room_for_zip(entries: &mut HashMap<String, Pooled>) {
    let mut zips: Vec<(Instant, String)> = entries
        .iter()
        .filter(|(_, pooled)| pooled.zip_stamp.is_some())
        .map(|(key, pooled)| (pooled.last_used, key.clone()))
        .collect();
    zips.sort();
    let excess = (zips.len() + 1).saturating_sub(MAX_POOLED_ZIPS);
    for (_, key) in zips.into_iter().take(excess) {
        entries.remove(&key);
    }
}

fn archive_stamp(archive: &str) -> Result<(u64, Option<SystemTime>), ApiError> {
    let metadata =
        std::fs::metadata(archive).map_err(|error| ApiError::from(error).context("ZIP lesen"))?;
    if !metadata.is_file() {
        return Err(ApiError::invalid("Das ZIP-Archiv ist keine Datei"));
    }
    Ok((metadata.len(), metadata.modified().ok()))
}
