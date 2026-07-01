use super::api::{drive_request, export_ext, export_format, open_stream, send_retry, API};
use super::core::{cloud_urlenc, norm, split_parent};
use super::transfer::open_writer;
use super::GDriveBackend;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::collections::HashMap;
use std::io::{Read, Write};

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
        let mut pending_ids: Vec<(String, String, String, Option<String>)> = Vec::new();
        let mut name_counts: HashMap<String, usize> = HashMap::new();
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
                    let Some(m) = Self::meta_from_json(f, None) else {
                        continue;
                    };
                    *name_counts.entry(m.name.clone()).or_default() += 1;
                    if let Some(fid) = f["id"].as_str() {
                        let child_path = if base.is_empty() {
                            m.name.clone()
                        } else {
                            format!("{}/{}", base, m.name)
                        };
                        pending_ids.push((
                            child_path,
                            m.name.clone(),
                            fid.to_string(),
                            f["mimeType"].as_str().map(str::to_string),
                        ));
                    }
                    out.push(m);
                }
            }
            page_token = v["nextPageToken"].as_str().map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }
        for (child_path, name, fid, mime) in pending_ids {
            if name_counts.get(&name).copied() == Some(1) {
                self.remember_path(&child_path, &fid, mime.as_deref())?;
            } else {
                self.forget_path_prefix(&child_path);
            }
        }
        // This directory's children are now fully enumerated -> future creates
        // here can skip the existence probe (see `upload`/`ensure_dir`).
        self.listed_guard()?.insert(norm(path));
        self.persist_path_cache();
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
        let fallback = key.rsplit('/').next().filter(|s| !s.is_empty());
        Self::meta_from_json(&v, fallback).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Drive-Metadaten ohne Namen",
            )
        })
    }

    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        self.resolve(path).map(Some)
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
        let resp =
            open_stream(|| drive_request(ureq::get(&url).set("Authorization", &bearer).call()))?;
        Ok(Box::new(resp.into_reader()))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let id = self.resolve(path)?;
        let auth = self.bearer()?;
        // Google-Docs editors files (Docs/Sheets/Slides/Drawings) have no binary
        // content and 403 on alt=media ("fileNotDownloadable") - they must be
        // EXPORTED to an Office/PDF format instead.
        let mime = self.mime_of(path).unwrap_or_default();
        let url = if let Some(fmt) = export_format(&mime) {
            format!("{}/files/{}/export?mimeType={}", API, id, cloud_urlenc(fmt))
        } else {
            format!("{}/files/{}?alt=media", API, id)
        };
        let bearer = format!("Bearer {}", auth);
        let resp =
            open_stream(|| drive_request(ureq::get(&url).set("Authorization", &bearer).call()))?;
        Ok(Box::new(resp.into_reader()))
    }

    /// The filename to save a download as. Google-Docs editors files carry no
    /// extension, so append the export format's extension (.docx/.xlsx/...) so
    /// the downloaded copy opens in the right app.
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
        Ok(open_writer(self, path))
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
            drive_request(
                ureq::request("PATCH", &url)
                    .set("Authorization", &bearer)
                    .set("Content-Type", "application/json")
                    .send_string(&payload),
            )
        })?;
        self.forget_path_prefix(&norm(src));
        self.remember_path(&norm(dst), &id, None)?;
        self.persist_path_cache();
        Ok(())
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }

    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        match id {
            Some(i) if !i.is_empty() => {
                self.trash_id(i)?;
                self.forget_path_prefix(path);
                Ok(())
            }
            _ => self.trash(path),
        }
    }

    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        let root_n = norm(root);
        let mut removed = 0usize;
        let mut stack = vec![root_n.clone()];
        while let Some(dir) = stack.pop() {
            let dir_rel = if root_n.is_empty() {
                dir.clone()
            } else {
                dir.strip_prefix(&root_n)
                    .unwrap_or("")
                    .trim_start_matches('/')
                    .to_string()
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
                group.sort_by_key(|m| std::cmp::Reverse(m.mtime_ms));
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

    fn provides_content_hash(&self) -> bool {
        // Drive returns `md5Checksum` in the file listing (binary files) - a free
        // content hash, no download. Lets sync compare by content even in the
        // size+mtime mode, so files whose mtime differs but content matches are
        // not re-transferred. (Google-native Docs have no md5 -> content_md5 None
        // -> those gracefully fall back to size+mtime.)
        true
    }

    fn supports_changes(&self) -> bool {
        true
    }

    fn change_root_id(&self, root: &str) -> VfsResult<Option<String>> {
        self.resolve(root).map(Some)
    }

    fn current_change_cursor(&self, _root: &str) -> VfsResult<Option<String>> {
        self.start_page_token().map(Some)
    }

    fn changes_since(&self, _root: &str, cursor: &str) -> VfsResult<crate::vfs::VfsChangeBatch> {
        self.drive_changes_since(cursor)
    }
}
