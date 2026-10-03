//! Closed SMB stage metadata/flush. LastWriteTime is the sole changed
//! FileBasicInformation field; no rename or volume durability is inferred.
use super::wire;
use smb2::client::Connection;
use smb2::msg::flush::{FlushRequest, FlushResponse};
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId, flags::FileAccessMask};
use smb2::Tree;

pub(super) fn filetime(ms: i64) -> Option<u64> {
    let ticks = i128::from(ms).checked_mul(10_000)?.checked_add(116_444_736_000_000_000)?;
    // Zero and negative signed FILETIMEs have control meanings in SET_INFO.
    (ticks > 0 && ticks <= i128::from(i64::MAX)).then(|| ticks as u64)
}

pub(super) async fn set_mtime(conn: &Connection, tree: &Tree, rel: &str, ticks: u64) -> smb2::Result<()> {
    let mut bytes = vec![0u8; 40];
    bytes[16..24].copy_from_slice(&ticks.to_le_bytes());
    let file_id = open_checked(conn, tree, rel, FileAccessMask::FILE_WRITE_ATTRIBUTES).await?;
    let set = wire::set_file_info(4, file_id, bytes);
    let sent = conn.execute(Command::SetInfo, &set, Some(tree.tree_id)).await;
    let mut connection = conn.clone();
    let closed = tree.close_handle(&mut connection, file_id).await;
    let frame = sent?;
    if frame.header.status != NtStatus::SUCCESS {
        return Err(wire::protocol(frame.header.status, Command::SetInfo));
    }
    closed
}

pub(super) async fn flush(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<()> {
    let file_id = open_checked(conn, tree, rel, FileAccessMask::FILE_WRITE_DATA).await?;
    let sent = conn.execute(Command::Flush, &FlushRequest { file_id }, Some(tree.tree_id)).await;
    let mut connection = conn.clone();
    let closed = tree.close_handle(&mut connection, file_id).await;
    let frame = sent?;
    if frame.header.status != NtStatus::SUCCESS {
        return Err(wire::protocol(frame.header.status, Command::Flush));
    }
    FlushResponse::unpack(&mut ReadCursor::new(&frame.body))?;
    closed
}

async fn open_checked(conn: &Connection, tree: &Tree, rel: &str, access: u32) -> smb2::Result<FileId> {
    let create = wire::open_request(tree, rel,
        access | FileAccessMask::FILE_READ_ATTRIBUTES | FileAccessMask::SYNCHRONIZE,
        wire::FILE_NON_DIRECTORY_FILE | wire::FILE_OPEN_REPARSE_POINT);
    let frame = conn.execute(Command::Create, &create, Some(tree.tree_id)).await?;
    let response = wire::created(&frame)?;
    let mut attributes = wire::attributes_of(&response);
    if attributes.is_reparse_point() {
        let queried = conn.execute(Command::QueryInfo, &wire::attribute_tag_request(response.file_id), Some(tree.tree_id)).await;
        attributes.reparse_tag = queried.ok().as_ref().and_then(wire::reparse_tag_of);
    }
    if attributes.is_link() || attributes.is_dir() {
        wire::close_quietly(conn, tree, response.file_id).await;
        return Err(smb2::Error::invalid_data("SMB stage changed into a directory or redirecting reparse point"));
    }
    Ok(response.file_id)
}
