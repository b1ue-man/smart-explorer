//! Renames as one compound of CREATE(DELETE), SET_INFO(FileRenameInformation)
//! and CLOSE. smb2's `Tree::rename` hard-codes `ReplaceIfExists = 0`; the
//! same request with 1 replaces an existing file in one server-side step
//! (the new file becomes visible without the name ever missing), which is
//! what staged promotion needs. The source is opened with
//! FILE_OPEN_REPARSE_POINT, so a link is renamed as itself.
use super::wire;
use smb2::client::Connection;
use smb2::types::flags::FileAccessMask;
use smb2::types::FileId;
use smb2::Tree;

/// FileRenameInformation class for SET_INFO (MS-FSCC 2.4.34.2).
pub(super) const FILE_RENAME_INFORMATION: u8 = 10;

/// FILE_RENAME_INFORMATION_TYPE_2 as SMB2 sends it: ReplaceIfExists (1
/// byte), 7 reserved bytes, RootDirectory (8 bytes, 0 = share-relative),
/// FileNameLength (4 bytes), then the target path relative to the share root
/// in UTF-16LE without a terminator.
pub(super) fn rename_info(target_wire: &str, replace: bool) -> Vec<u8> {
    let name: Vec<u16> = target_wire.encode_utf16().collect();
    let name_len = (name.len() * 2) as u32;
    let mut buffer = Vec::with_capacity(20 + name.len() * 2);
    buffer.push(u8::from(replace));
    buffer.extend_from_slice(&[0u8; 7]);
    buffer.extend_from_slice(&0u64.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    for unit in name {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    buffer
}

/// Renames `from` to `to` (both share-relative). Without `replace` an
/// existing target fails with OBJECT_NAME_COLLISION (atomic no-replace);
/// with it an existing file is replaced atomically by the server.
pub(super) async fn rename(
    conn: &Connection,
    tree: &Tree,
    from: &str,
    to: &str,
    replace: bool,
) -> smb2::Result<()> {
    let create = wire::open_request(
        tree,
        from,
        FileAccessMask::DELETE | FileAccessMask::FILE_READ_ATTRIBUTES,
        wire::FILE_OPEN_REPARSE_POINT,
    );
    // The target is share-relative even on a DFS-capable share (smb2's own
    // rename sends it without the `server\share` prefix as well).
    let set = wire::set_file_info(
        FILE_RENAME_INFORMATION,
        FileId::SENTINEL,
        rename_info(&smb2::encode_path(to), replace),
    );
    // The rename is done once SET_INFO succeeded; a failed CLOSE only leaks
    // the handle, which `create_set_close` closes by hand.
    wire::create_set_close(conn, tree, &create, &set, false).await
}
