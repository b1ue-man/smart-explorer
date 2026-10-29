//! An engine copy stage that fits into one WRITE (G3-07, 2026-10-28). The
//! exclusive CREATE still goes out when the writer is opened, so a taken
//! stage name is `AlreadyExists` right there, as the engine's stage loop
//! (`engine/publish.rs` `open_stage`) and `discard_copy_stage` ("the name is
//! this client's until it is published") expect. Its bytes are kept in
//! memory; the commit sends WRITE + CLOSE as one compound, the CLOSE with
//! `SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB`, whose answer describes the closed
//! stage, so its evidence needs no stat of its own (stage_evidence.rs). The
//! destination's stat goes out beside it. Two round trips after the open
//! (WRITE answer, CLOSE answer, then the stage's stat) become one.
//!
//! Why the CREATE is not in the compound as well: it would move "name taken"
//! from the open to `flush`, where the engine discards the stage by name and
//! would remove a file that is not this client's.
//!
//! Like the streaming writer (io.rs `Commit::Unsynced`): no FLUSH, the
//! WRITE answer must confirm every byte, a failed commit or a writer dropped
//! before it removes the stage, and the sizes must match exactly.
use super::backend::SmbBackend;
use super::batch_wire::{charge, packet_create, write_request, written};
use super::credit_retry::funded;
use super::errors;
use super::listing::Attributes;
use super::session::Generation;
use super::stage_evidence::StageReport;
use super::wire;
use crate::transfer::MemoryReservation;
use crate::vfs::VfsResult;
use smb2::msg::close::{CloseRequest, CloseResponse, SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB};
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId};
use smb2::{CompoundOp, Frame, Tree};
use std::io::{self, Write};
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Every dialect allows 64 KiB requests (MS-SMB2 3.3.5.4).
const MIN_IO: u64 = 65_536;
/// Write access as packet stages, plus reading the attributes the CLOSE
/// reports.
const ACCESS: u32 = FileAccessMask::FILE_WRITE_DATA
    | FileAccessMask::FILE_WRITE_ATTRIBUTES
    | FileAccessMask::FILE_READ_ATTRIBUTES
    | FileAccessMask::SYNCHRONIZE;

/// Whether a stage of `size` bytes goes in one WRITE on a server whose
/// MaxWriteSize is `max_write` (unknown: the 64 KiB every dialect allows).
pub(super) fn fits(size: u64, max_write: Option<u32>) -> bool {
    size <= max_write.map_or(MIN_IO, u64::from).max(MIN_IO)
}

/// The stage's attributes from a CLOSE answer; `None` unless the server
/// filled them (`SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB` echoed).
pub(super) fn closed_attributes(response: &CloseResponse) -> Option<Attributes> {
    if response.flags & SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB == 0 {
        return None;
    }
    Some(Attributes {
        attributes: response.file_attributes,
        size: response.end_of_file,
        last_write: response.last_write_time.0,
        creation: response.creation_time.0,
        reparse_tag: None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    Committed,
    Failed,
}

pub(super) struct CompoundStage {
    tree: Arc<Tree>,
    generation: Arc<Generation>,
    rt: Arc<Runtime>,
    /// The handle of the exclusive CREATE until the commit or the abort.
    file_id: Option<FileId>,
    rel: String,
    path: String,
    expected: u64,
    data: Vec<u8>,
    _reservation: MemoryReservation,
    state: State,
    report: Option<StageReport>,
}

/// The stage writer for `path` of `size` bytes, created now; `Ok(None)` when
/// it does not fit one WRITE or its memory cannot be reserved (the caller
/// streams, as before).
pub(super) fn open(
    backend: &SmbBackend,
    path: &str,
    size: u64,
) -> VfsResult<Option<Box<dyn Write + Send>>> {
    let target = super::url::entry_path(path)?;
    let (generation, tree) = backend.session.target(&target.share)?;
    let conn = generation.connection();
    if !fits(size, conn.params().map(|params| params.max_write_size)) {
        return Ok(None);
    }
    let Some(reservation) = crate::transfer::try_reserve_memory(size) else {
        return Ok(None);
    };
    let mut create = packet_create(wire::wire_path(&tree, &target.rel), true);
    create.desired_access = FileAccessMask::new(ACCESS);
    let created = backend.session.block_on(async {
        let frame = conn
            .execute(Command::Create, &create, Some(tree.tree_id))
            .await?;
        wire::created(&frame)
    });
    let created = created.map_err(|error| {
        generation.note(&error);
        errors::map(error, "Datei anlegen", path)
    })?;
    Ok(Some(Box::new(CompoundStage {
        tree,
        generation,
        rt: backend.session.runtime(),
        file_id: Some(created.file_id),
        rel: target.rel,
        path: path.to_string(),
        expected: size,
        data: Vec::with_capacity(size as usize),
        _reservation: reservation,
        state: State::Open,
        report: StageReport::for_stage(path, &backend.written),
    })))
}

impl CompoundStage {
    fn check_open(&self) -> io::Result<()> {
        match self.state {
            State::Open => Ok(()),
            State::Committed => Err(io::Error::other("SMB-Upload ist bereits abgeschlossen")),
            State::Failed => Err(io::Error::other(
                "SMB-Upload ist fehlgeschlagen; weitere Daten werden nicht angenommen",
            )),
        }
    }

    /// Closes a handle still open and removes the stage (best effort).
    fn remove(&mut self, handle: Option<FileId>) {
        self.state = State::Failed;
        let (tree, rel) = (self.tree.clone(), self.rel.clone());
        let mut conn = self.generation.connection();
        self.rt.block_on(async move {
            if let Some(id) = handle {
                wire::close_quietly(&conn, &tree, id).await;
            }
            let _ = tree.delete_file(&mut conn, &rel).await;
        });
    }

    fn wrong_length(&mut self, message: String) -> io::Error {
        let handle = self.file_id.take();
        self.remove(handle);
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    /// WRITE + CLOSE (or the CLOSE alone for an empty stage) beside the
    /// destination's stat: the stage's closed attributes, if reported.
    fn commit(&mut self, id: FileId) -> io::Result<()> {
        let conn = self.generation.connection();
        let tree = self.tree.clone();
        let probe = self
            .report
            .as_ref()
            .map(|report| report.destination_rel.clone());
        let data = std::mem::take(&mut self.data);
        let (compound, destination) = self.rt.block_on(async {
            let write = write_request(id, 0, &data);
            let close = CloseRequest {
                flags: SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB,
                file_id: if data.is_empty() {
                    id
                } else {
                    FileId::SENTINEL
                },
            };
            let tree_id = Some(tree.tree_id);
            let mut ops = Vec::with_capacity(2);
            if !data.is_empty() {
                ops.push(CompoundOp {
                    command: Command::Write,
                    body: &write,
                    tree_id,
                    credit_charge: charge(data.len()),
                });
            }
            ops.push(CompoundOp::new(Command::Close, &close, tree_id));
            let sent = async {
                let (link, chain) = (&conn, &ops[..]);
                let sent = funded(link, move || link.execute_compound(chain)).await?;
                let mut frames = wire::frames(sent, ops.len())?;
                if !data.is_empty() {
                    let count = written(&frames[0])?;
                    if count != data.len() {
                        return Ok::<Result<Frame, usize>, smb2::Error>(Err(count));
                    }
                }
                let answer = frames
                    .pop()
                    .ok_or_else(|| smb2::Error::invalid_data("no CLOSE answer"))?;
                Ok::<Result<Frame, usize>, smb2::Error>(Ok(answer))
            };
            let destination = async {
                match &probe {
                    Some(rel) => Some(wire::stat(&conn, &tree, rel).await),
                    None => None,
                }
            };
            futures_util::future::join(sent, destination).await
        });
        let close = match compound {
            Ok(Ok(close)) => close,
            Ok(Err(count)) => {
                self.remove(Some(id));
                return Err(io::Error::other(format!(
                    "SMB-Server hat nur {count} von {} Bytes bestätigt",
                    self.expected
                )));
            }
            Err(error) => {
                self.generation.note(&error);
                let error = errors::map(error, "Schreiben", &self.path);
                self.remove(Some(id));
                return Err(error);
            }
        };
        if close.header.status != NtStatus::SUCCESS {
            let error = wire::protocol(close.header.status, Command::Close);
            let error = errors::map(error, "Schließen", &self.path);
            self.remove(Some(id));
            return Err(error);
        }
        let attributes = CloseResponse::unpack(&mut ReadCursor::new(&close.body))
            .ok()
            .as_ref()
            .and_then(closed_attributes);
        if let Some(report) = self.report.take() {
            // Without the CLOSE's attributes the stage is asked, as before.
            let stage = match attributes {
                Some(attributes) => Ok(attributes),
                None => self.rt.block_on(wire::stat(&conn, &self.tree, &self.rel)),
            };
            report.record(&stage, destination.as_ref(), self.expected);
        }
        Ok(())
    }
}

impl Write for CompoundStage {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.check_open()?;
        if (self.data.len() + data.len()) as u64 > self.expected {
            return Err(
                self.wrong_length("Quelle ist während der Übertragung gewachsen".to_string())
            );
        }
        self.data.extend_from_slice(data);
        Ok(data.len())
    }

    /// The commit: fails unless exactly the announced length came and the
    /// server confirmed all of it.
    fn flush(&mut self) -> io::Result<()> {
        if self.state == State::Committed {
            return Ok(());
        }
        self.check_open()?;
        let accepted = self.data.len() as u64;
        if accepted != self.expected {
            let message = format!(
                "Quelle hat sich während der Übertragung geändert: {accepted} statt {} Bytes",
                self.expected
            );
            return Err(self.wrong_length(message));
        }
        let Some(id) = self.file_id.take() else {
            return Err(io::Error::other("SMB-Datei ist bereits geschlossen"));
        };
        self.commit(id)?;
        self.state = State::Committed;
        Ok(())
    }
}

impl Drop for CompoundStage {
    fn drop(&mut self) {
        // Dropped before its commit: nothing of it may stay behind.
        if let Some(id) = self.file_id.take() {
            self.remove(Some(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{closed_attributes, fits};
    use smb2::msg::close::{CloseResponse, SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB};
    use smb2::pack::FileTime;

    fn response(flags: u16) -> CloseResponse {
        CloseResponse {
            flags,
            creation_time: FileTime(1),
            last_access_time: FileTime(2),
            last_write_time: FileTime(133_000_000_000_000_000),
            change_time: FileTime(4),
            allocation_size: 4096,
            end_of_file: 5,
            file_attributes: 0x20,
        }
    }

    #[test]
    fn speed_task_smb_compound_stage_fits_one_write() {
        assert!(fits(1024 * 1024, Some(1024 * 1024)));
        assert!(!fits(1024 * 1024 + 1, Some(1024 * 1024)));
        assert!(fits(65_536, None), "every dialect allows 64 KiB");
        assert!(!fits(65_537, Some(1024)), "64 KiB at most here");
        assert!(fits(0, Some(1)));
    }

    #[test]
    fn speed_task_smb_close_attributes_only_when_reported() {
        let reported = closed_attributes(&response(SMB2_CLOSE_FLAG_POSTQUERY_ATTRIB));
        let reported = reported.unwrap();
        assert_eq!((reported.size, reported.attributes), (5, 0x20));
        assert_eq!(reported.last_write, 133_000_000_000_000_000);
        assert!(closed_attributes(&response(0)).is_none());
    }
}
