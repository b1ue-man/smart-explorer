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
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Mutex;

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

/// Parse a ureq result as JSON, surfacing the error body on 4xx/5xx.
fn resp_json(r: Result<ureq::Response, ureq::Error>) -> VfsResult<serde_json::Value> {
    match r {
        Ok(resp) => {
            let s = resp.into_string().map_err(err)?;
            if s.trim().is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                serde_json::from_str(&s).map_err(err)
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            Err(drive_err(code, resp.into_string().unwrap_or_default()))
        }
        Err(e) => Err(err(e)),
    }
}

/// Like `resp_json` but discards the body — for calls we only need to succeed.
fn check(r: Result<ureq::Response, ureq::Error>) -> VfsResult<()> {
    resp_json(r).map(|_| ())
}

pub struct GDriveBackend {
    tokens: Mutex<cloud::Tokens>,
    /// path (forward-slash, no trailing slash; "" == root) → fileId
    ids: Mutex<HashMap<String, String>>,
    /// path → mimeType (so we know which files are Google-Docs editors that
    /// must be exported instead of downloaded).
    mimes: Mutex<HashMap<String, String>>,
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
        resp_json(ureq::get(url).set("Authorization", &format!("Bearer {}", auth)).call())
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
        }
    }

    /// Ensure a folder path exists, returning the deepest folder's id.
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
        if let Some(id) = self.find_child(&parent_id, name)? {
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
        let v = resp_json(
            ureq::post(&format!("{}/files?fields=id", API))
                .set("Authorization", &format!("Bearer {}", auth))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
        )?;
        let id = v["id"].as_str().ok_or_else(|| err("kein id nach mkdir"))?.to_string();
        self.ids.lock().unwrap().insert(key, id.clone());
        Ok(id)
    }

    /// Upload bytes to `path` (create or update). Used by `DriveWriter::flush`.
    fn upload(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        let key = norm(path);
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;
        let existing = self.find_child(&parent_id, name)?;
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
        let ct = format!("multipart/related; boundary={}", boundary);
        let v = match existing {
            Some(id) => resp_json(
                ureq::request(
                    "PATCH",
                    &format!("{}/{}?uploadType=multipart&fields=id", UPLOAD, id),
                )
                .set("Authorization", &format!("Bearer {}", auth))
                .set("Content-Type", &ct)
                .send_bytes(&body),
            )?,
            None => resp_json(
                ureq::post(&format!("{}?uploadType=multipart&fields=id", UPLOAD))
                    .set("Authorization", &format!("Bearer {}", auth))
                    .set("Content-Type", &ct)
                    .send_bytes(&body),
            )?,
        };
        if let Some(id) = v["id"].as_str() {
            self.ids.lock().unwrap().insert(key, id.to_string());
        }
        Ok(())
    }

    fn trash(&self, path: &str) -> VfsResult<()> {
        let id = self.resolve(path)?;
        let auth = self.bearer()?;
        check(
            ureq::request("PATCH", &format!("{}/files/{}", API, id))
                .set("Authorization", &format!("Bearer {}", auth))
                .set("Content-Type", "application/json")
                .send_string(&serde_json::json!({ "trashed": true }).to_string()),
        )?;
        self.ids.lock().unwrap().remove(&norm(path));
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
                "{}/files?q={}&fields=nextPageToken,files(id,name,mimeType,size,modifiedTime,createdTime)&pageSize=1000",
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
            });
        }
        let id = self.resolve(&key)?;
        let url = format!(
            "{}/files/{}?fields=id,name,mimeType,size,modifiedTime,createdTime",
            API, id
        );
        let v = self.get_json(&url)?;
        Ok(Self::meta_from_json(&v))
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
        match ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", auth))
            .call()
        {
            Ok(resp) => Ok(Box::new(resp.into_reader())),
            Err(ureq::Error::Status(code, resp)) => {
                Err(drive_err(code, resp.into_string().unwrap_or_default()))
            }
            Err(e) => Err(err(e)),
        }
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
        let mut url = format!("{}/files/{}?fields=id", API, id);
        if src_parent_id != dst_parent_id {
            url.push_str(&format!(
                "&addParents={}&removeParents={}",
                dst_parent_id, src_parent_id
            ));
        }
        check(
            ureq::request("PATCH", &url)
                .set("Authorization", &format!("Bearer {}", auth))
                .set("Content-Type", "application/json")
                .send_string(&serde_json::json!({ "name": dst_name }).to_string()),
        )?;
        let mut ids = self.ids.lock().unwrap();
        ids.remove(&norm(src));
        ids.insert(norm(dst), id);
        Ok(())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.ensure_dir(path).map(|_| ())
    }

    fn parallelism(&self) -> usize {
        4 // Drive rate-limits; keep concurrency modest.
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
