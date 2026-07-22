use serde::{Deserialize, Serialize};
use std::io;

pub const DEFAULT_METADATA_PRELOAD_DEPTH: u8 = 2;
pub const MAX_METADATA_PRELOAD_DEPTH: u8 = 4;

/// Persistent, backend-neutral metadata behavior for one mounted drive.
/// Depth zero keeps the on-demand cache but disables proactive traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountMetadataPolicy {
    preload_depth: u8,
}

impl Default for MountMetadataPolicy {
    fn default() -> Self {
        Self {
            preload_depth: DEFAULT_METADATA_PRELOAD_DEPTH,
        }
    }
}

impl MountMetadataPolicy {
    pub fn new(preload_depth: u8) -> io::Result<Self> {
        let policy = Self { preload_depth };
        policy.validate()?;
        Ok(policy)
    }

    pub const fn preload_depth(self) -> u8 {
        self.preload_depth
    }

    pub fn validate(self) -> io::Result<()> {
        if self.preload_depth > MAX_METADATA_PRELOAD_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "metadata preload depth must be between 0 and {MAX_METADATA_PRELOAD_DEPTH}"
                ),
            ));
        }
        Ok(())
    }
}
