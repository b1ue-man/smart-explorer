use super::api::{drive_request, export_ext, export_format, open_stream, API};
use super::core::{cloud_urlenc, norm};
use super::transfer::open_writer;
use super::GDriveBackend;
use crate::vfs::{Backend, DedupeCandidate, Scheme, VfsMeta, VfsResult};
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

    fn state_identity(&self) -> String {
        use sha2::{Digest, Sha256};
        let account = self
            .tokens_guard()
            .ok()
            .map(|tokens| {
                let digest = Sha256::digest(tokens.refresh_token.as_bytes());
                digest[..12]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .unwrap_or_else(|| "token-cache-unavailable".into());
        format!("gdrive:{account}:{}", self.root)
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
        // Folder creation can use this complete snapshot to skip a redundant
        // lookup. File uploads still re-probe because Drive names are not unique.
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
        let resp = open_stream(|| {
            drive_request(
                self.stream_agent
                    .get(&url)
                    .set("Authorization", &bearer)
                    .call(),
            )
        })?;
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
        let resp = open_stream(|| {
            drive_request(
                self.stream_agent
                    .get(&url)
                    .set("Authorization", &bearer)
                    .call(),
            )
        })?;
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
        open_writer(self, path)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_serialized(src, dst)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote_staged_file(staged, destination)
    }

    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote_staged_file_no_replace(staged, destination)
    }

    fn staged_write_capabilities(&self, _root: &str) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: true,
            replace: true,
            // Drive updates an exact destination id and then cleans up the
            // staging object; it is not one atomic namespace rename.
            namespace_replace: false,
        }
    }

    fn root_confinement(&self, _root: &str) -> crate::vfs::RootConfinement {
        crate::vfs::RootConfinement::Enforced
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.trash(path)
    }

    fn delete_disposition(&self) -> crate::vfs::DeleteDisposition {
        crate::vfs::DeleteDisposition::Recycle
    }

    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        match id {
            Some(i) if !i.is_empty() => self.trash_path_id(path, i),
            _ => self.trash(path),
        }
    }

    fn plan_dedupe_recursive(
        &self,
        root: &str,
        keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<DedupeCandidate>> {
        self.plan_duplicate_cleanup(root, keep)
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
