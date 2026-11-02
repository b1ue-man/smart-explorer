//! MS-FSCC FileFsFullSizeInformation on the selected share, with caller quota.
use super::{backend::SmbBackend, url::split_path, wire};
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, VfsResult};
use smb2::{
    client::Connection,
    msg::query_info::{QueryInfoRequest, QueryInfoResponse},
    msg::set_info::InfoType,
    pack::{ReadCursor, Unpack},
    types::{flags::FileAccessMask, Command},
    Tree,
};

pub(super) fn query(backend: &SmbBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    let Some(target) = split_path(root)? else {
        return Ok(CapacitySnapshot::unknown(backend.state_identity(), root));
    };
    let result = backend.session.read(
        &target.share,
        "Speicherplatz abfragen",
        root,
        |conn, tree| {
            let rel = target.rel.clone();
            async move { filesystem(&conn, &tree, &rel).await }
        },
    );
    match result {
        Ok((total, free, available)) => Ok(CapacitySnapshot::known(
            backend.state_identity(),
            root,
            CapacitySource::Filesystem,
            total,
            total.saturating_sub(free),
            available,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            Ok(CapacitySnapshot::unknown(backend.state_identity(), root))
        }
        Err(error) => Ok(crate::vfs::capacity_failure(
            backend.state_identity(),
            root,
            CapacitySource::Filesystem,
            &error,
        )),
    }
}
async fn filesystem(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<(u64, u64, u64)> {
    let open = wire::open_request(
        tree,
        rel,
        FileAccessMask::FILE_READ_ATTRIBUTES,
        wire::FILE_OPEN_REPARSE_POINT | wire::FILE_DIRECTORY_FILE,
    );
    let frame = conn
        .execute(Command::Create, &open, Some(tree.tree_id))
        .await?;
    let handle = wire::created(&frame)?.file_id;
    let request = QueryInfoRequest {
        info_type: InfoType::Filesystem,
        file_info_class: 7,
        output_buffer_length: 32,
        additional_information: 0,
        flags: 0,
        file_id: handle,
        input_buffer: Vec::new(),
    };
    let result = conn
        .execute(Command::QueryInfo, &request, Some(tree.tree_id))
        .await;
    wire::close_quietly(conn, tree, handle).await;
    let frame = result?;
    if frame.header.status != smb2::types::status::NtStatus::SUCCESS {
        return Err(wire::protocol(frame.header.status, Command::QueryInfo));
    }
    let response = QueryInfoResponse::unpack(&mut ReadCursor::new(&frame.body))?;
    parse(&response.output_buffer)
}
fn parse(bytes: &[u8]) -> smb2::Result<(u64, u64, u64)> {
    if bytes.len() != 32 {
        return Err(smb2::Error::invalid_data(
            "invalid FileFsFullSizeInformation length",
        ));
    }
    let count = |at: usize| -> smb2::Result<u64> {
        let value = i64::from_le_bytes(bytes[at..at + 8].try_into().map_err(|_| invalid())?);
        u64::try_from(value).map_err(|_| invalid())
    };
    let sectors = u32::from_le_bytes(bytes[24..28].try_into().map_err(|_| invalid())?) as u64;
    let sector_bytes = u32::from_le_bytes(bytes[28..32].try_into().map_err(|_| invalid())?) as u64;
    let unit = sectors
        .checked_mul(sector_bytes)
        .filter(|value| *value > 0)
        .ok_or_else(invalid)?;
    let total = count(0)?.checked_mul(unit).ok_or_else(invalid)?;
    let caller = count(8)?.checked_mul(unit).ok_or_else(invalid)?;
    // TotalAllocationUnits is caller-specific; the physical volume may
    // have more free bytes than this user's quota. Do not subtract that
    // physical value from the caller's total or reject a valid quota.
    let _physical_free = count(16)?.checked_mul(unit).ok_or_else(invalid)?;
    if caller > total {
        return Err(invalid());
    }
    Ok((total, caller, caller))
}
fn invalid() -> smb2::Error {
    smb2::Error::invalid_data("invalid SMB filesystem capacity")
}
