//! Google Drive backend (#19, slice 2) — `impl vfs::Backend` over the Drive v3
//! REST API, so Drive plugs into the same browse/scan/sync machinery as SFTP &
//! co. Auth (PKCE OAuth, token refresh) lives in `cloud.rs`; this module only
//! makes authenticated REST calls.
//!
//! Drive is **ID-addressed**, not path-addressed, so we keep a `path → fileId`
//! cache and resolve lazily from the My-Drive root (`"root"`). Forward-slash
//! paths are the app's convention; `"/"` is the Drive root.
//!
//! NOTE: this code follows the documented Drive v3 API but cannot be exercised
//! in the headless build env (no OAuth client). It compiles for host +
//! windows-gnu and is gated behind an explicit, user-configured connection.

use crate::cloud::{self, Provider};
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::Duration;

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

fn err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Export MIME type for a Google-Docs editors file (None = a normal binary file
/// that downloads directly via alt=media).
fn export_format(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.google-apps.document" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        "application/vnd.google-apps.spreadsheet" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        "application/vnd.google-apps.presentation" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        "application/vnd.google-apps.drawing" => "image/png",
        m if m.starts_with("application/vnd.google-apps.") && m != FOLDER_MIME => {
            "application/pdf"
        }
        _ => return None,
    })
}

/// File extension matching `export_format`.
fn export_ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.google-apps.document" => "docx",
        "application/vnd.google-apps.spreadsheet" => "xlsx",
        "application/vnd.google-apps.presentation" => "pptx",
        "application/vnd.google-apps.drawing" => "png",
        m if m.starts_with("application/vnd.google-apps.") && m != FOLDER_MIME => "pdf",
        _ => return None,
    })
}
fn not_found(p: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("nicht gefunden: {}", p))
}

/// Turn a Drive API error response into a readable io::Error (Drive returns
/// `{"error":{"code":403,"message":"…","errors":[{"reason":"…"}]}}`), so the
/// user sees e.g. "HTTP 403: … (accessNotConfigured)" instead of "status 403".
fn drive_err(code: u16, body: String) -> io::Error {
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v["error"]["message"].as_str().map(|m| {
                let reason = v["error"]["errors"][0]["reason"].as_str().unwrap_or("");
                if reason.is_empty() {
                    m.to_string()
                } else {
                    format!("{} ({})", m, reason)
                }
            })
        })
        .unwrap_or(body);
    io::Error::new(io::ErrorKind::Other, format!("HTTP {}: {}", code, msg))
}

/// Drive returns 429 / 5xx on transient overload and 403 with a
/// `rateLimitExceeded`/`userRateLimitExceeded`/`quotaExceeded` reason when a
/// user runs many requests at once (exactly the 27k-file parallel-sync case).
/// Those are safe to retry with backoff; everything else is a hard error.
fn is_rate_limited(code: u16, body: &str) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504)
        || (code == 403 && (body.contains("ateLimitExceeded") || body.contains("uotaExceeded")))
}

/// Execute a Drive request, returning the streaming response. Retries transient
/// failures (rate-limit / 5xx / transport) with exponential backoff so the
/// parallel sync engine can drive high concurrency without falling over. The
/// closure rebuilds the request each attempt (ureq requests aren't reusable).
fn open_stream<F>(f: F) -> VfsResult<ureq::Response>
where
    F: Fn() -> Result<ureq::Response, ureq::Error>,
{
    let mut delay = Duration::from_millis(400);
    let mut last: Option<io::Error> = None;
    for attempt in 0..6 {
        match f() {
            Ok(resp) => return Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if attempt < 5 && is_rate_limited(code, &body) {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(16));
                    last = Some(drive_err(code, body));
                    continue;
                }
                return Err(drive_err(code, body));
            }
            Err(e) => {
                if attempt < 5 {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(16));
                    last = Some(err(e));
                    continue;
                }
                return Err(err(e));
            }
        }
    }
    Err(last.unwrap_or_else(|| err("retry exhausted")))
}

/// `open_stream` + read the whole body to a string (for JSON endpoints).
fn send_retry<F>(f: F) -> VfsResult<String>
where
    F: Fn() -> Result<ureq::Response, ureq::Error>,
{
    open_stream(f)?.into_string().map_err(err)
}

/// Parse a (possibly empty) JSON body.
fn parse_json(s: String) -> VfsResult<serde_json::Value> {
    if s.trim().is_empty() {
        Ok(serde_json::Value::Null)
    } else {
        serde_json::from_str(&s).map_err(err)
    }
}

pub struct GDriveBackend {
    tokens: Mutex<cloud::Tokens>,
    /// path (forward-slash, no trailing slash; "" == root) → fileId
    ids: Mutex<HashMap<String, String>>,
    /// path → mimeType (so we know which files are Google-Docs editors that
    /// must be exported instead of downloaded).
    mimes: Mutex<HashMap<String, String>>,
    /// Directories whose children are fully known (enumerated by `list_dir`, or
    /// freshly created and therefore empty). For such a parent, a path NOT in
    /// `ids` is known-absent → we can create it directly and skip the per-file
    /// existence probe. This halves the round-trips during a large first sync.
    listed: Mutex<HashSet<String>>,
    /// Serializes folder creation so concurrent transfers can't create the same
    /// directory twice (Drive happily makes duplicate same-name folders).
    create_lock: Mutex<()>,
    root: String,
}

impl GDriveBackend {
    /// Build from the stored refresh token (must already be connected via
    /// `cloud::authorize`). `root` is the forward-slash start folder.
    pub fn connect(root: &str) -> Result<Self, String> {
        let tokens = cloud::refresh_access(Provider::GDrive)?;
        let mut ids = HashMap::new();
        ids.insert(String::new(), "root".to_string());
        Ok(GDriveBackend {
            tokens: Mutex::new(tokens),
            ids: Mutex::new(ids),
            mimes: Mutex::new(HashMap::new()),
            listed: Mutex::new(HashSet::new()),
            create_lock: Mutex::new(()),
            root: norm(root),
        })
    }

    /// The Drive mimeType for `path` (cached from list_dir, else a stat call).
    fn mime_of(&self, path: &str) -> Option<String> {
        let key = norm(path);
        if let Some(m) = self.mimes.lock().unwrap().get(&key).cloned() {
            return Some(m);
        }
        let id = self.resolve(&key).ok()?;
        let url = format!("{}/files/{}?fields=mimeType", API, id);
        let v = self.get_json(&url).ok()?;
        let m = v["mimeType"].as_str()?.to_string();
        self.mimes.lock().unwrap().insert(key, m.clone());
        Some(m)
    }

    fn bearer(&self) -> VfsResult<String> {
        let mut t = self.tokens.lock().unwrap();
        if now_secs() >= t.expires_at {
            *t = cloud::refresh_access(Provider::GDrive).map_err(err)?;
        }
        Ok(t.access_token.clone())
    }

    fn get_json(&self, url: &str) -> VfsResult<serde_json::Value> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        parse_json(send_retry(|| ureq::get(url).set("Authorization", &bearer).call())?)
    }

    /// Resolve a forward-slash path to a Drive fileId (walking + caching).
    fn resolve(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if let Some(id) = self.ids.lock().unwrap().get(&key).cloned() {
            return Ok(id);
        }
        // Walk segment by segment from the deepest cached ancestor.
        let segs: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        let mut cur_id = "root".to_string();
        let mut cur_path = String::new();
        for seg in segs {
            let next_path = if cur_path.is_empty() {
                seg.to_string()
            } else {
                format!("{}/{}", cur_path, seg)
            };
            if let Some(id) = self.ids.lock().unwrap().get(&next_path).cloned() {
                cur_id = id;
                cur_path = next_path;
                continue;
            }
            let child = self
                .find_child(&cur_id, seg)?
                .ok_or_else(|| not_found(&next_path))?;
            self.ids.lock().unwrap().insert(next_path.clone(), child.clone());
            cur_id = child;
            cur_path = next_path;
        }
        Ok(cur_id)
    }

    fn find_child(&self, parent_id: &str, name: &str) -> VfsResult<Option<String>> {
        let q = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            parent_id,
            name.replace('\'', "\\'")
        );
        let url = format!(
            "{}/files?q={}&fields=files(id,name)&pageSize=1",
            API,
            cloud_urlenc(&q)
        );
        let v = self.get_json(&url)?;
        Ok(v["files"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|f| f["id"].as_str())
            .map(|s| s.to_string()))
    }

    fn meta_from_json(f: &serde_json::Value) -> VfsMeta {
        let is_dir = f["mimeType"].as_str() == Some(FOLDER_MIME);
        VfsMeta {
            name: f["name"].as_str().unwrap_or_default().to_string(),
            is_dir,
            is_symlink: false,
            size: f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
            mtime_ms: f["modifiedTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            btime_ms: f["createdTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            hidden: false,
            system: false,
            id: f["id"].as_str().map(|s| s.to_string()),
            content_md5: f["md5Checksum"].as_str().map(|s| s.to_string()),
        }
    }

    /// MIME type of a file by its id (for export detection when opening by id).
    fn mime_of_id(&self, id: &str) -> Option<String> {
        let url = format!("{}/files/{}?fields=mimeType", API, id);
        self.get_json(&url).ok()?["mimeType"]
            .as_str()
            .map(|s| s.to_string())
    }

    /// Ensure a folder path exists, returning the deepest folder's id.
    /// Thread-safe: concurrent transfers may need the same folder, so the
    /// find-or-create of each level is serialized (parents are resolved first,
    /// outside the lock, to avoid re-entrancy).
    fn ensure_dir(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if key.is_empty() {
            return Ok("root".to_string());
        }
        if let Some(id) = self.ids.lock().unwrap().get(&key).cloned() {
            return Ok(id);
        }
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;

        let _g = self.create_lock.lock().unwrap();
        // Re-check under the lock — another thread may have just created it.
        if let Some(id) = self.ids.lock().unwrap().get(&key).cloned() {
            return Ok(id);
        }
        // If the parent's children are fully known and this folder isn't among
        // them, it's known-absent → skip the existence query.
        let known_absent = self.listed.lock().unwrap().contains(&parent);
        let existing = if known_absent {
            None
        } else {
            self.find_child(&parent_id, name)?
        };
        if let Some(id) = existing {
            self.ids.lock().unwrap().insert(key, id.clone());
            return Ok(id);
        }
        // Create the folder.
        let body = serde_json::json!({
            "name": name,
            "mimeType": FOLDER_MIME,
            "parents": [parent_id],
        });
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let payload = body.to_string();
        let v = parse_json(send_retry(|| {
            ureq::post(&format!("{}/files?fields=id", API))
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload)
        })?)?;
        let id = v["id"].as_str().ok_or_else(|| err("kein id nach mkdir"))?.to_string();
        self.ids.lock().unwrap().insert(key.clone(), id.clone());
        // A brand-new folder has no children → its contents are fully known.
        self.listed.lock().unwrap().insert(key);
        Ok(id)
    }

    /// Upload bytes to `path` (create or update). Used by `DriveWriter::flush`.
    fn upload(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        let key = norm(path);
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;
        // Existence: a cached id means update; otherwise, if the parent's
        // children are fully known (first sync into a fresh/empty folder), a
        // missing cache entry means it's a new file → create without the extra
        // existence probe (one fewer round-trip per file across 27k files).
        let existing = match self.ids.lock().unwrap().get(&key).cloned() {
            Some(id) => Some(id),
            None => {
                if self.listed.lock().unwrap().contains(&parent) {
                    None
                } else {
                    self.find_child(&parent_id, name)?
                }
            }
        };
        let boundary = "se_boundary_4f8a2c1d";
        let meta = if existing.is_some() {
            serde_json::json!({ "name": name })
        } else {
            serde_json::json!({ "name": name, "parents": [parent_id] })
        };
        let mut body: Vec<u8> = Vec::with_capacity(data.len() + 256);
        let head = format!(
            "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{m}\r\n--{b}\r\nContent-Type: application/octet-stream\r\n\r\n",
            b = boundary,
            m = meta
        );
        body.extend_from_slice(head.as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let ct = format!("multipart/related; boundary={}", boundary);
        let v = match &existing {
            Some(id) => {
                let url = format!("{}/{}?uploadType=multipart&fields=id", UPLOAD, id);
                parse_json(send_retry(|| {
                    ureq::request("PATCH", &url)
                        .set("Authorization", &bearer)
                        .set("Content-Type", &ct)
                        .send_bytes(&body)
                })?)?
            }
            None => {
                let url = format!("{}?uploadType=multipart&fields=id", UPLOAD);
                parse_json(send_retry(|| {
                    ureq::post(&url)
                        .set("Authorization", &bearer)
                        .set("Content-Type", &ct)
                        .send_bytes(&body)
                })?)?
            }
        };
        if let Some(id) = v["id"].as_str() {
            self.ids.lock().unwrap().insert(key, id.to_string());
        }
        Ok(())
    }

    fn trash(&self, path: &str) -> VfsResult<()> {
        let id = self.resolve(path)?;
        self.trash_id(&id)?;
        self.ids.lock().unwrap().remove(&norm(path));
        Ok(())
    }

    /// Trash one file by its exact id (targets a specific duplicate-named file).
    fn trash_id(&self, id: &str) -> VfsResult<()> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let url = format!("{}/files/{}", API, id);
        let payload = serde_json::json!({ "trashed": true }).to_string();
        send_retry(|| {
            ureq::request("PATCH", &url)
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload)
        })?;
        Ok(())
    }
}

impl Backend for GDriveBackend {
    fn scheme(&self) -> Scheme {
        Scheme::GDrive
    }
    fn root_display(&self) -> String {
        if self.root.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.root)
        }
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let id = self.resolve(path)?;
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let q = format!("'{}' in parents and trashed = false", id);
            let mut url = format!(
                "{}/files?q={}&fields=nextPageToken,files(id,name,mimeType,size,modifiedTime,createdTime,md5Checksum)&pageSize=1000",
                API,
                cloud_urlenc(&q)
            );
            if let Some(t) = &page_token {
                url.push_str(&format!("&pageToken={}", cloud_urlenc(t)));
            }
            let v = self.get_json(&url)?;
            if let Some(files) = v["files"].as_array() {
                let base = norm(path);
                for f in files {
                    let m = Self::meta_from_json(f);
                    if let Some(fid) = f["id"].as_str() {
                        let child_path = if base.is_empty() {
                            m.name.clone()
                        } else {
                            format!("{}/{}", base, m.name)
                        };
                        if let Some(mime) = f["mimeType"].as_str() {
                            self.mimes
                                .lock()
                                .unwrap()
                                .insert(child_path.clone(), mime.to_string());
                        }
                        self.ids.lock().unwrap().insert(child_path, fid.to_string());
                    }
                    out.push(m);
                }
            }
            page_token = v["nextPageToken"].as_str().map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }
        // This directory's children are now fully enumerated → future creates
        // here can skip the existence probe (see `upload`/`ensure_dir`).
        self.listed.lock().unwrap().insert(norm(path));
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let key = norm(path);
        if key.is_empty() {
            return Ok(VfsMeta {
                name: "/".into(),
                is_dir: true,
                is_symlink: false,
                size: 0,
                mtime_ms: 0,
                btime_ms: 0,
                hidden: false,
                system: false,
                id: None,
                content_md5: None,
            });
        }
        let id = self.resolve(&key)?;
        let url = format!(
            "{}/files/{}?fields=id,name,mimeType,size,modifiedTime,createdTime,md5Checksum",
            API, id
        );
        let v = self.get_json(&url)?;
        Ok(Self::meta_from_json(&v))
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        // Target a specific file by id (disambiguates duplicate names); fall back
        // to the path-based open when no id is supplied.
        let id = match id {
            Some(i) if !i.is_empty() => i.to_string(),
            _ => return self.open_read(path),
        };
        let auth = self.bearer()?;
        let mime = self.mime_of_id(&id).unwrap_or_default();
        let url = if let Some(fmt) = export_format(&mime) {
            format!("{}/files/{}/export?mimeType={}", API, id, cloud_urlenc(fmt))
        } else {
            format!("{}/files/{}?alt=media", API, id)
        };
        let bearer = format!("Bearer {}", auth);
        let resp = open_stream(|| ureq::get(&url).set("Authorization", &bearer).call())?;
        Ok(Box::new(resp.into_reader()))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let id = self.resolve(path)?;
        let auth = self.bearer()?;
        // Google-Docs editors files (Docs/Sheets/Slides/Drawings) have no binary
        // content and 403 on alt=media ("fileNotDownloadable") — they must be
        // EXPORTED to an Office/PDF format instead.
        let mime = self.mime_of(path).unwrap_or_default();
        let url = if let Some(fmt) = export_format(&mime) {
            format!("{}/files/{}/export?mimeType={}", API, id, cloud_urlenc(fmt))
        } else {
            format!("{}/files/{}?alt=media", API, id)
        };
        let bearer = format!("Bearer {}", auth);
        let resp = open_stream(|| ureq::get(&url).set("Authorization", &bearer).call())?;
        Ok(Box::new(resp.into_reader()))
    }

    /// The filename to save a download as. Google-Docs editors files carry no
    /// extension, so append the export format's extension (.docx/.xlsx/…) so the
    /// downloaded copy opens in the right app.
    fn download_name(&self, path: &str, name: &str) -> String {
        let mime = self.mime_of(path).unwrap_or_default();
        match export_ext(&mime) {
            Some(ext) if !name.to_lowercase().ends_with(&format!(".{}", ext)) => {
                format!("{}.{}", name, ext)
            }
            _ => name.to_string(),
        }
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(DriveWriter {
            backend: self as *const _,
            path: norm(path),
            buf: Vec::new(),
            done: false,
        }))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let id = self.resolve(src)?;
        let src_key = norm(src);
        let dst_key = norm(dst);
        let (src_parent, _) = split_parent(&src_key);
        let (dst_parent, dst_name) = split_parent(&dst_key);
        let src_parent_id = self.ensure_dir(&src_parent)?;
        let dst_parent_id = self.ensure_dir(&dst_parent)?;
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let mut url = format!("{}/files/{}?fields=id", API, id);
        if src_parent_id != dst_parent_id {
            url.push_str(&format!(
                "&addParents={}&removeParents={}",
                dst_parent_id, src_parent_id
            ));
        }
        let payload = serde_json::json!({ "name": dst_name }).to_string();
        send_retry(|| {
            ureq::request("PATCH", &url)
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload)
        })?;
        let mut ids = self.ids.lock().unwrap();
        ids.remove(&norm(src));
        ids.insert(norm(dst), id);
        Ok(())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        match id {
            Some(i) if !i.is_empty() => self.trash_id(i),
            _ => self.trash(path),
        }
    }

    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        use std::collections::HashMap;
        let root_n = norm(root);
        let mut removed = 0usize;
        let mut stack = vec![root_n.clone()];
        while let Some(dir) = stack.pop() {
            let dir_rel = if root_n.is_empty() {
                dir.clone()
            } else {
                dir.strip_prefix(&root_n).unwrap_or("").trim_start_matches('/').to_string()
            };
            let entries = self.list_dir(&dir)?;
            let mut by_name: HashMap<String, Vec<VfsMeta>> = HashMap::new();
            for m in entries {
                if m.is_dir {
                    let child = if dir.is_empty() {
                        m.name.clone()
                    } else {
                        format!("{}/{}", dir.trim_end_matches('/'), m.name)
                    };
                    stack.push(child);
                } else {
                    by_name.entry(m.name.clone()).or_default().push(m);
                }
            }
            for (name, mut group) in by_name {
                if group.len() < 2 {
                    continue; // only ever remove EXTRA copies, never singletons
                }
                let rel = if dir_rel.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", dir_rel, name)
                };
                // Newest first; if the name belongs in the mirror, keep it and
                // trash the older copies; if it's an orphaned dup, trash them all.
                group.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
                let start = if keep(&rel) { 1 } else { 0 };
                for extra in &group[start..] {
                    if let Some(id) = &extra.id {
                        if self.trash_id(id).is_ok() {
                            removed += 1;
                        }
                    }
                }
            }
        }
        Ok(removed)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.ensure_dir(path).map(|_| ())
    }

    fn parallelism(&self) -> usize {
        // Per-file transfers are latency-bound (each is a couple of HTTPS
        // round-trips), so concurrency is the dominant throughput lever for
        // many-small-files syncs. Drive tolerates this well and `open_stream`
        // backs off on the rare rate-limit response.
        16
    }
}

/// Buffers written bytes and uploads to Drive on `flush` (so bisync's
/// `copy_between`, which flushes, surfaces upload errors) — and as a safety net
/// on drop if flush was never called.
struct DriveWriter {
    backend: *const GDriveBackend,
    path: String,
    buf: Vec<u8>,
    done: bool,
}
// The pointer is only used synchronously while the owning backend is alive
// (the writer never outlives the copy call); Send is needed for Box<dyn Write+Send>.
unsafe impl Send for DriveWriter {}

impl DriveWriter {
    fn flush_upload(&mut self) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        let be = unsafe { &*self.backend };
        be.upload(&self.path, &self.buf)?;
        self.done = true;
        Ok(())
    }
}

impl Write for DriveWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flush_upload()
    }
}
impl Drop for DriveWriter {
    fn drop(&mut self) {
        let _ = self.flush_upload();
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn norm(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
}

fn split_parent(key: &str) -> (String, &str) {
    match key.rsplit_once('/') {
        Some((par, name)) => (par.to_string(), name),
        None => (String::new(), key),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Minimal URL-component encoder (reuses the same rules as cloud.rs).
fn cloud_urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// RFC 3339 (e.g. "2024-06-01T12:34:56.000Z") → unix millis (best effort).
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_and_split() {
        assert_eq!(norm("/a/b/"), "a/b");
        assert_eq!(norm("/"), "");
        let (p, n) = split_parent("a/b/c");
        assert_eq!(p, "a/b");
        assert_eq!(n, "c");
        let (p, n) = split_parent("x");
        assert_eq!(p, "");
        assert_eq!(n, "x");
    }

    #[test]
    fn rfc3339_parses() {
        assert!(parse_rfc3339_ms("2024-06-01T12:34:56Z").unwrap() > 0);
        assert!(parse_rfc3339_ms("not a date").is_none());
    }
}
