//! Atomic CREATE(FileCreate) and volume-scoped 128-bit IDs, MS-FSCC 2.4.26.
use super::{backend::SmbBackend, url::split_path, wire};
use crate::vfs::{Backend, VfsResult};
use smb2::{
    client::Connection,
    msg::query_info::{QueryInfoRequest, QueryInfoResponse},
    msg::set_info::InfoType,
    pack::{ReadCursor, Unpack},
    types::{flags::FileAccessMask, status::NtStatus, Command},
    Tree,
};
use std::io;

pub(super) fn create(backend: &SmbBackend, path: &str) -> VfsResult<()> {
    backend.create_dir_new(path)
}

pub(super) fn identity(backend: &SmbBackend, path: &str) -> VfsResult<Option<String>> {
    let Some(target) = split_path(path)? else {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "SMB server root has no directory object ID"));
    };
    let rel = target.rel;
    let value = backend.session.read(&target.share, "Ordneridentität abfragen", path, |conn, tree| {
        let rel = rel.clone();
        async move { object_id(&conn, &tree, &rel).await }
    })?;
    let namespace = backend.namespace_identity();
    Ok(value.map(|(volume, id, creation)| format!("smb-folder:{}:{namespace}:{}:{}:{volume:016x}:{id:032x}:{creation:016x}",
        namespace.len(), target.share.len(), target.share)))
}

async fn object_id(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<Option<(u64, u128, u64)>> {
    // Read attributes and the ID of the SAME held directory, never a link
    // target or the protocol's connection-lifetime CREATE handle FileId.
    let open = wire::open_request(tree, rel, FileAccessMask::FILE_READ_ATTRIBUTES,
        wire::FILE_DIRECTORY_FILE | wire::FILE_OPEN_REPARSE_POINT);
    let frame = conn.execute(Command::Create, &open, Some(tree.tree_id)).await?;
    if frame.header.status == NtStatus(0xC000_0103) { return Ok(None); }
    let opened = wire::created(&frame)?;
    let result = async {
        let mut attributes = wire::attributes_of(&opened);
        if attributes.is_reparse_point() {
            let tag = wire::attribute_tag_request(opened.file_id);
            let answer = conn.execute(Command::QueryInfo, &tag, Some(tree.tree_id)).await?;
            attributes.reparse_tag = wire::reparse_tag_of(&answer);
        }
        if !attributes.is_dir() || attributes.is_link() {
            return Ok(None);
        }
        let query = QueryInfoRequest {
            info_type: InfoType::File, file_info_class: 59, output_buffer_length: 24,
            additional_information: 0, flags: 0, file_id: opened.file_id, input_buffer: Vec::new(),
        };
        let answer = conn.execute(Command::QueryInfo, &query, Some(tree.tree_id)).await?;
        // Unsupported information class is not a path-format or permission
        // failure. Other refusals retain their actual provider error class.
        if matches!(answer.header.status, NtStatus(0xC000_0002 | 0xC000_0003 | 0xC000_000D | 0xC000_00BB)) {
            return Ok(None);
        }
        if answer.header.status != NtStatus::SUCCESS {
            return Err(wire::protocol(answer.header.status, Command::QueryInfo));
        }
        let response = QueryInfoResponse::unpack(&mut ReadCursor::new(&answer.body))?;
        if response.output_buffer.len() != 24 {
            return Err(smb2::Error::invalid_data("invalid FileIdInformation length"));
        }
        let bytes = &response.output_buffer;
        let volume = u64::from_le_bytes(bytes[..8].try_into()
            .map_err(|_| smb2::Error::invalid_data("invalid volume serial number"))?);
        let id = u128::from_le_bytes(bytes[8..].try_into()
            .map_err(|_| smb2::Error::invalid_data("invalid 128-bit file ID"))?);
        // Zero is unsupported by MS-FSCC; all-ones is also kept unproven.
        Ok((id != 0 && id != u128::MAX).then_some((volume, id, opened.creation_time.0)))
    }.await;
    wire::close_quietly(conn, tree, opened.file_id).await;
    result
}
