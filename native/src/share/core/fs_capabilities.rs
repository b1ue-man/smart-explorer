use std::io;
use std::sync::{Arc, Mutex};

use crate::vfs::StagedWriteCapabilities;

use super::fs::{self, ShareExportConfig};

const CONNECTIONS_ROOT: &str = "/Verbindungen";

pub(crate) fn staged_write_capabilities(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<StagedWriteCapabilities> {
    let normalized = normalize(path);
    if normalized == "/" || normalized == CONNECTIONS_ROOT {
        // These are synthetic mountpoint containers, not writable namespace
        // targets. Treating them as an intersection of children would falsely
        // permit create/cross-export rename and would fan one capability probe
        // out into credential and network access for every saved connection.
        return Ok(StagedWriteCapabilities::default());
    }
    let target = fs::resolve(&normalized, exports)?;
    Ok(target.backend.staged_write_capabilities(&target.path))
}

fn normalize(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}
