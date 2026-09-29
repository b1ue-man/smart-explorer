//! Pre-generated Drive IDs (`files.generateIds`) in growing batches instead of
//! one request per new file or folder (ref §1). Each ID is handed out once;
//! callers that must survive an ambiguous create keep their ID themselves.
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The API's default `count`: after a pause the first refill costs no more
/// than one of the former per-object calls.
const FIRST_BATCH: usize = 10;
/// Documented maximum of one `generateIds` call.
const MAX_BATCH: usize = 1000;
/// The API documents no lifetime for generated IDs. Formerly each was used
/// seconds after its generation; older pooled IDs are dropped (and the batch
/// size starts over) instead of relying on an unspecified lifetime.
const MAX_AGE: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub(super) struct IdPool {
    state: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    ids: Vec<String>,
    fetched: Option<Instant>,
    next_batch: usize,
}

impl IdPool {
    fn state(&self) -> io::Result<MutexGuard<'_, PoolState>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("Drive-ID-Vorrat vergiftet"))
    }
}

impl GDriveBackend {
    /// One never-used ID for a new file or folder.
    pub(super) fn take_generated_id(&self) -> VfsResult<String> {
        let count = {
            let mut pool = self.id_pool.state()?;
            if pool.fetched.is_some_and(|at| at.elapsed() > MAX_AGE) {
                pool.ids.clear();
                pool.next_batch = FIRST_BATCH;
            }
            if let Some(id) = pool.ids.pop() {
                return Ok(id);
            }
            // Each refill doubles: bulk work needs O(log n) calls, a single
            // new object still only ten IDs.
            let count = pool.next_batch.clamp(FIRST_BATCH, MAX_BATCH);
            pool.next_batch = count.saturating_mul(2).min(MAX_BATCH);
            count
        };
        // Outside the pool lock: other threads keep taking pooled IDs.
        let url = self.api_url(&format!(
            "files/generateIds?count={count}&space=drive&type=files"
        ));
        let mut ids = parse_generated_ids(&self.get_json(&url)?)?;
        let id = ids.pop().ok_or_else(no_usable_id)?;
        let mut pool = self.id_pool.state()?;
        pool.ids.extend(ids);
        pool.fetched = Some(Instant::now());
        Ok(id)
    }
}

/// Every usable ID of a `generateIds` answer; at least one.
pub(super) fn parse_generated_ids(json: &serde_json::Value) -> VfsResult<Vec<String>> {
    let ids: Vec<String> = json["ids"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(no_usable_id());
    }
    Ok(ids)
}

fn no_usable_id() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Drive generateIds response has no usable id",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_drive_generated_ids_parse_strictly() {
        let json = serde_json::json!({"ids": ["a", "", "b"]});
        assert_eq!(parse_generated_ids(&json).unwrap(), ["a", "b"]);
        assert!(parse_generated_ids(&serde_json::json!({"ids": []})).is_err());
        assert!(parse_generated_ids(&serde_json::json!({})).is_err());
    }
}
