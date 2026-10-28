//! The real wire of the SMB packet paths (batch.rs): compounds built from the
//! pieces of `wire.rs`. Every CREATE carries FILE_OPEN_REPARSE_POINT and
//! FILE_NON_DIRECTORY_FILE, so a link is never followed; a new stage is
//! created with `FileCreate` and no sharing, so the server refuses a taken
//! name atomically. WRITEs are not flushed: a copy stage is not forced to
//! stable storage before it is published (`engine/publish.rs`).
use super::batch::{self, FileState, ReadResult, SmbWire, WireError, WireFuture};
use super::errors;
use super::replace;
use super::session::{Generation, SmbSession};
use super::url::{entry_path, rename_paths};
use super::wire::{self, DeleteKind, FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT};
use crate::vfs::{BatchGet, BatchPut, BatchPutOutcome, BatchSink, VfsResult};
use smb2::client::Connection;
use smb2::msg::create::{CreateDisposition, CreateRequest, ImpersonationLevel, ShareAccess};
use smb2::msg::read::{ReadRequest, ReadResponse, SMB2_CHANNEL_NONE};
use smb2::msg::write::{WriteRequest, WriteResponse};
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, CreditCharge, FileId, OplockLevel};
use smb2::{CompoundOp, Frame, Tree};
use std::io::{self, Read};
use std::sync::Arc;

/// Every dialect allows 64 KiB requests (MS-SMB2 3.3.5.4).
const MIN_IO: usize = 65_536;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// DataOffset of a WRITE: SMB2 header (64) + fixed WRITE body (48).
const WRITE_DATA_OFFSET: u16 = 0x70;
/// Read access to the entry itself; a link is opened as the link.
const READ_ACCESS: u32 = FileAccessMask::FILE_READ_DATA
    | FileAccessMask::FILE_READ_ATTRIBUTES
    | FileAccessMask::SYNCHRONIZE;
const WRITE_ACCESS: u32 = FileAccessMask::FILE_WRITE_DATA
    | FileAccessMask::FILE_WRITE_ATTRIBUTES
    | FileAccessMask::SYNCHRONIZE;
/// The options of every CREATE of a packet (plan U46-3).
pub(super) const PACKET_OPTIONS: u32 = FILE_OPEN_REPARSE_POINT | FILE_NON_DIRECTORY_FILE;

/// The CREATE of a packet member; `name` is the wire path of the entry. A
/// new stage is created (`FileCreate`, no sharing), a source is opened.
/// Both carry FILE_OPEN_REPARSE_POINT: a link is never followed.
pub(super) fn packet_create(name: String, new_stage: bool) -> CreateRequest {
    let sharing = if new_stage {
        ShareAccess(0)
    } else {
        ShareAccess(
            ShareAccess::FILE_SHARE_READ
                | ShareAccess::FILE_SHARE_WRITE
                | ShareAccess::FILE_SHARE_DELETE,
        )
    };
    CreateRequest {
        requested_oplock_level: OplockLevel::None,
        impersonation_level: ImpersonationLevel::Impersonation,
        desired_access: FileAccessMask::new(if new_stage { WRITE_ACCESS } else { READ_ACCESS }),
        file_attributes: if new_stage { FILE_ATTRIBUTE_NORMAL } else { 0 },
        share_access: sharing,
        create_disposition: if new_stage {
            CreateDisposition::FileCreate
        } else {
            CreateDisposition::FileOpen
        },
        create_options: PACKET_OPTIONS,
        name,
        create_contexts: Vec::new(),
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

fn write_request(file_id: FileId, offset: u64, data: &[u8]) -> WriteRequest {
    WriteRequest {
        data_offset: WRITE_DATA_OFFSET,
        offset,
        file_id,
        channel: 0,
        remaining_bytes: 0,
        write_channel_info_offset: 0,
        write_channel_info_length: 0,
        flags: 0,
        data: data.to_vec(),
    }
}

fn read_request(file_id: FileId, offset: u64, len: usize) -> ReadRequest {
    ReadRequest {
        padding: 0x50,
        flags: 0,
        length: len as u32,
        offset,
        file_id,
        minimum_count: 0,
        channel: SMB2_CHANNEL_NONE,
        remaining_bytes: 0,
        read_channel_info: Vec::new(),
    }
}

/// Bytes the server confirmed for a WRITE.
fn written(frame: &Frame) -> smb2::Result<usize> {
    if frame.header.status != NtStatus::SUCCESS {
        return Err(wire::protocol(frame.header.status, Command::Write));
    }
    Ok(WriteResponse::unpack(&mut ReadCursor::new(&frame.body))?.count as usize)
}

/// The bytes of a READ; reading at or past the end is an empty answer.
fn read_data(frame: &Frame) -> smb2::Result<Vec<u8>> {
    let status = frame.header.status;
    if status == NtStatus::END_OF_FILE {
        return Ok(Vec::new());
    }
    if status != NtStatus::SUCCESS {
        return Err(wire::protocol(status, Command::Read));
    }
    Ok(ReadResponse::unpack(&mut ReadCursor::new(&frame.body))?.data)
}

pub(super) struct TreeWire {
    conn: Connection,
    tree: Arc<Tree>,
    generation: Arc<Generation>,
    max_write: usize,
    max_read: usize,
}

impl TreeWire {
    fn new(generation: &Arc<Generation>, tree: Arc<Tree>) -> Self {
        let conn = generation.connection();
        let params = conn.params();
        let limit = |value: Option<u32>| value.map_or(MIN_IO, |bytes| bytes as usize).max(MIN_IO);
        Self {
            max_write: limit(params.as_ref().map(|params| params.max_write_size)),
            max_read: limit(params.as_ref().map(|params| params.max_read_size)),
            conn,
            tree,
            generation: generation.clone(),
        }
    }

    fn fail(&self, error: smb2::Error, action: &str, path: &str, created: bool) -> WireError {
        self.generation.note(&error);
        WireError {
            answered: error.status().is_some(),
            created,
            error: errors::map(error, action, path),
        }
    }

    /// Nothing was sent: the request is refused before the wire.
    fn refused(error: io::Error) -> WireError {
        WireError {
            error,
            answered: true,
            created: false,
        }
    }
}

impl SmbWire for TreeWire {
    fn create_write_close_new<'a>(
        &'a self,
        stage: &'a str,
        data: &'a [u8],
        first_block: usize,
    ) -> WireFuture<'a, FileState> {
        Box::pin(async move {
            let target = entry_path(stage).map_err(Self::refused)?;
            let name = wire::wire_path(&self.tree, &target.rel);
            let create = packet_create(name, true);
            let first = &data[..first_block.min(data.len())];
            let whole = first.len() == data.len();
            let write = write_request(FileId::SENTINEL, 0, first);
            let close = wire::related_close();
            let tree_id = Some(self.tree.tree_id);
            let mut ops = vec![CompoundOp::new(Command::Create, &create, tree_id)];
            if !first.is_empty() {
                ops.push(CompoundOp {
                    command: Command::Write,
                    body: &write,
                    tree_id,
                    credit_charge: charge(first.len()),
                });
            }
            if whole {
                ops.push(CompoundOp::new(Command::Close, &close, tree_id));
            }
            let fail = |error: smb2::Error| self.fail(error, "Anlegen", stage, false);
            let frames = wire::frames(
                self.conn.execute_compound(&ops).await.map_err(fail)?,
                ops.len(),
            )
            .map_err(fail)?;
            let id = wire::created(&frames[0]).map_err(fail)?.file_id;
            let mut next = 1;
            if !first.is_empty() {
                let count = written(&frames[1]);
                let short = count.as_ref().is_ok_and(|count| *count != first.len());
                if count.is_err() || short {
                    wire::close_quietly(&self.conn, &self.tree, id).await;
                }
                if short {
                    return Err(WireError {
                        error: io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("SMB-Server hat nicht alle Bytes von {stage} bestätigt"),
                        ),
                        answered: true,
                        created: true,
                    });
                }
                count.map_err(|error| self.fail(error, "Schreiben", stage, true))?;
                next = 2;
            }
            if whole && frames[next].header.status != NtStatus::SUCCESS {
                wire::close_quietly(&self.conn, &self.tree, id).await;
                let error = wire::protocol(frames[next].header.status, Command::Close);
                return Err(self.fail(error, "Schließen", stage, true));
            }
            Ok(FileState { id, closed: whole })
        })
    }

    fn write_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        data: &'a [u8],
    ) -> WireFuture<'a, ()> {
        Box::pin(async move {
            let request = write_request(file.id, offset, data);
            let sent = self
                .conn
                .execute_with_credits(
                    Command::Write,
                    &request,
                    Some(self.tree.tree_id),
                    charge(data.len()),
                )
                .await;
            let count = sent.and_then(|frame| written(&frame));
            match count {
                Ok(count) if count == data.len() => Ok(()),
                Ok(_) => Err(WireError {
                    error: io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SMB-Server hat nicht alle Bytes einer Upload-Stufe bestätigt",
                    ),
                    answered: true,
                    created: true,
                }),
                Err(error) => Err(self.fail(error, "Schreiben", "Upload-Stufe", true)),
            }
        })
    }

    fn close<'a>(&'a self, file: &'a FileState) -> WireFuture<'a, ()> {
        Box::pin(async move {
            let mut conn = self.conn.clone();
            let closed = self.tree.close_handle(&mut conn, file.id).await;
            closed.map_err(|error| self.fail(error, "Schließen", "SMB-Datei", true))
        })
    }

    fn rename_no_replace<'a>(&'a self, from: &'a str, to: &'a str) -> WireFuture<'a, ()> {
        Box::pin(async move {
            let (from, to) = rename_paths(from, to).map_err(Self::refused)?;
            let renamed = replace::rename(&self.conn, &self.tree, &from.rel, &to.rel, false);
            let renamed = renamed.await;
            renamed.map_err(|error| self.fail(error, "Umbenennen", &to.rel, false))
        })
    }

    fn remove<'a>(&'a self, path: &'a str) -> WireFuture<'a, ()> {
        Box::pin(async move {
            let target = entry_path(path).map_err(Self::refused)?;
            let removed = wire::delete(&self.conn, &self.tree, &target.rel, DeleteKind::File);
            removed
                .await
                .map_err(|error| self.fail(error, "Löschen", path, false))
        })
    }

    fn open_read_whole<'a>(&'a self, path: &'a str, expected: u64) -> WireFuture<'a, ReadResult> {
        Box::pin(async move {
            let target = entry_path(path).map_err(Self::refused)?;
            let create = packet_create(wire::wire_path(&self.tree, &target.rel), false);
            let whole = expected <= self.max_read as u64;
            let len = expected.min(self.max_read as u64) as usize;
            let read = read_request(FileId::SENTINEL, 0, len);
            let close = wire::related_close();
            let tree_id = Some(self.tree.tree_id);
            let mut ops = vec![CompoundOp::new(Command::Create, &create, tree_id)];
            if len > 0 {
                ops.push(CompoundOp {
                    command: Command::Read,
                    body: &read,
                    tree_id,
                    credit_charge: charge(len),
                });
            }
            if whole {
                ops.push(CompoundOp::new(Command::Close, &close, tree_id));
            }
            let fail = |error: smb2::Error| self.fail(error, "Lesen", path, false);
            let frames = wire::frames(
                self.conn.execute_compound(&ops).await.map_err(fail)?,
                ops.len(),
            )
            .map_err(fail)?;
            let response = wire::created(&frames[0]).map_err(fail)?;
            let id = response.file_id;
            let reparse = wire::attributes_of(&response).is_reparse_point();
            let mut data = Vec::new();
            if len > 0 {
                match read_data(&frames[1]) {
                    Ok(bytes) => data = bytes,
                    Err(error) => {
                        wire::close_quietly(&self.conn, &self.tree, id).await;
                        // The stub of a data reparse point need not be readable.
                        if !reparse {
                            return Err(fail(error));
                        }
                    }
                }
            }
            // The data came back; a failed CLOSE only leaks the handle.
            if whole && frames[frames.len() - 1].header.status != NtStatus::SUCCESS {
                wire::close_quietly(&self.conn, &self.tree, id).await;
            }
            Ok(ReadResult {
                size: response.end_of_file,
                data,
                file: (!whole).then_some(FileState { id, closed: false }),
                reparse,
            })
        })
    }

    fn read_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        len: usize,
    ) -> WireFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let request = read_request(file.id, offset, len);
            let sent = self
                .conn
                .execute_with_credits(
                    Command::Read,
                    &request,
                    Some(self.tree.tree_id),
                    charge(len),
                )
                .await;
            let data = sent.and_then(|frame| read_data(&frame));
            data.map_err(|error| self.fail(error, "Lesen", "SMB-Datei", false))
        })
    }

    fn read_followed<'a>(&'a self, path: &'a str) -> WireFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let target = entry_path(path).map_err(Self::refused)?;
            let fail = |error: smb2::Error| self.fail(error, "Lesen", path, false);
            let attributes = wire::stat(&self.conn, &self.tree, &target.rel)
                .await
                .map_err(fail)?;
            if attributes.is_link() {
                return Ok(None);
            }
            let mut conn = self.conn.clone();
            let bytes = self.tree.read_file_pipelined(&mut conn, &target.rel);
            bytes.await.map(Some).map_err(fail)
        })
    }

    fn max_write(&self) -> usize {
        self.max_write
    }

    fn max_read(&self) -> usize {
        self.max_read
    }
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message.to_string())
}

/// The share every path lives in; mixed shares or unusable paths are
/// refused before anything is sent, so the engine moves the files singly.
fn common_share<'a>(paths: impl Iterator<Item = &'a str>) -> io::Result<String> {
    let mut common: Option<String> = None;
    for path in paths {
        let target = entry_path(path).map_err(|error| unsupported(&error.to_string()))?;
        match &common {
            None => common = Some(target.share),
            Some(share) if share.eq_ignore_ascii_case(&target.share) => {}
            Some(_) => return Err(unsupported("Paket über mehrere SMB-Freigaben")),
        }
    }
    common.ok_or_else(|| unsupported("Leeres Paket"))
}

fn wire_for(session: &SmbSession, share: &str) -> io::Result<TreeWire> {
    let (generation, tree) = session.target(share)?;
    Ok(TreeWire::new(&generation, tree))
}

pub(super) fn put(
    session: &SmbSession,
    entries: &[BatchPut],
    data: &mut dyn Read,
) -> VfsResult<Vec<BatchPutOutcome>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if entries.len() < 2 {
        return Err(unsupported("Paket-Upload braucht mindestens zwei Dateien"));
    }
    let share = common_share(entries.iter().map(|entry| entry.path.as_str()))?;
    let wire = wire_for(session, &share)?;
    Ok(session.block_on(batch::put_all(&wire, entries, data)))
}

pub(super) fn get(
    session: &SmbSession,
    items: &[BatchGet],
    sink: &mut dyn BatchSink,
) -> VfsResult<()> {
    if items.len() < 2 {
        return Err(unsupported(
            "Paket-Download braucht mindestens zwei Dateien",
        ));
    }
    let share = common_share(items.iter().map(|item| item.path.as_str()))?;
    // The wire runs on a fetcher thread, the sink here outside the runtime
    // (G3-01: it may call this backend; the wire goes on while it publishes).
    crate::sftp::hand_off_packet(
        "smb-packet-get",
        |forward| {
            let wire = wire_for(session, &share)?;
            session.block_on(batch::get_all_with(&wire, items, forward))
        },
        |index, done| match items.get(index) {
            Some(item) => batch::deliver(sink, index, item, done),
            None => Err(batch::unknown_member()),
        },
    )
}
