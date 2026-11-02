//! russh-sftp fs_info checks the negotiated statvfs@openssh.com v2 feature.
use super::backend::SftpBackend;
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, VfsResult};
pub(super) fn query(backend: &SftpBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    query_inner(backend, root).or_else(|error| {
        Ok(crate::vfs::capacity_failure(
            backend.state_identity(),
            root,
            CapacitySource::SftpStatvfs,
            &error,
        ))
    })
}
fn query_inner(backend: &SftpBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    let source = CapacitySource::SftpStatvfs;
    let info = match backend
        .connection
        .safe_metadata(|generation| Box::pin(generation.sftp().fs_info(root.to_string())))
    {
        Ok(Some(info)) => info,
        Ok(None) => return Ok(CapacitySnapshot::unknown(backend.state_identity(), root)),
        Err(error) => {
            return Ok(crate::vfs::capacity_failure(
                backend.state_identity(),
                root,
                source,
                &error,
            ))
        }
    };
    let bytes = |blocks: u64| {
        blocks.checked_mul(info.fragment_size).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "SFTP capacity overflow")
        })
    };
    if info.fragment_size == 0
        || info.blocks_free > info.blocks
        || info.blocks_avail > info.blocks_free
    {
        return Ok(crate::vfs::capacity_failure(
            backend.state_identity(),
            root,
            source,
            &std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid SFTP capacity accounting",
            ),
        ));
    }
    let total = bytes(info.blocks)?;
    let mut snapshot = CapacitySnapshot::known(
        backend.state_identity(),
        root,
        source,
        total,
        total.saturating_sub(bytes(info.blocks_free)?),
        bytes(info.blocks_avail)?,
    );
    snapshot.read_only = info.flags & 1 != 0;
    Ok(snapshot)
}
