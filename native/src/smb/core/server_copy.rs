//! Server-side copy inside one SMB share: `FSCTL_SRV_REQUEST_RESUME_KEY` on
//! the source and `FSCTL_SRV_COPYCHUNK` into the new stage (MS-SMB2
//! 2.2.31.1, smb2's `request_resume_key` and `server_side_copy_range`, which
//! batches 16 × 1 MiB per request and adopts the limits a server
//! advertises), so no byte crosses the network. The stage is created
//! exclusively with read and write access (COPYCHUNK needs both on its
//! destination, MS-SMB2 3.3.5.15.6) and closed without FLUSH (an engine copy
//! keeps its source, plan W1). A server without COPYCHUNK (smb2
//! `ErrorKind::Unsupported`: NOT_SUPPORTED, INVALID_DEVICE_REQUEST,
//! NOT_IMPLEMENTED) keeps nothing of the attempt and the engine streams, as
//! for any other status the copy requests are answered with: same-share
//! copies streamed before, and a quirky server must not turn them into
//! failures. Only a full target, an overloaded server and a lost connection
//! stay errors.
//!
//! COPYCHUNK cannot read past the source's end, so the stream path's
//! one-byte-more check becomes two length checks: at open the source must
//! have the listed length, after the copy it must still have it.
use super::errors::overloaded;
use super::session::Generation;
use super::wire::{self, created, open_request, DeleteKind, FILE_NON_DIRECTORY_FILE};
use smb2::client::Connection;
use smb2::msg::create::{CreateDisposition, CreateRequest, CreateResponse};
use smb2::msg::query_info::{InfoType, QueryInfoRequest, QueryInfoResponse};
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId};
use smb2::{Error, ErrorKind, Tree};

/// FileStandardInformation (MS-FSCC 2.4.41): AllocationSize, EndOfFile,
/// NumberOfLinks, DeletePending, Directory, Reserved.
const FILE_STANDARD_INFORMATION: u8 = 5;
const STANDARD_INFORMATION_LEN: u32 = 24;
const END_OF_FILE: std::ops::Range<usize> = 8..16;
/// MS-FSCC 2.6, as smb2 creates its write handles.
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

/// How a server-side copy ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ServerCopy {
    /// The stage holds this many bytes, the listed length.
    Copied(u64),
    /// The source's length is no longer the listed one (this is its new
    /// length); the stage, if any, is the engine's to discard.
    Changed(u64),
    /// The server has no COPYCHUNK; the stage was removed again.
    Refused,
    /// Nothing of the attempt exists (the source did not open, or the server
    /// declined the copy and the stage was removed): the stream path takes
    /// over and reports a real file problem as the source's or target's.
    Stream,
}

impl ServerCopy {
    /// `server_copy_to_stage`'s answer: the length for the engine to check
    /// (a changed one fails the file as a changed source), `None` to stream.
    pub(super) fn engine_answer(&self) -> Option<u64> {
        match self {
            ServerCopy::Copied(length) | ServerCopy::Changed(length) => Some(*length),
            ServerCopy::Refused | ServerCopy::Stream => None,
        }
    }
}

/// A server without server-side copy.
pub(super) fn refused(error: &Error) -> bool {
    error.kind() == ErrorKind::Unsupported
}

/// A copy request the server answered without copying. The source is open
/// for reading and the stage exists, so its answer is about the server's
/// copy: streaming reports any real file problem itself. A full target
/// would only fail again after every byte crossed the network, an
/// overloaded server is congestion the transfer backs off from (plan K13),
/// and a lost connection is no answer: they stay errors.
pub(super) fn declined(error: &Error) -> Option<ServerCopy> {
    if refused(error) {
        return Some(ServerCopy::Refused);
    }
    match error {
        Error::Protocol { .. } | Error::InvalidData { .. }
            if error.kind() != ErrorKind::DiskFull && !overloaded(error) =>
        {
            Some(ServerCopy::Stream)
        }
        _ => None,
    }
}

/// Read access, as smb2's reader opens a file.
pub(super) const SOURCE_ACCESS: u32 = FileAccessMask::FILE_READ_DATA
    | FileAccessMask::FILE_READ_ATTRIBUTES
    | FileAccessMask::SYNCHRONIZE;
/// Read and write: COPYCHUNK needs both on its destination.
pub(super) const STAGE_ACCESS: u32 = FileAccessMask::FILE_READ_DATA
    | FileAccessMask::FILE_WRITE_DATA
    | FileAccessMask::FILE_READ_ATTRIBUTES
    | FileAccessMask::FILE_WRITE_ATTRIBUTES
    | FileAccessMask::SYNCHRONIZE;

/// The source: an existing file, never a folder.
fn source_request(tree: &Tree, rel: &str) -> CreateRequest {
    open_request(tree, rel, SOURCE_ACCESS, FILE_NON_DIRECTORY_FILE)
}

/// The stage: a new file, readable and writable for COPYCHUNK.
fn stage_request(tree: &Tree, rel: &str) -> CreateRequest {
    as_new_file(open_request(
        tree,
        rel,
        STAGE_ACCESS,
        FILE_NON_DIRECTORY_FILE,
    ))
}

/// A CREATE of a new file only (`FileCreate` refuses a taken name with
/// OBJECT_NAME_COLLISION), with the normal attributes smb2's writers give.
pub(super) fn as_new_file(mut request: CreateRequest) -> CreateRequest {
    request.create_disposition = CreateDisposition::FileCreate;
    request.file_attributes = FILE_ATTRIBUTE_NORMAL;
    request
}

/// QUERY_INFO(FileStandardInformation) of an open file.
pub(super) fn length_request(file_id: FileId) -> QueryInfoRequest {
    QueryInfoRequest {
        info_type: InfoType::File,
        file_info_class: FILE_STANDARD_INFORMATION,
        output_buffer_length: STANDARD_INFORMATION_LEN,
        additional_information: 0,
        flags: 0,
        file_id,
        input_buffer: Vec::new(),
    }
}

/// EndOfFile of a FileStandardInformation buffer (little-endian).
pub(super) fn end_of_file(buffer: &[u8]) -> smb2::Result<u64> {
    buffer
        .get(END_OF_FILE)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| Error::invalid_data("FileStandardInformation ohne Dateilänge"))
}

/// The requests of one server-side copy: the backend's tree, or a script
/// in tests.
pub(super) trait CopyOps {
    type File: Copy;
    /// The source's handle and its length now.
    async fn open_source(&mut self, rel: &str) -> smb2::Result<(Self::File, u64)>;
    async fn create_stage(&mut self, rel: &str) -> smb2::Result<Self::File>;
    /// Copies `[0, length)` of `source` into `stage`; the bytes copied.
    async fn copy(
        &mut self,
        source: Self::File,
        stage: Self::File,
        length: u64,
    ) -> smb2::Result<u64>;
    async fn length(&mut self, file: Self::File) -> smb2::Result<u64>;
    async fn close(&mut self, file: Self::File) -> smb2::Result<()>;
    async fn delete(&mut self, rel: &str) -> smb2::Result<()>;
}

/// Copies `source` (listed with `size` bytes) into the new stage `stage`.
pub(super) async fn copy_to_stage<O: CopyOps>(
    ops: &mut O,
    source: &str,
    stage: &str,
    size: u64,
) -> smb2::Result<ServerCopy> {
    let Ok((source_file, length)) = ops.open_source(source).await else {
        return Ok(ServerCopy::Stream);
    };
    if length != size {
        let _ = ops.close(source_file).await;
        return Ok(ServerCopy::Changed(length));
    }
    let stage_file = match ops.create_stage(stage).await {
        Ok(file) => file,
        Err(error) => {
            let _ = ops.close(source_file).await;
            return Err(error);
        }
    };
    // Only the copy requests can be declined; the length query after them
    // is plain SMB2.
    let outcome = match ops.copy(source_file, stage_file, size).await {
        Ok(copied) => match ops.length(source_file).await {
            Ok(now) if now == size => Ok(ServerCopy::Copied(copied)),
            Ok(now) => Ok(ServerCopy::Changed(now)),
            Err(error) => Err(error),
        },
        Err(error) => declined(&error).ok_or(error),
    };
    let _ = ops.close(source_file).await;
    match outcome {
        Ok(answer @ (ServerCopy::Refused | ServerCopy::Stream)) => {
            let _ = ops.close(stage_file).await;
            ops.delete(stage).await?;
            Ok(answer)
        }
        Ok(copy) => {
            ops.close(stage_file).await?;
            Ok(copy)
        }
        Err(error) => {
            let _ = ops.close(stage_file).await;
            Err(error)
        }
    }
}

/// The requests on the backend's tree. Every failure is noted on the
/// generation, so a lost connection retires it.
pub(super) struct TreeOps<'a> {
    pub(super) conn: Connection,
    pub(super) tree: &'a Tree,
    pub(super) generation: &'a Generation,
}

impl TreeOps<'_> {
    fn noted(&self, error: Error) -> Error {
        self.generation.note(&error);
        error
    }

    async fn create(&self, request: &CreateRequest) -> smb2::Result<CreateResponse> {
        let frame = self
            .conn
            .execute(Command::Create, request, Some(self.tree.tree_id))
            .await
            .map_err(|error| self.noted(error))?;
        created(&frame).map_err(|error| self.noted(error))
    }
}

impl CopyOps for TreeOps<'_> {
    type File = FileId;

    async fn open_source(&mut self, rel: &str) -> smb2::Result<(FileId, u64)> {
        let opened = self.create(&source_request(self.tree, rel)).await?;
        Ok((opened.file_id, opened.end_of_file))
    }

    async fn create_stage(&mut self, rel: &str) -> smb2::Result<FileId> {
        let created = self.create(&stage_request(self.tree, rel)).await?;
        Ok(created.file_id)
    }

    async fn copy(&mut self, source: FileId, stage: FileId, length: u64) -> smb2::Result<u64> {
        if length == 0 {
            return Ok(0);
        }
        let key = match self.tree.request_resume_key(&mut self.conn, source).await {
            Ok(key) => key,
            Err(error) => return Err(self.noted(error)),
        };
        let copied = self
            .tree
            .server_side_copy_range(&mut self.conn, stage, &key, 0, 0, length)
            .await;
        copied.map_err(|error| self.noted(error))
    }

    async fn length(&mut self, file: FileId) -> smb2::Result<u64> {
        let request = length_request(file);
        let frame = self
            .conn
            .execute(Command::QueryInfo, &request, Some(self.tree.tree_id))
            .await
            .map_err(|error| self.noted(error))?;
        if frame.header.status != NtStatus::SUCCESS {
            return Err(wire::protocol(frame.header.status, Command::QueryInfo));
        }
        let response = QueryInfoResponse::unpack(&mut ReadCursor::new(&frame.body))?;
        end_of_file(&response.output_buffer)
    }

    async fn close(&mut self, file: FileId) -> smb2::Result<()> {
        let closed = self.tree.close_handle(&mut self.conn, file).await;
        closed.map_err(|error| self.noted(error))
    }

    async fn delete(&mut self, rel: &str) -> smb2::Result<()> {
        let deleted = wire::delete(&self.conn, self.tree, rel, DeleteKind::File).await;
        deleted.map_err(|error| self.noted(error))
    }
}
