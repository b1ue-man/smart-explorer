use super::listing;
use super::reader::SmbReader;
use super::replace;
use super::session::{SmbConfig, SmbSession};
use super::streams::SmbWriter;
use super::url::{entry_path, rename_paths, split_path, SmbPath};
use super::wire::{self, DeleteKind};
use crate::vfs::{Backend, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult};
use smb2::ErrorKind;
use std::io::{self, Read, Write};
use std::sync::Arc;

#[derive(Clone)]
pub struct SmbBackend {
    session: Arc<SmbSession>,
    /// The share of the connection root: the only entry at the server level.
    share: String,
    root: String,
    /// `smb://user@host:port/root`, the persisted-state identity.
    url: String,
    /// `smb://user@host:port`, independent of the start folder.
    namespace: String,
}

fn name_of(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

impl SmbBackend {
    /// Connects, signs in and connects the share of `config.root` (a wrong
    /// share name fails the connect with `NotFound`).
    pub fn connect(config: SmbConfig) -> io::Result<SmbBackend> {
        let share = match split_path(&config.root)? {
            Some(root) => root.share,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Bei SMB beginnt der Startordner mit der Freigabe („/freigabe/ordner“)",
                ))
            }
        };
        let namespace = format!("smb://{}@{}:{}", config.user, config.host, config.port);
        let url = format!("{namespace}{}", config.root);
        let root = config.root.clone();
        let session = SmbSession::connect(config, &share)?;
        Ok(SmbBackend {
            session,
            share,
            root,
            url,
            namespace,
        })
    }

    fn write_file(
        &self,
        path: &str,
        exclusive: bool,
        expected: Option<u64>,
    ) -> VfsResult<Box<dyn Write + Send>> {
        let target = entry_path(path)?;
        let (generation, tree) = self.session.target(&target.share)?;
        let opened = self.session.block_on(async {
            if exclusive {
                tree.create_file_writer_exclusive(generation.connection(), &target.rel)
                    .await
            } else {
                tree.create_file_writer(generation.connection(), &target.rel)
                    .await
            }
        });
        let writer = opened.map_err(|error| {
            generation.note(&error);
            super::errors::map(error, "Datei anlegen", path)
        })?;
        Ok(Box::new(SmbWriter::new(
            writer,
            &target.rel,
            path,
            tree,
            generation,
            self.session.runtime(),
            expected,
        )))
    }

    /// A reader from byte `start`; a connection lost before any byte was
    /// read is replaced and the open repeated once.
    fn reader_at(&self, path: &str, start: u64) -> VfsResult<Box<dyn Read + Send>> {
        let target = entry_path(path)?;
        let mut retried = false;
        loop {
            let (generation, tree) = self.session.target(&target.share)?;
            let opened = self
                .session
                .block_on(tree.open_file_reader(generation.connection(), &target.rel));
            match opened {
                Ok(reader) => {
                    return Ok(Box::new(SmbReader::new(
                        reader,
                        start,
                        path,
                        generation,
                        self.session.runtime(),
                    )))
                }
                Err(error) if generation.note(&error) && !retried => retried = true,
                Err(error) => return Err(super::errors::map(error, "Öffnen", path)),
            }
        }
    }

    /// One folder in one round trip (`create_dir`: an existing real folder
    /// is fine; `create_dir_new`: any existing entry is `AlreadyExists`).
    fn make_dir(&self, path: &str, exclusive: bool) -> VfsResult<()> {
        let target = match split_path(path)? {
            Some(target) if !target.rel.is_empty() => target,
            // The server level and the share root always exist.
            _ if exclusive => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("Existiert bereits: {path}"),
                ))
            }
            _ => return self.mkdir_all(path),
        };
        let rel = target.rel;
        self.session.write(
            &target.share,
            "Ordner anlegen",
            path,
            |conn, tree| async move {
                match wire::create_directory(&conn, &tree, &rel).await {
                    Err(error) if !exclusive && error.kind() == ErrorKind::AlreadyExists => {
                        let attributes = wire::stat(&conn, &tree, &rel).await?;
                        if attributes.is_dir() && !attributes.is_link() {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    }
                    result => result,
                }
            },
        )
    }

    fn rename_with(&self, src: &str, dst: &str, replace_existing: bool) -> VfsResult<()> {
        let (from, to): (SmbPath, SmbPath) = rename_paths(src, dst)?;
        let (from_rel, to_rel) = (from.rel, to.rel);
        self.session
            .write(&from.share, "Umbenennen", src, |conn, tree| async move {
                replace::rename(&conn, &tree, &from_rel, &to_rel, replace_existing).await
            })
    }

    fn delete(&self, path: &str, kind: DeleteKind) -> VfsResult<()> {
        let target = entry_path(path)?;
        let rel = target.rel;
        self.session
            .write(&target.share, "Löschen", path, |conn, tree| async move {
                wire::delete(&conn, &tree, &rel, kind).await
            })
    }
}

impl Backend for SmbBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Smb
    }

    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn state_identity(&self) -> String {
        self.url.clone()
    }

    fn namespace_identity(&self) -> String {
        self.namespace.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let Some(target) = split_path(path)? else {
            return Ok(listing::server_level(&self.share));
        };
        let rel = target.rel;
        self.session
            .read(&target.share, "Ordner lesen", path, |conn, tree| {
                let rel = rel.clone();
                async move { wire::list(&conn, &tree, &rel).await }
            })
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let Some(target) = split_path(path)? else {
            return Ok(VfsMeta {
                is_dir: true,
                ..VfsMeta::default()
            });
        };
        let rel = target.rel;
        let attributes =
            self.session
                .read(&target.share, "Eigenschaften lesen", path, |conn, tree| {
                    let rel = rel.clone();
                    async move { wire::stat(&conn, &tree, &rel).await }
                })?;
        Ok(listing::meta(name_of(path), &attributes))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reader_at(path, 0)
    }

    /// READs are positioned, so a resumed download starts at `offset`.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let _ = id;
        self.reader_at(path, offset).map(Some)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, false, None)
    }

    /// One CREATE with `FileCreate`: an existing name fails with
    /// `AlreadyExists` instead of being truncated.
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, true, None)
    }

    /// The exclusive stage streams anyway; its known length is enforced at
    /// `flush`, so a source that changed size never publishes.
    fn open_write_copy_stage_sized(
        &self,
        path: &str,
        size: u64,
    ) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, true, Some(size))
    }

    /// ReplaceIfExists = 1: an existing file is replaced in one step.
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_with(src, dst, true)
    }

    /// ReplaceIfExists = 0: the server refuses an existing target.
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_with(src, dst, false)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.delete(path, DeleteKind::File)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.delete(path, DeleteKind::Directory)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let Some(target) = split_path(path)? else {
            return Ok(());
        };
        let rel = target.rel;
        self.session.write(
            &target.share,
            "Ordner anlegen",
            path,
            |mut conn, tree| async move {
                let mut current = String::new();
                for part in rel.split('/').filter(|part| !part.is_empty()) {
                    if !current.is_empty() {
                        current.push('/');
                    }
                    current.push_str(part);
                    match tree.create_directory(&mut conn, &current).await {
                        Err(error) if error.kind() != ErrorKind::AlreadyExists => {
                            return Err(error)
                        }
                        _ => {}
                    }
                }
                // The share root always exists; a name taken by a file must
                // not pass as the created folder.
                let attributes = wire::stat(&conn, &tree, &rel).await?;
                if !attributes.is_dir() {
                    return Err(wire::protocol(
                        smb2::types::status::NtStatus::OBJECT_NAME_COLLISION,
                        smb2::types::Command::Create,
                    ));
                }
                Ok::<(), smb2::Error>(())
            },
        )
    }

    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, false)
    }

    /// The CREATE with `FileCreate` refuses an existing name atomically.
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.make_dir(path, true)
    }

    /// Stages are created with `FileCreate`, so the name is this client's
    /// until it is published; only a regular file is removed.
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        match self.stat(stage) {
            Ok(meta) if !meta.is_dir && !meta.is_symlink => self.remove_file(stage),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Übertragungsstufe {stage} ist keine reguläre Datei und bleibt stehen"),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn parallelism(&self) -> usize {
        // One session multiplexes requests; two keep a walk moving without
        // starving the credit window of a small NAS.
        2
    }

    /// All transfers over this session share one controller, also the
    /// clones of this backend.
    fn flow_key(&self, path: &str) -> String {
        let _ = path;
        format!("{}#{:p}", self.namespace, Arc::as_ptr(&self.session))
    }

    /// Replacing is one ReplaceIfExists rename and new files are created
    /// exclusively, so every staged-write guarantee holds. Drive mounts
    /// still refuse SMB explicitly (`daemon` mount source).
    fn rename_overwrites(&self) -> bool {
        true
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }
}
