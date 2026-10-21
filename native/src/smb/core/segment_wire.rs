//! The wire under the SMB segmented stage (segmented_stage.rs), through
//! smb2's public message API: a CREATE with `FileOpen` that can only open,
//! the identity queries compounded onto it, and a writer that pipelines the
//! WRITEs of one range over its handle.
//!
//! Every CREATE here carries `FILE_NON_DIRECTORY_FILE` and
//! `FILE_OPEN_REPARSE_POINT` (a link is opened as the link and shows its
//! reparse attribute; it is never followed). The access of a range handle is
//! `FILE_WRITE_DATA | FILE_READ_ATTRIBUTES` and nothing else: no `DELETE`, no
//! `WRITE_DAC`, no `WRITE_OWNER`. Sharing is read and write, never delete.
use super::backend::SmbBackend;
use super::errors;
use super::segmented_stage::{
    identity_from, standard_from, Created, Facts, RangeHandle, RangeOpen, StageWire, NO_RANGES_FOR,
};
use super::session::Generation;
use super::url::entry_path;
use super::wire::{self, FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT};
use crate::transfer::read_pipeline::Pipeline;
use smb2::client::Connection;
use smb2::msg::create::{
    CreateDisposition, CreateRequest, CreateResponse, ImpersonationLevel, ShareAccess,
};
use smb2::msg::query_info::{QueryInfoRequest, QueryInfoResponse};
use smb2::msg::set_info::InfoType;
use smb2::msg::write::WriteRequest;
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, CreditCharge, FileId, OplockLevel};
use smb2::{CompoundOp, Frame, Tree};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// MS-FSCC information classes.
const FILE_STANDARD_INFORMATION: u8 = 5;
const FILE_INTERNAL_INFORMATION: u8 = 6;
const FILE_ID_INFORMATION: u8 = 59;
const FILE_END_OF_FILE_INFORMATION: u8 = 20;
const STANDARD_LEN: u32 = 24;
const INTERNAL_LEN: u32 = 8;
const FILE_ID_LEN: u32 = 24;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// Every dialect allows 64 KiB requests (MS-SMB2 3.3.5.4).
const MIN_IO: usize = 65_536;
/// Bytes gathered before a WRITE goes out, at most.
const WRITE_CHUNK: usize = 1 << 20;
/// DataOffset of a WRITE: SMB2 header (64) + fixed WRITE body (48).
const WRITE_DATA_OFFSET: u16 = 0x70;

/// What a range handle may do: write data, read the attributes it is asked.
const ACCESS: u32 = FileAccessMask::FILE_WRITE_DATA | FileAccessMask::FILE_READ_ATTRIBUTES;

fn request(tree: &Tree, rel: &str, disposition: CreateDisposition) -> CreateRequest {
    CreateRequest {
        requested_oplock_level: OplockLevel::None,
        impersonation_level: ImpersonationLevel::Impersonation,
        desired_access: FileAccessMask::new(ACCESS),
        file_attributes: if disposition == CreateDisposition::FileCreate {
            FILE_ATTRIBUTE_NORMAL
        } else {
            0
        },
        // Handles of the creator and of the ranges let each other write;
        // nobody may delete or rename the stage meanwhile.
        share_access: ShareAccess(ShareAccess::FILE_SHARE_READ | ShareAccess::FILE_SHARE_WRITE),
        create_disposition: disposition,
        create_options: FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        name: wire::wire_path(tree, rel),
        create_contexts: Vec::new(),
    }
}

/// QUERY_INFO(FileInformation) on the handle of the CREATE before it.
fn query(class: u8, length: u32) -> QueryInfoRequest {
    QueryInfoRequest {
        info_type: InfoType::File,
        file_info_class: class,
        output_buffer_length: length,
        additional_information: 0,
        flags: 0,
        file_id: FileId::SENTINEL,
        input_buffer: Vec::new(),
    }
}

/// The answer of a query, `None` when the server refused the class.
fn payload(frame: &Frame) -> Option<Vec<u8>> {
    if frame.header.status != NtStatus::SUCCESS {
        return None;
    }
    let response = QueryInfoResponse::unpack(&mut ReadCursor::new(&frame.body)).ok()?;
    Some(response.output_buffer)
}

fn facts_of(
    created: &CreateResponse,
    id: &Frame,
    index: &Frame,
    standard: Option<&Frame>,
) -> Facts {
    let identity = identity_from(
        payload(id).as_deref(),
        payload(index).as_deref(),
        created.creation_time.0,
    );
    let (end_of_file, delete_pending, directory) = standard
        .and_then(payload)
        .and_then(|bytes| standard_from(&bytes))
        .unwrap_or((created.end_of_file, false, false));
    Facts {
        identity,
        action: created.create_action,
        attributes: created.file_attributes,
        end_of_file,
        directory,
        delete_pending,
    }
}

/// One credit per 64 KiB of payload (MS-SMB2 3.2.4.1.5).
fn charge(len: usize) -> CreditCharge {
    CreditCharge(
        (len as u64)
            .div_ceil(MIN_IO as u64)
            .clamp(1, u64::from(u16::MAX)) as u16,
    )
}

/// The creator's handle; closed when dropped.
struct Held {
    rt: Arc<Runtime>,
    conn: Connection,
    tree: Arc<Tree>,
    id: FileId,
}

impl Drop for Held {
    fn drop(&mut self) {
        close(&self.rt, &self.conn, &self.tree, self.id);
    }
}

/// Best effort: a handle that cannot be closed is released with the session.
fn close(rt: &Runtime, conn: &Connection, tree: &Tree, id: FileId) {
    let mut conn = conn.clone();
    let _ = rt.block_on(tree.close_handle(&mut conn, id));
}

/// An opened, checked range handle that nothing was written to yet.
struct Handle {
    rt: Arc<Runtime>,
    conn: Connection,
    tree: Arc<Tree>,
    generation: Arc<Generation>,
    id: FileId,
    open: bool,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if self.open {
            close(&self.rt, &self.conn, &self.tree, self.id);
        }
    }
}

impl RangeHandle for Handle {
    fn into_writer(mut self: Box<Self>, offset: u64, len: u64) -> Box<dyn Write + Send> {
        self.open = false;
        let max_write = self
            .conn
            .params()
            .map_or(MIN_IO, |params| params.max_write_size as usize)
            .max(MIN_IO);
        let chunk = max_write.min(WRITE_CHUNK);
        let cap = self.conn.credit_capacity_for(chunk as u64);
        Box::new(RangeWriter {
            rt: self.rt.clone(),
            conn: self.conn.clone(),
            tree: self.tree.clone(),
            generation: self.generation.clone(),
            id: self.id,
            next: offset,
            expected: len,
            accepted: 0,
            chunk,
            buffer: Vec::with_capacity(chunk),
            inflight: VecDeque::new(),
            pipeline: Pipeline::new(chunk as u64, cap),
            done: false,
        })
    }
}

impl StageWire for SmbBackend {
    fn create(&self, stage: &str, size: u64) -> io::Result<Option<Created>> {
        let target = entry_path(stage)?;
        let (generation, tree) = self.session.target(&target.share)?;
        let conn = generation.connection();
        let create = request(&tree, &target.rel, CreateDisposition::FileCreate);
        let length = wire::set_file_info(
            FILE_END_OF_FILE_INFORMATION,
            FileId::SENTINEL,
            size.to_le_bytes().to_vec(),
        );
        let (id_query, index_query) = (
            query(FILE_ID_INFORMATION, FILE_ID_LEN),
            query(FILE_INTERNAL_INFORMATION, INTERNAL_LEN),
        );
        let tree_id = Some(tree.tree_id);
        let ops = [
            CompoundOp::new(Command::Create, &create, tree_id),
            CompoundOp::new(Command::SetInfo, &length, tree_id),
            CompoundOp::new(Command::QueryInfo, &id_query, tree_id),
            CompoundOp::new(Command::QueryInfo, &index_query, tree_id),
        ];
        let sent = self.session.block_on(async {
            let frames = conn.execute_compound(&ops).await?;
            wire::frames(frames, ops.len())
        });
        let fail = |error: smb2::Error| {
            generation.note(&error);
            errors::map(error, "Datei anlegen", stage)
        };
        let frames = sent.map_err(fail)?;
        let response = wire::created(&frames[0]).map_err(fail)?;
        let hold = Held {
            rt: self.session.runtime(),
            conn: conn.clone(),
            tree: tree.clone(),
            id: response.file_id,
        };
        if frames[1].header.status != NtStatus::SUCCESS {
            // The length cannot be set (full volume, no support): the stage
            // goes, the file streams.
            drop(hold);
            let _ = StageWire::discard(self, stage);
            return Ok(None);
        }
        Ok(Some(Created {
            identity: identity_from(
                payload(&frames[2]).as_deref(),
                payload(&frames[3]).as_deref(),
                response.creation_time.0,
            ),
            hold: Box::new(hold),
        }))
    }

    fn open_range(&self, stage: &str) -> io::Result<RangeOpen> {
        let target = entry_path(stage)?;
        let (generation, tree) = self.session.target(&target.share)?;
        let conn = generation.connection();
        // `FileOpen`: a missing name is an error, never a new file.
        let create = request(&tree, &target.rel, CreateDisposition::FileOpen);
        let queries = [
            query(FILE_ID_INFORMATION, FILE_ID_LEN),
            query(FILE_INTERNAL_INFORMATION, INTERNAL_LEN),
            query(FILE_STANDARD_INFORMATION, STANDARD_LEN),
        ];
        let tree_id = Some(tree.tree_id);
        let ops = [
            CompoundOp::new(Command::Create, &create, tree_id),
            CompoundOp::new(Command::QueryInfo, &queries[0], tree_id),
            CompoundOp::new(Command::QueryInfo, &queries[1], tree_id),
            CompoundOp::new(Command::QueryInfo, &queries[2], tree_id),
        ];
        let sent = self.session.block_on(async {
            let frames = conn.execute_compound(&ops).await?;
            wire::frames(frames, ops.len())
        });
        let fail = |error: smb2::Error| {
            generation.note(&error);
            if error.status() == Some(NtStatus::SHARING_VIOLATION) {
                return io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("SMB-Server erlaubt keine gemeinsamen Handles auf {stage}"),
                );
            }
            errors::map(error, "Bereich öffnen", stage)
        };
        let frames = sent.map_err(fail)?;
        let response = wire::created(&frames[0]).map_err(fail)?;
        let handle = Handle {
            rt: self.session.runtime(),
            conn,
            tree,
            generation,
            id: response.file_id,
            open: true,
        };
        let facts = facts_of(&response, &frames[1], &frames[2], Some(&frames[3]));
        Ok(RangeOpen {
            facts,
            handle: Box::new(handle),
        })
    }

    fn discard(&self, stage: &str) -> io::Result<()> {
        crate::vfs::Backend::discard_copy_stage(self, stage)
    }

    fn refused(&self) -> bool {
        self.no_ranges_until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some_and(|until| until > Instant::now())
    }

    fn refuse(&self) {
        *self
            .no_ranges_until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Instant::now() + NO_RANGES_FOR);
    }
}

struct Pending {
    len: usize,
    sent: Instant,
    task: JoinHandle<(smb2::Result<usize>, Instant)>,
}

/// Writes exactly `expected` bytes from the handle's `next` offset on, with
/// several WRITEs in flight; `flush` waits for every answer and closes the
/// handle (no FLUSH: a copy stage is not forced to stable storage).
struct RangeWriter {
    rt: Arc<Runtime>,
    conn: Connection,
    tree: Arc<Tree>,
    generation: Arc<Generation>,
    id: FileId,
    next: u64,
    expected: u64,
    accepted: u64,
    chunk: usize,
    buffer: Vec<u8>,
    inflight: VecDeque<Pending>,
    pipeline: Pipeline,
    /// The handle is closed (by `flush` or `Drop`).
    done: bool,
}

impl RangeWriter {
    fn submit(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let data = std::mem::replace(&mut self.buffer, Vec::with_capacity(self.chunk));
        let len = data.len();
        let write = WriteRequest {
            data_offset: WRITE_DATA_OFFSET,
            offset: self.next,
            file_id: self.id,
            channel: 0,
            remaining_bytes: 0,
            write_channel_info_offset: 0,
            write_channel_info_length: 0,
            flags: 0,
            data,
        };
        self.next += len as u64;
        let conn = self.conn.clone();
        let tree_id = self.tree.tree_id;
        let task = self.rt.spawn(async move {
            let sent = conn
                .execute_with_credits(Command::Write, &write, Some(tree_id), charge(len))
                .await;
            let count = sent.and_then(|frame| {
                if frame.header.status != NtStatus::SUCCESS {
                    return Err(wire::protocol(frame.header.status, Command::Write));
                }
                let response =
                    smb2::msg::write::WriteResponse::unpack(&mut ReadCursor::new(&frame.body))?;
                Ok(response.count as usize)
            });
            (count, Instant::now())
        });
        self.inflight.push_back(Pending {
            len,
            sent: Instant::now(),
            task,
        });
        while self.inflight.len() > self.pipeline.depth() {
            self.await_oldest()?;
        }
        Ok(())
    }

    fn await_oldest(&mut self) -> io::Result<()> {
        let Some(write) = self.inflight.pop_front() else {
            return Ok(());
        };
        let (count, arrived) = match self.rt.block_on(write.task) {
            Ok(answer) => answer,
            Err(error) => {
                return Err(io::Error::other(format!(
                    "SMB-Schreibauftrag wurde abgebrochen: {error}"
                )))
            }
        };
        match count {
            Ok(count) if count == write.len => {
                self.pipeline.answered(
                    arrived.saturating_duration_since(write.sent),
                    write.len == self.chunk,
                );
                Ok(())
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SMB-Server hat nicht alle Bytes eines Bereichs bestätigt",
            )),
            Err(error) => {
                self.generation.note(&error);
                Err(errors::map(error, "Schreiben", "Upload-Stufe"))
            }
        }
    }

    fn close(&mut self) {
        if !self.done {
            self.done = true;
            close(&self.rt, &self.conn, &self.tree, self.id);
        }
    }
}

impl Write for RangeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.done {
            return Err(io::Error::other("SMB-Bereich ist bereits abgeschlossen"));
        }
        let accepted = self.accepted.saturating_add(data.len() as u64);
        if accepted > self.expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle ist während der Übertragung gewachsen",
            ));
        }
        self.accepted = accepted;
        self.buffer.extend_from_slice(data);
        if self.buffer.len() >= self.chunk {
            self.submit()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        if self.accepted != self.expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Quelle hat sich während der Übertragung geändert: {} statt {} Bytes",
                    self.accepted, self.expected
                ),
            ));
        }
        self.submit()?;
        while !self.inflight.is_empty() {
            self.await_oldest()?;
        }
        let mut conn = self.conn.clone();
        let closed = self.rt.block_on(self.tree.close_handle(&mut conn, self.id));
        self.done = true;
        closed.map_err(|error| {
            self.generation.note(&error);
            errors::map(error, "Schließen", "Upload-Stufe")
        })
    }
}

impl Drop for RangeWriter {
    fn drop(&mut self) {
        // Given up before the commit: nothing in flight matters, the stage
        // itself is removed by its `abort`.
        for write in self.inflight.drain(..) {
            write.task.abort();
        }
        self.close();
    }
}
