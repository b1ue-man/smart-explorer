//! Account quota includes usage from every Google service, not usageInDrive.
use super::GDriveBackend;
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, CapacityState, VfsResult};
pub(super) fn query(backend: &GDriveBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    query_inner(backend, root).or_else(|error| {
        Ok(crate::vfs::capacity_failure(
            backend.state_identity(),
            root,
            CapacitySource::DriveAccount,
            &error,
        ))
    })
}
fn query_inner(backend: &GDriveBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    let source = CapacitySource::DriveAccount;
    let root_id = backend.resolve(root)?;
    if backend.narrowed_listing_corpus(&root_id)? != "&corpora=user" {
        // Shared-drive organizational storage is not the user's My Drive
        // account quota. No shared-drive quota API was established here.
        let mut unknown = CapacitySnapshot::unknown(backend.state_identity(), root);
        unknown.source = source;
        return Ok(unknown);
    }
    let value = match backend.get_json(&backend.api_url("about?fields=storageQuota")) {
        Ok(value) => value,
        Err(error) => {
            return Ok(crate::vfs::capacity_failure(
                backend.state_identity(),
                root,
                source,
                &error,
            ))
        }
    };
    let quota = value
        .get("storageQuota")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Drive storageQuota missing",
            )
        })?;
    let number = |name: &str| -> VfsResult<Option<u64>> {
        quota
            .get(name)
            .map(|value| {
                value
                    .as_str()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Drive quota value invalid",
                        )
                    })
            })
            .transpose()
    };
    let used = number("usage")?;
    let mut snapshot = CapacitySnapshot::unknown(backend.state_identity(), root);
    snapshot.source = source;
    snapshot.observed_at_ms = crate::vfs::observed_now_ms();
    snapshot.used_bytes = used;
    match number("limit")? {
        None => snapshot.state = CapacityState::Unbounded,
        Some(total) => {
            snapshot.state = CapacityState::AccountQuota;
            snapshot.total_bytes = Some(total);
            snapshot.available_bytes = used.map(|used| total.saturating_sub(used));
        }
    }
    Ok(snapshot)
}
