//! Single-file reads with their first READ in the open (G3-07, 2026-10-28):
//! CREATE + READ go out as one compound, so a file up to one READ block
//! arrives in one round trip instead of two (smb2's reader opened with
//! CREATE + two name queries first and read after that). The compound also
//! carries the CLOSE when the caller needs nothing beyond that READ (a
//! range that fits); otherwise the handle stays open for the pipelined READs
//! of reader.rs and is closed without waiting when the stream ends.
//!
//! The CREATE is the one smb2's `FileReader` sent (read access, every
//! share mode, `FileOpen`, no create options), so links and data reparse
//! points open exactly as before. Its answer's size bounds the READs, as the
//! size seen at open did. It is not offered as `open_read_evidence`: this
//! CREATE does not open a link as itself, so its attributes need not be what
//! `stat` (FILE_OPEN_REPARSE_POINT) answers.
use super::backend::SmbBackend;
use super::batch_wire::{charge, read_data, read_request};
use super::reader::{read_chunk, SmbReader};
use super::wire;
use crate::vfs::VfsResult;
use smb2::client::Connection;
use smb2::msg::create::{CreateDisposition, CreateRequest, ImpersonationLevel, ShareAccess};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId, OplockLevel};
use smb2::{CompoundOp, Tree};
use std::io::Read;
use std::sync::Arc;

/// What a reader opens with: the length of the READ inside the open's
/// compound and whether the CLOSE goes with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FirstRead {
    pub(super) len: u64,
    pub(super) close: bool,
}

/// The first READ for a stream from `start` (to `end` for a range): one
/// block; the CLOSE joins it when the range ends within that block.
pub(super) fn first_read(chunk: u64, start: u64, end: Option<u64>) -> FirstRead {
    match end {
        Some(end) => {
            let wanted = end.saturating_sub(start);
            FirstRead {
                len: wanted.min(chunk),
                close: wanted <= chunk,
            }
        }
        None => FirstRead {
            len: chunk,
            close: false,
        },
    }
}

/// The open handle positioned READs go through (several at once: every
/// call takes `&self` and keeps no cursor, as smb2's `FileReader`).
pub(super) struct Handle {
    tree: Arc<Tree>,
    conn: Connection,
    file_id: FileId,
    size: u64,
    /// Longest READ: one block of the reader (`read_chunk`).
    chunk: u64,
}

impl Handle {
    /// Up to `len` bytes from `offset`, fewer only at the end of the file
    /// (as `FileReader::read_at`).
    pub(super) async fn read_at(&self, offset: u64, len: u64) -> smb2::Result<Vec<u8>> {
        if len == 0 || offset >= self.size {
            return Ok(Vec::new());
        }
        let end = offset + len.min(self.size - offset);
        let mut out = Vec::with_capacity((end - offset) as usize);
        let mut position = offset;
        while position < end {
            let block = (end - position).min(self.chunk) as usize;
            let request = read_request(self.file_id, position, block);
            let frame = self
                .conn
                .execute_with_credits(
                    Command::Read,
                    &request,
                    Some(self.tree.tree_id),
                    charge(block),
                )
                .await?;
            // A racing truncation shows as END_OF_FILE: a short read.
            let data = read_data(&frame)?;
            if data.is_empty() {
                break;
            }
            position += data.len() as u64;
            out.extend_from_slice(&data);
        }
        Ok(out)
    }

    pub(super) async fn close(self) -> smb2::Result<()> {
        let mut conn = self.conn.clone();
        self.tree.close_handle(&mut conn, self.file_id).await
    }
}

/// The CREATE of smb2's `FileReader` (`Tree::read_open_request`).
fn read_create(tree: &Tree, rel: &str) -> CreateRequest {
    CreateRequest {
        requested_oplock_level: OplockLevel::None,
        impersonation_level: ImpersonationLevel::Impersonation,
        desired_access: FileAccessMask::new(
            FileAccessMask::FILE_READ_DATA
                | FileAccessMask::FILE_READ_ATTRIBUTES
                | FileAccessMask::SYNCHRONIZE,
        ),
        file_attributes: 0,
        share_access: ShareAccess(
            ShareAccess::FILE_SHARE_READ
                | ShareAccess::FILE_SHARE_WRITE
                | ShareAccess::FILE_SHARE_DELETE,
        ),
        create_disposition: CreateDisposition::FileOpen,
        create_options: 0,
        name: wire::wire_path(tree, rel),
        create_contexts: Vec::new(),
    }
}

/// An opened file: its handle (`None` when the compound closed it), its
/// size at open and the bytes of the first READ.
pub(super) struct Opened {
    pub(super) handle: Option<Handle>,
    pub(super) size: u64,
    pub(super) first: FirstRead,
    pub(super) prefix: Vec<u8>,
}

/// CREATE + READ (+ CLOSE) of `rel` in one round trip.
pub(super) async fn open(
    tree: Arc<Tree>,
    conn: Connection,
    rel: &str,
    start: u64,
    first: FirstRead,
) -> smb2::Result<Opened> {
    let create = read_create(&tree, rel);
    let read = read_request(FileId::SENTINEL, start, first.len as usize);
    let close = wire::related_close();
    let tree_id = Some(tree.tree_id);
    let mut ops = vec![CompoundOp::new(Command::Create, &create, tree_id)];
    if first.len > 0 {
        ops.push(CompoundOp {
            command: Command::Read,
            body: &read,
            tree_id,
            credit_charge: charge(first.len as usize),
        });
    }
    if first.close {
        ops.push(CompoundOp::new(Command::Close, &close, tree_id));
    }
    let frames = wire::frames(conn.execute_compound(&ops).await?, ops.len())?;
    let response = wire::created(&frames[0])?;
    let id = response.file_id;
    let mut prefix = Vec::new();
    if first.len > 0 {
        match read_data(&frames[1]) {
            Ok(data) => prefix = data,
            Err(error) => {
                wire::close_quietly(&conn, &tree, id).await;
                return Err(error);
            }
        }
    }
    if first.close && frames[ops.len() - 1].header.status != NtStatus::SUCCESS {
        wire::close_quietly(&conn, &tree, id).await;
    }
    let chunk = read_chunk(&conn);
    let size = response.end_of_file;
    let handle = (!first.close).then(|| Handle {
        tree,
        conn,
        file_id: id,
        size,
        chunk,
    });
    Ok(Opened {
        handle,
        size,
        first,
        prefix,
    })
}

impl SmbBackend {
    /// A reader from byte `start` (to `end` for a range); a connection lost
    /// before any byte was read is replaced and the open repeated once.
    pub(super) fn open_reader(
        &self,
        path: &str,
        start: u64,
        end: Option<u64>,
    ) -> VfsResult<Box<dyn Read + Send>> {
        let target = super::url::entry_path(path)?;
        let mut retried = false;
        loop {
            let (generation, tree) = self.session.target(&target.share)?;
            let conn = generation.connection();
            let first = first_read(read_chunk(&conn), start, end);
            let opened = self
                .session
                .block_on(open(tree, conn, &target.rel, start, first));
            match opened {
                Ok(opened) => {
                    let reader = SmbReader::new(
                        opened,
                        start,
                        end,
                        path,
                        generation,
                        self.session.runtime(),
                    );
                    return Ok(Box::new(reader));
                }
                Err(error) if generation.note(&error) && !retried => retried = true,
                Err(error) => return Err(super::errors::map(error, "Öffnen", path)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{first_read, FirstRead};

    fn read(len: u64, close: bool) -> FirstRead {
        FirstRead { len, close }
    }

    #[test]
    fn speed_task_smb_first_read_closes_only_when_the_range_fits() {
        let chunk = 512 * 1024;
        assert_eq!(first_read(chunk, 0, None), read(chunk, false));
        assert_eq!(first_read(chunk, 100, Some(1100)), read(1000, true));
        assert_eq!(first_read(chunk, 0, Some(chunk + 1)), read(chunk, false));
        assert_eq!(first_read(chunk, 5, Some(5)), read(0, true));
    }
}
