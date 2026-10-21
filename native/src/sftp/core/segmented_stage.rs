//! A stage several writers fill at once (`SegmentedStage`): the file of a
//! large upload is cut into ranges, each range goes over its own pool
//! channel (its own SSH window) to its own offset of one stage.
//!
//! The stage is created exclusively, once, and closed again (`OPEN` with
//! `WRITE | CREATE | EXCL`, `CLOSE`); every range reopens it by path with
//! `WRITE` only (no `CREATE`, no `TRUNCATE`), so no range can create or
//! cut a file. SFTP v3 has no `O_NOFOLLOW` and no inode in the attributes,
//! so between the creation and a reopening the name could be made to name
//! another file that this account may write (security question S7). The
//! answer: ranges exist only where nobody but this account and root can
//! rename or replace an entry on the way (`chain_is_safe`: every folder from
//! the parent of the stage up to `/` belongs to this account or to root and
//! is not writable by group or others, or is sticky and belongs to root);
//! where an owner or the mode bits are not stated, the file goes as one
//! stream. The check of the chain is made once per folder
//! (`SftpBackend::stage_folders`). As a second line every range handle is
//! looked at with `fstat` before its first byte (`HandleCheck`): a regular
//! file of this account that is no longer than the stage.
use super::backend::{create_new_flags, SftpBackend};
use super::io_adapters::Commit;
use crate::vfs::{Backend, SegmentedStage, VfsResult};
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Ranges of an upload over several pool channels (security answer S7: only
/// with the check of the folder chain, `chain_is_safe`).
pub(super) const SFTP_SEGMENTED_WRITE: bool = true;

/// How long the verdict on a folder chain holds: the files of one job share
/// it, a later job looks again.
pub(super) const CHAIN_MEMORY: Duration = Duration::from_secs(30);

/// `S_IFMT` and `S_IFDIR` of a mode, `S_ISVTX` and the write bits of group
/// and others.
const TYPE_MASK: u32 = 0o170_000;
const TYPE_DIR: u32 = 0o040_000;
const TYPE_REGULAR: u32 = 0o100_000;
const STICKY: u32 = 0o1000;
const GROUP_OTHER_WRITE: u32 = 0o022;

/// What a server says of one folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FolderFacts {
    pub uid: Option<u32>,
    pub mode: Option<u32>,
}

/// Whether nobody but `account` and root can rename or replace an entry of
/// `folder`: it belongs to one of them and either group and others cannot
/// write it, or it is sticky and belongs to root (the sticky bit lets only
/// the owner of an entry, or of the folder, remove or rename it). A folder
/// whose owner or mode is not stated does not count.
pub(super) fn folder_is_safe(account: u32, folder: FolderFacts) -> bool {
    let (Some(uid), Some(mode)) = (folder.uid, folder.mode) else {
        return false;
    };
    if mode & TYPE_MASK != TYPE_DIR {
        return false;
    }
    let ours = uid == account || uid == 0;
    let closed = mode & GROUP_OTHER_WRITE == 0;
    let sticky_root = mode & STICKY != 0 && uid == 0;
    ours && (closed || sticky_root)
}

/// Every folder of the chain, the stage's parent first, up to `/`.
pub(super) fn chain_is_safe(account: u32, chain: &[FolderFacts]) -> bool {
    !chain.is_empty() && chain.iter().all(|folder| folder_is_safe(account, *folder))
}

/// Whether the mode names a regular file. (`FileAttributes::is_regular` tests
/// one bit and also accepts a link, whose mode contains it.)
fn is_regular(attrs: &FileAttributes) -> bool {
    attrs
        .permissions
        .is_some_and(|mode| mode & TYPE_MASK == TYPE_REGULAR)
}

/// What a range handle must look like before the first byte is written.
#[derive(Clone, Copy, Debug)]
pub(super) struct HandleCheck {
    /// The account that created the stage.
    pub uid: u32,
    /// The final length of the stage: no handle may show more.
    pub size: u64,
}

impl HandleCheck {
    pub(super) fn verify(&self, attrs: &FileAttributes) -> io::Result<()> {
        let regular = is_regular(attrs);
        let ours = attrs.uid == Some(self.uid);
        let fits = attrs.size.is_some_and(|size| size <= self.size);
        if regular && ours && fits {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Die Upload-Stufe ist nicht mehr die Datei, die angelegt wurde; sie wird nicht beschrieben",
        ))
    }
}

/// What the stage needs from the connection; `SftpBackend` is the real one,
/// tests bring a store in memory.
pub(super) trait StageWire: Clone + Send + Sync + 'static {
    /// Creates `stage` exclusively and closes it again. `Ok(false)`: no pool
    /// channel can serve (the file streams over the main session);
    /// `AlreadyExists` when the name is taken.
    fn create(&self, stage: &str) -> io::Result<bool>;
    /// The account the server shows as the owner of the new, regular `stage`;
    /// `None` when it states none.
    fn account_of(&self, stage: &str) -> io::Result<Option<u32>>;
    /// The folders from `parent` (resolved) up to `/`; `None` when the server
    /// cannot say.
    fn folder_chain(&self, parent: &str) -> io::Result<Option<Vec<FolderFacts>>>;
    /// An earlier verdict on the chain of `parent`, still valid.
    fn verdict(&self, parent: &str) -> Option<bool>;
    fn remember(&self, parent: &str, safe: bool);
    /// Writer for exactly `[offset, offset + len)` of the existing `stage`,
    /// on a channel of its own, after the handle passed `check`: `flush`
    /// fails unless `len` bytes came.
    fn range_writer(
        &self,
        stage: &str,
        offset: u64,
        len: u64,
        check: HandleCheck,
    ) -> io::Result<Box<dyn Write + Send>>;
    /// The length of `stage` as the server states it.
    fn size_of(&self, stage: &str) -> io::Result<u64>;
    /// Removes `stage`, if it is a file.
    fn discard(&self, stage: &str) -> io::Result<()>;
}

/// The stage of one upload: `size` bytes in `stage`, created by `account`.
pub(super) struct SftpSegmentedStage<W: StageWire> {
    wire: W,
    stage: String,
    size: u64,
    account: u32,
}

/// The folder a stage lies in; `.` for a relative name without folder.
pub(super) fn parent_of(stage: &str) -> &str {
    match stage.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => ".",
    }
}

/// Opens the stage of an upload of `size` bytes if `enabled`, a pool channel
/// can serve and the folder chain is safe; `Ok(None)` otherwise (the file
/// streams, no error).
pub(super) fn open_with<W: StageWire>(
    wire: &W,
    stage: &str,
    size: u64,
    enabled: bool,
) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
    if !enabled {
        return Ok(None);
    }
    let parent = parent_of(stage);
    let known = wire.verdict(parent);
    if known == Some(false) {
        return Ok(None);
    }
    if !wire.create(stage)? {
        return Ok(None);
    }
    let account = match wire.account_of(stage) {
        Ok(Some(account)) => account,
        // The server states no owner: nothing proves the stage stays ours.
        Ok(None) => return refuse(wire, stage, parent, known),
        Err(error) => {
            let _ = wire.discard(stage);
            return Err(error);
        }
    };
    if known != Some(true) {
        match wire.folder_chain(parent) {
            Ok(Some(chain)) if chain_is_safe(account, &chain) => wire.remember(parent, true),
            Ok(_) => return refuse(wire, stage, parent, known),
            Err(error) => {
                let _ = wire.discard(stage);
                return Err(error);
            }
        }
    }
    Ok(Some(Box::new(SftpSegmentedStage {
        wire: wire.clone(),
        stage: stage.to_string(),
        size,
        account,
    })))
}

/// The chain is not safe (or cannot be judged): the stage that was just
/// created goes, the verdict is remembered, the file streams.
fn refuse<W: StageWire>(
    wire: &W,
    stage: &str,
    parent: &str,
    known: Option<bool>,
) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
    let _ = wire.discard(stage);
    if known.is_none() {
        wire.remember(parent, false);
    }
    Ok(None)
}

impl<W: StageWire> SegmentedStage for SftpSegmentedStage<W> {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        if offset.checked_add(len).is_none_or(|end| end > self.size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Bereich {offset}+{len} liegt hinter dem Ende der Stufe ({} Bytes)",
                    self.size
                ),
            ));
        }
        let check = HandleCheck {
            uid: self.account,
            size: self.size,
        };
        self.wire.range_writer(&self.stage, offset, len, check)
    }

    fn finish(self: Box<Self>) -> VfsResult<()> {
        let size = self.wire.size_of(&self.stage)?;
        if size != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Stufe „{}“ hat {size} statt {} Bytes: die Bereiche sind unvollständig",
                    self.stage, self.size
                ),
            ));
        }
        Ok(())
    }

    fn abort(self: Box<Self>) {
        // Only a file this client created: `discard` removes files only.
        let _ = self.wire.discard(&self.stage);
    }
}

impl StageWire for SftpBackend {
    fn create(&self, stage: &str) -> io::Result<bool> {
        let opened = self.open_pool_writer(stage, create_new_flags(), Some(0), Commit::Unsynced);
        let mut writer = match opened {
            Ok(Some(writer)) => writer,
            Ok(None) => return Ok(false),
            Err(error) => return Err(self.exclusive_failed(stage, error)),
        };
        writer.flush()?;
        Ok(true)
    }

    fn account_of(&self, stage: &str) -> io::Result<Option<u32>> {
        let attrs = self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().symlink_metadata(stage.to_string()))
        })?;
        Ok(is_regular(&attrs).then_some(attrs.uid).flatten())
    }

    fn folder_chain(&self, parent: &str) -> io::Result<Option<Vec<FolderFacts>>> {
        let resolved = self.connection.safe_metadata(|generation| {
            Box::pin(generation.sftp().canonicalize(parent.to_string()))
        })?;
        if !resolved.starts_with('/') {
            return Ok(None);
        }
        let mut chain = Vec::new();
        let mut folder = resolved.trim_end_matches('/').to_string();
        loop {
            let path = if folder.is_empty() {
                "/".to_string()
            } else {
                folder.clone()
            };
            let attrs = self
                .connection
                .safe_metadata(|generation| Box::pin(generation.sftp().metadata(path.clone())))?;
            chain.push(FolderFacts {
                uid: attrs.uid,
                mode: attrs.permissions,
            });
            if folder.is_empty() {
                return Ok(Some(chain));
            }
            folder = match folder.rsplit_once('/') {
                Some((above, _)) => above.to_string(),
                None => String::new(),
            };
        }
    }

    fn verdict(&self, parent: &str) -> Option<bool> {
        let mut known = self
            .stage_folders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.retain(|_, (_, when)| when.elapsed() < CHAIN_MEMORY);
        known.get(parent).map(|(safe, _)| *safe)
    }

    fn remember(&self, parent: &str, safe: bool) {
        self.stage_folders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(parent.to_string(), (safe, Instant::now()));
    }

    fn range_writer(
        &self,
        stage: &str,
        offset: u64,
        len: u64,
        check: HandleCheck,
    ) -> io::Result<Box<dyn Write + Send>> {
        let opened = self
            .open_pool_writer_at(
                stage,
                OpenFlags::WRITE,
                offset,
                Some(len),
                Commit::Unsynced,
                Some(check),
            )
            .map_err(|error| self.target_error(stage, error))?;
        match opened {
            Some(writer) => Ok(self.target_writer(stage, Box::new(writer))),
            None => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Kein SFTP-Übertragungskanal für einen Bereich verfügbar",
            )),
        }
    }

    fn size_of(&self, stage: &str) -> io::Result<u64> {
        Backend::stat(self, stage).map(|meta| meta.size)
    }

    fn discard(&self, stage: &str) -> io::Result<()> {
        self.discard_stage(stage)
    }
}

impl SftpBackend {
    /// An exclusive OPEN that failed. SFTP v3 has no status for "exists",
    /// a server answers a plain failure; when the name is there, it is taken.
    fn exclusive_failed(&self, stage: &str, error: io::Error) -> io::Error {
        if error.kind() == io::ErrorKind::Other && self.try_exists(stage).unwrap_or(false) {
            return io::Error::new(io::ErrorKind::AlreadyExists, error);
        }
        self.target_error(stage, error)
    }

    /// Stage for a large upload several channels fill; see the module text.
    pub(super) fn segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        open_with(self, stage, size, SFTP_SEGMENTED_WRITE)
    }
}
