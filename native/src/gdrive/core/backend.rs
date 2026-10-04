use super::api::{export_ext, export_format};
use super::core::{cloud_urlenc, norm};
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

    fn state_identity(&self) -> String {
        // The permission ID identifies the account through refresh-token
        // rotation. Persisted path-v2 locators retain their existing meaning.
        format!("gdrive:path-v2:{}:{}", self.drive_account_key, self.root)
    }

    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let (raw, folders) = self.projected_listing(path)?;
        let unprojected = raw.iter().filter(|entry| !entry.meta.is_dir
            || !entry.meta.id.as_deref().is_some_and(|id| folders.contains_key(id)))
            .map(|entry| entry.meta.clone()).collect();
        let mut listed = Vec::with_capacity(raw.len());
        let mut used = std::collections::HashSet::new();
        for entry in raw.iter().filter(|entry| entry.meta.is_dir) {
            let Some(folder) = entry.meta.id.as_deref().and_then(|id| folders.get(id)) else {
                continue;
            };
            let mut meta = entry.meta.clone();
            meta.name = folder.segment.clone();
            used.insert(meta.name.clone());
            listed.push(meta);
        }
        // Titles outside the sync component contract remain browsable through
        // the established encoded namespace, including duplicate directories.
        for mut file in super::duplicates::disambiguate(unprojected) {
            if used.contains(&file.name) {
                let title = raw.iter().find(|entry| entry.meta.id == file.id)
                    .map(|entry| entry.meta.name.as_str())
                    .ok_or_else(|| std::io::Error::other("Drive browser file has no identity"))?;
                let id = file.id.as_deref().ok_or_else(|| std::io::Error::other("Drive browser file has no ID"))?;
                file.name = format!("{}{}{}{}", super::names::encode(title),
                    super::duplicates::MARKER_PREFIX, id, super::duplicates::MARKER_SUFFIX);
            }
            used.insert(file.name.clone());
            listed.push(file);
        }
        let mimes: HashMap<_, _> = raw.iter().filter_map(|entry| {
            Some((entry.meta.id.as_deref()?, entry.mime.as_deref()?))
        }).collect();
        let mut names = std::collections::HashSet::new();
        for entry in &listed {
            crate::vfs::validate_child_name(&entry.name)?;
            if !names.insert(&entry.name) {
                return Err(std::io::Error::other("Drive returned conflicting path names"));
            }
            let id = entry.id.as_deref().ok_or_else(|| std::io::Error::other("Drive browser entry has no ID"))?;
            self.remember_path(&super::sync_projection::child_key(path, &entry.name), id, mimes.get(id).copied())?;
        }
        self.listed_guard()?.insert(norm(path));
        self.persist_path_cache();
        Ok(listed)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stat_marker_aware(path)
    }

    fn has_duplicate_file_names(&self) -> bool {
        true
    }

    fn list_dir_for_sync(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let listing = self.sync_listing(path)?;
        if !listing.omitted.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Drive sync listing has protected omissions; use the tolerant listing",
            ));
        }
        Ok(listing.entries)
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
        // A download right after a listing costs one request: the listing
        // cached this exact ID's type.
        let mime = self.mime_for(path, &id).unwrap_or_default();
        let url = if let Some(fmt) = export_format(&mime) {
            self.api_url(&format!(
                "files/{}/export?mimeType={}",
                cloud_urlenc(&id),
                cloud_urlenc(fmt)
            ))
        } else {
            self.api_url(&format!("files/{}?alt=media", cloud_urlenc(&id)))
        };
        let resp = self.authenticated_stream(&url, None)?;
        Ok(Box::new(resp.into_reader()))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let id = self.resolve(path)?;
        // Google-Docs editors files (Docs/Sheets/Slides/Drawings) have no binary
        // content and 403 on alt=media ("fileNotDownloadable") - they must be
        // EXPORTED to an Office/PDF format instead.
        let mime = self.mime_of(path).unwrap_or_default();
        let url = if let Some(fmt) = export_format(&mime) {
            self.api_url(&format!(
                "files/{}/export?mimeType={}",
                cloud_urlenc(&id),
                cloud_urlenc(fmt)
            ))
        } else {
            self.api_url(&format!("files/{}?alt=media", cloud_urlenc(&id)))
        };
        let resp = self.authenticated_stream(&url, None)?;
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

    fn read_size(&self, path: &str, metadata_size: u64) -> VfsResult<Option<u64>> {
        let mime = self.mime_of(path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Drive read type could not be determined",
            )
        })?;
        Ok(export_format(&mime).is_none().then_some(metadata_size))
    }

    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        super::copy_writer::open_writer(self, path)
    }

    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        super::sized_writer::open_stage(self, path, size)
    }

    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        self.copy_to_stage(src, stage, size)
    }

    fn open_write_fresh(&self, path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>> {
        super::sized_writer::open_fresh(self, path, size).map(Some)
    }

    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        if offset == 0 {
            return self.open_read_id(path, id).map(Some);
        }
        self.read_from(path, id, offset)
    }

    fn transfer_hint(&self) -> Option<String> {
        // Drive's documented sustained write rate per account (ref §2); it
        // cannot be raised, so it is worth showing next to a slow upload.
        Some("Google Drive nimmt höchstens etwa 3 neue Dateien pro Sekunde an".to_string())
    }

    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.create_dir_exclusive(path)
    }

    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        self.discard_stage(stage)
    }

    fn flow_key(&self, _path: &str) -> String {
        // Quotas and the write rate apply per Drive account, so every
        // connection (and clone) of one account shares one adaptive flow.
        format!("gdrive:{}", self.drive_account_key)
    }

    fn namespace_identity(&self) -> String {
        // Paths are absolute in My Drive whatever the start folder, so all
        // connections of one account share one namespace (overlap checks,
        // server-side copies).
        format!("gdrive:path-v2:{}", self.drive_account_key)
    }

    fn promote_copy_stage(&self, staged: &str, destination: &str) -> VfsResult<()> {
        // Drive names are not unique. This exact-ID move verifies uniqueness
        // and rolls back collisions; it is not an atomic name reservation.
        self.promote_staged_file_no_replace(staged, destination)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_serialized(src, dst)
    }

    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote_staged_file(staged, destination)
    }
    fn promote_staged_to_id(
        &self,
        staged: &str,
        destination: &str,
        id: Option<&str>,
    ) -> VfsResult<()> {
        self.promote_staged_file_to_id(staged, destination, id)
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

impl GDriveBackend {
    /// The former token-keyed identity, solely to migrate an existing
    /// baseline belonging to the currently authenticated account. It cannot
    /// recover identities of tokens that were already rotated and lost.
    pub(crate) fn legacy_state_identity(&self) -> VfsResult<String> {
        use sha2::{Digest, Sha256};
        let tokens = self.tokens_guard()?;
        let digest = Sha256::digest(tokens.refresh_token.as_bytes());
        let account = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(format!("gdrive:path-v2:{account}:{}", self.root))
    }

}
