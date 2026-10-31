//! `sync.clearHashCache` (api.md §4.7): forgets the file digests this device
//! keeps for sync compares. The embedded worker runs in the app process, so
//! the clear of this process covers it; the answer names each step.
use serde_json::Value;

use crate::mobile::ApiError;

pub(super) fn clear() -> Result<Value, ApiError> {
    Ok(crate::daemon::clear_hash_cache().json())
}
