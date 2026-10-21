//! A stage several writers fill at once (`SegmentedStage`) for SMB. The stage
//! is created exclusively once, given its final length, and kept open by the
//! creator without `FILE_SHARE_DELETE` until `finish` or `abort`; every range
//! reopens it by path with its own handle and writes at its offset (the
//! wire is in segment_wire.rs: smb2's `create_file_writer_at` opens with
//! `FileOpenIf`, which would create a missing name, and shows neither the
//! file's identity nor the `CreateAction`).
//!
//! Reopening by path is safe only because a handle is bound to its object and
//! the object's identity can be read from the handle (security answer S14).
//! At creation the identity is noted: `FileIdInformation` (volume serial and
//! 128-bit file id) when the server answers that class, otherwise
//! `FileInternalInformation.IndexNumber` with the `CreationTime` of the
//! CREATE answer (Samba 4.20 does not know class 59). A range handle asks for
//! the same, and for `FileStandardInformation`, in the compound of its
//! CREATE, and writes its first byte only when it is the same file, not a
//! folder, no reparse point, not delete-pending, of exactly the stage's
//! length, and opened rather than created. Otherwise the handle is closed
//! unwritten, the file goes as one stream, and this connection offers no
//! ranges for 15 minutes. `finish` makes the same check at a handle of its
//! own. A server that gives no usable identity (missing or 0) never gets
//! ranges.
use super::backend::SmbBackend;
use crate::vfs::{SegmentedStage, VfsResult};
use smb2::msg::create::CreateAction;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Ranges of an upload through handles of their own (security answer S14).
pub(super) const SMB_SEGMENTED_WRITE: bool = true;

/// After a deviation or a refusal this connection offers no ranges for this
/// long.
pub(super) const NO_RANGES_FOR: Duration = Duration::from_secs(15 * 60);

pub(super) const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub(super) const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Which file a handle points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Identity {
    /// `FileIdInformation` (MS-FSCC 2.4.20): volume serial and file id.
    FileId { volume: u64, id: [u8; 16] },
    /// `FileInternalInformation` (2.4.22) with the creation time of the
    /// CREATE answer.
    Index { index: u64, created: u64 },
}

impl Identity {
    /// A usable identity tells files apart: not 0.
    pub(super) fn is_usable(&self) -> bool {
        match self {
            Identity::FileId { id, .. } => id.iter().any(|byte| *byte != 0),
            Identity::Index { index, .. } => *index != 0,
        }
    }
}

/// The identity from the answers to the two queries; the file id wins when
/// the server answered that class. `None`: neither was answered.
pub(super) fn identity_from(
    file_id: Option<&[u8]>,
    index: Option<&[u8]>,
    created: u64,
) -> Option<Identity> {
    if let Some(bytes) = file_id.filter(|bytes| bytes.len() >= 24) {
        let volume = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let id: [u8; 16] = bytes[8..24].try_into().ok()?;
        return Some(Identity::FileId { volume, id });
    }
    let bytes = index.filter(|bytes| bytes.len() >= 8)?;
    let index = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    Some(Identity::Index { index, created })
}

/// `FileStandardInformation` (MS-FSCC 2.4.41): end of file, delete pending,
/// directory.
pub(super) fn standard_from(bytes: &[u8]) -> Option<(u64, bool, bool)> {
    if bytes.len() < 22 {
        return None;
    }
    let end_of_file = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    Some((end_of_file, bytes[20] != 0, bytes[21] != 0))
}

/// What a handle shows right after its CREATE.
#[derive(Clone, Copy, Debug)]
pub(super) struct Facts {
    pub identity: Option<Identity>,
    pub action: CreateAction,
    pub attributes: u32,
    pub end_of_file: u64,
    pub directory: bool,
    pub delete_pending: bool,
}

/// How a handle differs from the stage that was created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Deviation {
    Created,
    Directory,
    ReparsePoint,
    DeletePending,
    Size,
    Identity,
}

impl Deviation {
    fn text(self) -> &'static str {
        match self {
            Deviation::Created => "der Name wurde neu angelegt statt geöffnet",
            Deviation::Directory => "der Name bezeichnet einen Ordner",
            Deviation::ReparsePoint => "der Name bezeichnet einen Reparse-Punkt",
            Deviation::DeletePending => "die Stufe ist zum Löschen vorgemerkt",
            Deviation::Size => "die Stufe hat nicht die gesetzte Länge",
            Deviation::Identity => "der Name bezeichnet nicht mehr die angelegte Datei",
        }
    }
}

/// Whether a handle is the stage that was created, of its length.
pub(super) fn judge(stage: &Identity, size: u64, facts: &Facts) -> Result<(), Deviation> {
    if facts.action != CreateAction::FileOpened {
        return Err(Deviation::Created);
    }
    if facts.directory || facts.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(Deviation::Directory);
    }
    if facts.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Deviation::ReparsePoint);
    }
    if facts.delete_pending {
        return Err(Deviation::DeletePending);
    }
    if facts.end_of_file != size {
        return Err(Deviation::Size);
    }
    if facts.identity != Some(*stage) {
        return Err(Deviation::Identity);
    }
    Ok(())
}

/// The creator's handle, kept open until the stage is finished or given up;
/// dropping it closes it.
pub(super) struct Created {
    pub identity: Option<Identity>,
    pub hold: Box<dyn Send + Sync>,
}

/// A range handle that passed (or failed) the checks; dropping it closes it
/// without a byte written.
pub(super) trait RangeHandle: Send {
    fn into_writer(self: Box<Self>, offset: u64, len: u64) -> Box<dyn Write + Send>;
}

pub(super) struct RangeOpen {
    pub facts: Facts,
    pub handle: Box<dyn RangeHandle>,
}

/// What the stage needs from the connection; `SmbBackend` is the real one
/// (segment_wire.rs), tests bring a store in memory.
pub(super) trait StageWire: Clone + Send + Sync + 'static {
    /// Creates `stage` exclusively, sets its length to `size`, keeps it open.
    /// `AlreadyExists` when the name is taken; `Ok(None)` when the length
    /// cannot be set (the file streams).
    fn create(&self, stage: &str, size: u64) -> io::Result<Option<Created>>;
    /// Opens the existing `stage` for a range (never creates it) and asks
    /// for its facts. A refusal that means "no ranges here" (a sharing
    /// violation) is `Unsupported`.
    fn open_range(&self, stage: &str) -> io::Result<RangeOpen>;
    /// Removes `stage`, if it is a file.
    fn discard(&self, stage: &str) -> io::Result<()>;
    /// Whether this connection offers no ranges at the moment.
    fn refused(&self) -> bool;
    /// Remembers for `NO_RANGES_FOR` that this connection offers no ranges.
    fn refuse(&self);
}

/// The stage of one upload: `size` bytes in `stage`, created as `identity`.
pub(super) struct SmbSegmentedStage<W: StageWire> {
    wire: W,
    stage: String,
    size: u64,
    identity: Identity,
    hold: Option<Box<dyn Send + Sync>>,
    /// A range was written to: from then on a deviation is an error that
    /// ends this attempt (and the retry streams), not a fallback.
    opened: AtomicBool,
}

/// Opens the stage of an upload of `size` bytes if `enabled`, this
/// connection has not turned ranges down and the server gives the stage an
/// identity; `Ok(None)` otherwise (the file streams, no error).
pub(super) fn open_with<W: StageWire>(
    wire: &W,
    stage: &str,
    size: u64,
    enabled: bool,
) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
    if !enabled || wire.refused() {
        return Ok(None);
    }
    let Some(created) = wire.create(stage, size)? else {
        return Ok(None);
    };
    let Some(identity) = created.identity.filter(Identity::is_usable) else {
        // The server names no file: nothing could prove the stage stays ours.
        drop(created.hold);
        let _ = wire.discard(stage);
        return Ok(None);
    };
    Ok(Some(Box::new(SmbSegmentedStage {
        wire: wire.clone(),
        stage: stage.to_string(),
        size,
        identity,
        hold: Some(created.hold),
        opened: AtomicBool::new(false),
    })))
}

impl<W: StageWire> SmbSegmentedStage<W> {
    /// Opens a handle and checks it. Any deviation, a vanished stage or a
    /// sharing violation closes the handle unwritten, makes this connection
    /// offer no ranges and ends in the error of `refusal`.
    fn checked(&self) -> io::Result<RangeOpen> {
        let open = match self.wire.open_range(&self.stage) {
            Ok(open) => open,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Unsupported | io::ErrorKind::NotFound
                ) =>
            {
                self.wire.refuse();
                return Err(self.refusal(&error.to_string()));
            }
            Err(error) => return Err(error),
        };
        match judge(&self.identity, self.size, &open.facts) {
            Ok(()) => Ok(open),
            Err(deviation) => {
                drop(open.handle);
                if deviation == Deviation::Created {
                    // Never expected after FILE_OPEN: the name is ours now.
                    let _ = self.wire.discard(&self.stage);
                }
                self.wire.refuse();
                Err(self.refusal(deviation.text()))
            }
        }
    }

    /// Before any range was written the engine streams the file at once
    /// (`Unsupported`); later it ends the attempt and the retry streams.
    fn refusal(&self, why: &str) -> io::Error {
        let kind = if self.opened.load(Ordering::Acquire) {
            io::ErrorKind::ConnectionAborted
        } else {
            io::ErrorKind::Unsupported
        };
        io::Error::new(
            kind,
            format!(
                "SMB-Stufe „{}“ nimmt keine Bereiche: {why}; die Datei geht als ein Strom",
                self.stage
            ),
        )
    }
}

impl<W: StageWire> SegmentedStage for SmbSegmentedStage<W> {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        if offset.checked_add(len).is_none_or(|end| end > self.size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Bereich {offset}+{len} liegt hinter dem Ende der Stufe"),
            ));
        }
        let open = self.checked()?;
        self.opened.store(true, Ordering::Release);
        Ok(open.handle.into_writer(offset, len))
    }

    fn finish(mut self: Box<Self>) -> VfsResult<()> {
        // Length and identity at one handle, not a `stat` by path.
        let checked = self.checked();
        drop(self.hold.take());
        checked.map(|_| ())
    }

    fn abort(mut self: Box<Self>) {
        // The creator's handle forbids deleting: it goes first.
        drop(self.hold.take());
        let _ = self.wire.discard(&self.stage);
    }
}

impl SmbBackend {
    /// Stage for a large upload several handles fill; see the module text.
    pub(super) fn segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        open_with(self, stage, size, SMB_SEGMENTED_WRITE)
    }
}
