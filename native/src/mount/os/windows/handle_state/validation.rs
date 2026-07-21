use std::io;

use super::{HandleRecord, PendingDelete, State};

use super::super::{
    handle_access::{invalid_handle, share_allows, sharing_violation},
    handle_types::HandleSnapshot,
};

pub(super) fn check_share_compatibility(
    state: &State,
    path: &str,
    desired_access: u32,
    share_access: u32,
) -> io::Result<()> {
    for record in state
        .handles
        .values()
        .filter(|record| record.share_active && record.namespace_attached && record.path == path)
    {
        if !share_allows(record.share_access, desired_access)
            || !share_allows(share_access, record.desired_access)
        {
            return Err(sharing_violation(
                "requested file access conflicts with an open handle",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_record<'a>(
    state: &'a State,
    key: u64,
    path: &str,
    is_directory: bool,
) -> io::Result<&'a HandleRecord> {
    let record = state
        .handles
        .get(&key)
        .ok_or_else(|| invalid_handle("unknown file handle"))?;
    if record.path != path || record.is_directory != is_directory {
        return Err(invalid_handle("file handle path or type does not match"));
    }
    Ok(record)
}

pub(super) fn snapshot_record(record: &HandleRecord) -> io::Result<HandleSnapshot> {
    let node = record
        .node
        .ok_or_else(|| invalid_handle("file handle is not fully opened"))?;
    Ok(HandleSnapshot {
        node,
        path: record.path.clone(),
        delete_requested: record.delete_requested,
        delete_committed: record.delete_committed,
    })
}

pub(super) fn matching_delete_type(delete: &PendingDelete, is_directory: bool) -> io::Result<()> {
    if delete.is_directory == is_directory {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete-pending object type does not match the callback",
        ))
    }
}
